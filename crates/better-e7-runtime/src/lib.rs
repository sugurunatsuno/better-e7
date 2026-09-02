use std::{
    error::Error,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use better_e7_adb::{AdbClient, AdbDevice, DeviceLister};
use better_e7_android::{ActiveVideoSession, ScrcpySessionFactory, VideoSessionFactory};
use better_e7_config::AppConfig;
use better_e7_core::Frame;
use better_e7_video::{FfmpegProcessDecoderFactory, VideoDecoder, VideoDecoderFactory};
use tokio::{
    runtime::{Builder, Runtime},
    sync::mpsc,
    time::MissedTickBehavior,
};

const VIDEO_BUFFER_SIZE: usize = 64 * 1_024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeCommand {
    RefreshDevices,
    SelectDevice(String),
    StartAutomation,
    StopAutomation,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationState {
    Stopped,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEvent {
    DevicesUpdated(Vec<AdbDevice>),
    SelectedDeviceChanged(Option<String>),
    AutomationStateChanged(AutomationState),
    ConnectionStateChanged(ConnectionState),
    VideoBytesReceived(u64),
    Error(String),
}

pub struct AppRuntime {
    command_tx: mpsc::UnboundedSender<RuntimeCommand>,
    event_rx: mpsc::UnboundedReceiver<RuntimeEvent>,
    latest_frame: Arc<Mutex<Option<Frame>>>,
    _runtime: Runtime,
}

impl AppRuntime {
    pub fn new(config: &AppConfig) -> Result<Self, RuntimeError> {
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .thread_name("better-e7-worker")
            .build()
            .map_err(RuntimeError::Build)?;
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let lister: Arc<dyn DeviceLister> = Arc::new(AdbClient::new(config.adb_path.clone()));
        let session_factory: Arc<dyn VideoSessionFactory> =
            Arc::new(ScrcpySessionFactory::new(config));
        let decoder_factory: Arc<dyn VideoDecoderFactory> =
            Arc::new(FfmpegProcessDecoderFactory::new(config.ffmpeg_path.clone()));
        let latest_frame = Arc::new(Mutex::new(None));
        let refresh_interval = Duration::from_millis(config.device_refresh_interval_ms);

        let _coordinator = runtime.spawn(run_coordinator(
            lister,
            session_factory,
            decoder_factory,
            Arc::clone(&latest_frame),
            refresh_interval,
            command_rx,
            event_tx,
        ));

        Ok(Self {
            command_tx,
            event_rx,
            latest_frame,
            _runtime: runtime,
        })
    }

    pub fn send(&self, command: RuntimeCommand) -> Result<(), RuntimeError> {
        self.command_tx
            .send(command)
            .map_err(|_| RuntimeError::Stopped)
    }

    pub fn try_next_event(&mut self) -> Option<RuntimeEvent> {
        self.event_rx.try_recv().ok()
    }

    pub fn take_latest_frame(&self) -> Option<Frame> {
        self.latest_frame.lock().ok()?.take()
    }
}

impl Drop for AppRuntime {
    fn drop(&mut self) {
        let _ = self.command_tx.send(RuntimeCommand::Shutdown);
    }
}

async fn run_coordinator(
    lister: Arc<dyn DeviceLister>,
    session_factory: Arc<dyn VideoSessionFactory>,
    decoder_factory: Arc<dyn VideoDecoderFactory>,
    latest_frame: Arc<Mutex<Option<Frame>>>,
    refresh_interval: Duration,
    mut command_rx: mpsc::UnboundedReceiver<RuntimeCommand>,
    event_tx: mpsc::UnboundedSender<RuntimeEvent>,
) {
    let mut interval = tokio::time::interval(refresh_interval);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let (worker_event_tx, mut worker_event_rx) = mpsc::unbounded_channel();
    let mut devices = Vec::new();
    let mut selected_device: Option<String> = None;
    let mut automation_state = AutomationState::Stopped;
    let mut connection_state = ConnectionState::Disconnected;
    let mut video_worker_stop: Option<Arc<AtomicBool>> = None;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                refresh_devices(
                    Arc::clone(&lister),
                    &event_tx,
                    &mut devices,
                    &mut selected_device,
                    &mut automation_state,
                    &mut connection_state,
                    video_worker_stop.as_ref(),
                ).await;
            }
            worker_event = worker_event_rx.recv() => {
                if let Some(worker_event) = worker_event {
                    match worker_event {
                        VideoWorkerEvent::Progress(total_bytes) => {
                            send_event(&event_tx, RuntimeEvent::VideoBytesReceived(total_bytes));
                        }
                        VideoWorkerEvent::Ended(result) => {
                            video_worker_stop = None;
                            automation_state = AutomationState::Stopped;
                            connection_state = ConnectionState::Disconnected;
                            send_event(
                                &event_tx,
                                RuntimeEvent::AutomationStateChanged(automation_state),
                            );
                            send_event(
                                &event_tx,
                                RuntimeEvent::ConnectionStateChanged(connection_state),
                            );
                            if let Err(message) = result {
                                send_event(&event_tx, RuntimeEvent::Error(message));
                            }
                        }
                    }
                }
            }
            command = command_rx.recv() => {
                let Some(command) = command else {
                    request_video_stop(video_worker_stop.as_ref());
                    break;
                };
                match command {
                    RuntimeCommand::RefreshDevices => {
                        refresh_devices(
                            Arc::clone(&lister),
                            &event_tx,
                            &mut devices,
                            &mut selected_device,
                            &mut automation_state,
                            &mut connection_state,
                            video_worker_stop.as_ref(),
                        ).await;
                    }
                    RuntimeCommand::SelectDevice(serial) => {
                        if connection_state != ConnectionState::Disconnected {
                            send_event(&event_tx, RuntimeEvent::Error(
                                "stop the video session before changing the device".to_owned(),
                            ));
                        } else if devices.iter().any(|device| device.serial == serial && device.is_ready()) {
                            selected_device = Some(serial);
                            send_event(
                                &event_tx,
                                RuntimeEvent::SelectedDeviceChanged(selected_device.clone()),
                            );
                        } else {
                            send_event(&event_tx, RuntimeEvent::Error(
                                "the selected device is not ready".to_owned(),
                            ));
                        }
                    }
                    RuntimeCommand::StartAutomation => {
                        if connection_state != ConnectionState::Disconnected {
                            send_event(&event_tx, RuntimeEvent::Error(
                                "a video session is already active".to_owned(),
                            ));
                            continue;
                        }
                        let serial = selected_device.as_ref().filter(|serial| {
                            devices.iter().any(|device| &device.serial == *serial && device.is_ready())
                        }).cloned();
                        let Some(serial) = serial else {
                            send_event(&event_tx, RuntimeEvent::Error(
                                "select a ready device before starting automation".to_owned(),
                            ));
                            continue;
                        };

                        connection_state = ConnectionState::Connecting;
                        send_event(
                            &event_tx,
                            RuntimeEvent::ConnectionStateChanged(connection_state),
                        );
                        let factory = Arc::clone(&session_factory);
                        match tokio::task::spawn_blocking(move || factory.start(&serial)).await {
                            Ok(Ok(session)) => {
                                let stop = Arc::new(AtomicBool::new(false));
                                spawn_video_worker(
                                    session,
                                    Arc::clone(&decoder_factory),
                                    Arc::clone(&latest_frame),
                                    Arc::clone(&stop),
                                    worker_event_tx.clone(),
                                );
                                video_worker_stop = Some(stop);
                                automation_state = AutomationState::Running;
                                connection_state = ConnectionState::Connected;
                                send_event(
                                    &event_tx,
                                    RuntimeEvent::AutomationStateChanged(automation_state),
                                );
                                send_event(
                                    &event_tx,
                                    RuntimeEvent::ConnectionStateChanged(connection_state),
                                );
                            }
                            Ok(Err(error)) => {
                                connection_state = ConnectionState::Disconnected;
                                send_event(
                                    &event_tx,
                                    RuntimeEvent::ConnectionStateChanged(connection_state),
                                );
                                send_event(&event_tx, RuntimeEvent::Error(error.to_string()));
                            }
                            Err(error) => {
                                connection_state = ConnectionState::Disconnected;
                                send_event(
                                    &event_tx,
                                    RuntimeEvent::ConnectionStateChanged(connection_state),
                                );
                                send_event(
                                    &event_tx,
                                    RuntimeEvent::Error(format!("session worker failed: {error}")),
                                );
                            }
                        }
                    }
                    RuntimeCommand::StopAutomation => {
                        if video_worker_stop.is_some() {
                            connection_state = ConnectionState::Disconnecting;
                            send_event(
                                &event_tx,
                                RuntimeEvent::ConnectionStateChanged(connection_state),
                            );
                            request_video_stop(video_worker_stop.as_ref());
                        }
                    }
                    RuntimeCommand::Shutdown => {
                        request_video_stop(video_worker_stop.as_ref());
                        break;
                    }
                }
            }
        }
    }
}

async fn refresh_devices(
    lister: Arc<dyn DeviceLister>,
    event_tx: &mpsc::UnboundedSender<RuntimeEvent>,
    devices: &mut Vec<AdbDevice>,
    selected_device: &mut Option<String>,
    automation_state: &mut AutomationState,
    connection_state: &mut ConnectionState,
    video_worker_stop: Option<&Arc<AtomicBool>>,
) {
    let result = tokio::task::spawn_blocking(move || lister.list_devices()).await;
    match result {
        Ok(Ok(updated_devices)) => {
            let selected_is_ready = selected_device.as_ref().is_some_and(|serial| {
                updated_devices
                    .iter()
                    .any(|device| &device.serial == serial && device.is_ready())
            });
            if !selected_is_ready && selected_device.take().is_some() {
                send_event(event_tx, RuntimeEvent::SelectedDeviceChanged(None));
                request_video_stop(video_worker_stop);
                if *automation_state == AutomationState::Running {
                    *automation_state = AutomationState::Stopped;
                    *connection_state = ConnectionState::Disconnecting;
                    send_event(
                        event_tx,
                        RuntimeEvent::AutomationStateChanged(*automation_state),
                    );
                    send_event(
                        event_tx,
                        RuntimeEvent::ConnectionStateChanged(*connection_state),
                    );
                }
            }
            *devices = updated_devices;
            send_event(event_tx, RuntimeEvent::DevicesUpdated(devices.clone()));
        }
        Ok(Err(error)) => send_event(event_tx, RuntimeEvent::Error(error.to_string())),
        Err(error) => send_event(
            event_tx,
            RuntimeEvent::Error(format!("device scan worker failed: {error}")),
        ),
    }
}

fn spawn_video_worker(
    mut session: Box<dyn ActiveVideoSession>,
    decoder_factory: Arc<dyn VideoDecoderFactory>,
    latest_frame: Arc<Mutex<Option<Frame>>>,
    stop: Arc<AtomicBool>,
    worker_event_tx: mpsc::UnboundedSender<VideoWorkerEvent>,
) {
    let _worker = tokio::task::spawn_blocking(move || {
        let mut buffer = vec![0_u8; VIDEO_BUFFER_SIZE];
        let mut total_bytes = 0_u64;
        let mut decoder = match decoder_factory.create() {
            Ok(decoder) => decoder,
            Err(error) => {
                let _ = session.stop();
                let _ = worker_event_tx.send(VideoWorkerEvent::Ended(Err(error.to_string())));
                return;
            }
        };
        let result = loop {
            if stop.load(Ordering::Acquire) {
                break Ok(());
            }
            match session.read_video(&mut buffer) {
                Ok(0) => break Ok(()),
                Ok(bytes_read) => {
                    total_bytes = total_bytes.saturating_add(bytes_read as u64);
                    let _ = worker_event_tx.send(VideoWorkerEvent::Progress(total_bytes));
                    if let Err(error) = decoder.push(&buffer[..bytes_read]) {
                        break Err(error.to_string());
                    }
                }
                Err(error) if error.is_retryable() => {}
                Err(error) => break Err(error.to_string()),
            }
            if let Err(error) = store_decoded_frames(decoder.as_mut(), &latest_frame) {
                break Err(error);
            }
        };

        let result = result.and_then(|()| session.stop().map_err(|error| error.to_string()));
        let _ = worker_event_tx.send(VideoWorkerEvent::Ended(result));
    });
}

fn store_decoded_frames(
    decoder: &mut dyn VideoDecoder,
    latest_frame: &Mutex<Option<Frame>>,
) -> Result<(), String> {
    while let Some(frame) = decoder
        .try_next_frame()
        .map_err(|error| error.to_string())?
    {
        let mut slot = latest_frame
            .lock()
            .map_err(|_| "latest frame store is unavailable".to_owned())?;
        *slot = Some(frame);
    }
    Ok(())
}

fn request_video_stop(stop: Option<&Arc<AtomicBool>>) {
    if let Some(stop) = stop {
        stop.store(true, Ordering::Release);
    }
}

fn send_event(event_tx: &mpsc::UnboundedSender<RuntimeEvent>, event: RuntimeEvent) {
    let _ = event_tx.send(event);
}

enum VideoWorkerEvent {
    Progress(u64),
    Ended(Result<(), String>),
}

#[derive(Debug)]
pub enum RuntimeError {
    Build(std::io::Error),
    Stopped,
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Build(error) => write!(formatter, "failed to build runtime: {error}"),
            Self::Stopped => formatter.write_str("runtime has stopped"),
        }
    }
}

impl Error for RuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Build(error) => Some(error),
            Self::Stopped => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, time::Instant};

    use better_e7_core::PixelFormat;
    use better_e7_video::VideoDecodeError;

    use super::*;

    struct MockDecoder {
        frames: VecDeque<Frame>,
    }

    impl VideoDecoder for MockDecoder {
        fn push(&mut self, _data: &[u8]) -> Result<(), VideoDecodeError> {
            Ok(())
        }

        fn try_next_frame(&mut self) -> Result<Option<Frame>, VideoDecodeError> {
            Ok(self.frames.pop_front())
        }
    }

    #[test]
    fn keeps_only_the_latest_decoded_frame() {
        let frames = [1_u64, 2]
            .into_iter()
            .map(|id| Frame::new(id, Instant::now(), 1, 1, PixelFormat::Rgb8, vec![0; 3]).unwrap())
            .collect();
        let mut decoder = MockDecoder { frames };
        let latest = Mutex::new(None);

        store_decoded_frames(&mut decoder, &latest).unwrap();

        assert_eq!(latest.lock().unwrap().as_ref().unwrap().id(), 2);
    }
}

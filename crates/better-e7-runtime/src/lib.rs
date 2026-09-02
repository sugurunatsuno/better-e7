use std::{
    error::Error,
    fmt,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc as std_mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use better_e7_adb::{AdbClient, AdbDevice, AdbInputController, DeviceLister};
use better_e7_android::{ActiveVideoSession, ScrcpySessionFactory, VideoSessionFactory};
use better_e7_config::AppConfig;
use better_e7_core::{Frame, InputCommand, InputController, PixelInputCommand};
use better_e7_video::{FfmpegProcessDecoderFactory, VideoDecoder, VideoDecoderFactory};
use tokio::{
    runtime::{Builder, Runtime},
    sync::mpsc,
    time::MissedTickBehavior,
};

const VIDEO_BUFFER_SIZE: usize = 64 * 1_024;
const INPUT_QUEUE_SIZE: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeCommand {
    RefreshDevices,
    SelectDevice(String),
    StartAutomation,
    StopAutomation,
    SubmitInput(InputCommand),
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
    InputQueued(PixelInputCommand),
    InputExecuted(PixelInputCommand),
    Error(String),
}

#[derive(Default)]
struct LatestFrameStore {
    pending: Option<Frame>,
    dimensions: Option<(u32, u32)>,
}

pub struct AppRuntime {
    command_tx: mpsc::UnboundedSender<RuntimeCommand>,
    event_rx: mpsc::UnboundedReceiver<RuntimeEvent>,
    latest_frame: Arc<Mutex<LatestFrameStore>>,
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
        let latest_frame = Arc::new(Mutex::new(LatestFrameStore::default()));
        let refresh_interval = Duration::from_millis(config.device_refresh_interval_ms);

        let resources = CoordinatorResources {
            lister,
            session_factory,
            decoder_factory,
            latest_frame: Arc::clone(&latest_frame),
            adb_path: config.adb_path.clone(),
        };
        let _coordinator = runtime.spawn(run_coordinator(
            resources,
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
        self.latest_frame.lock().ok()?.pending.take()
    }
}

impl Drop for AppRuntime {
    fn drop(&mut self) {
        let _ = self.command_tx.send(RuntimeCommand::Shutdown);
    }
}

struct CoordinatorResources {
    lister: Arc<dyn DeviceLister>,
    session_factory: Arc<dyn VideoSessionFactory>,
    decoder_factory: Arc<dyn VideoDecoderFactory>,
    latest_frame: Arc<Mutex<LatestFrameStore>>,
    adb_path: PathBuf,
}

async fn run_coordinator(
    resources: CoordinatorResources,
    refresh_interval: Duration,
    mut command_rx: mpsc::UnboundedReceiver<RuntimeCommand>,
    event_tx: mpsc::UnboundedSender<RuntimeEvent>,
) {
    let CoordinatorResources {
        lister,
        session_factory,
        decoder_factory,
        latest_frame,
        adb_path,
    } = resources;
    let mut interval = tokio::time::interval(refresh_interval);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let (worker_event_tx, mut worker_event_rx) = mpsc::unbounded_channel();
    let mut devices = Vec::new();
    let mut selected_device: Option<String> = None;
    let mut automation_state = AutomationState::Stopped;
    let mut connection_state = ConnectionState::Disconnected;
    let mut video_worker_stop: Option<Arc<AtomicBool>> = None;
    let mut input_queue: Option<InputQueue> = None;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                refresh_devices(
                    Arc::clone(&lister),
                    &event_tx,
                    DeviceRefreshState {
                        devices: &mut devices,
                        selected_device: &mut selected_device,
                        automation_state: &mut automation_state,
                        connection_state: &mut connection_state,
                        input_queue: &mut input_queue,
                        video_worker_stop: video_worker_stop.as_ref(),
                    },
                ).await;
            }
            worker_event = worker_event_rx.recv() => {
                if let Some(worker_event) = worker_event {
                    match worker_event {
                        WorkerEvent::VideoProgress(total_bytes) => {
                            send_event(&event_tx, RuntimeEvent::VideoBytesReceived(total_bytes));
                        }
                        WorkerEvent::VideoEnded(result) => {
                            if let Some(mut queue) = input_queue.take() {
                                queue.stop();
                            }
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
                        WorkerEvent::InputExecuted(command) => {
                            send_event(&event_tx, RuntimeEvent::InputExecuted(command));
                        }
                        WorkerEvent::InputFailed(message) => {
                            send_event(&event_tx, RuntimeEvent::Error(message));
                        }
                    }
                }
            }
            command = command_rx.recv() => {
                let Some(command) = command else {
                    request_video_stop(video_worker_stop.as_ref());
                    request_input_stop(input_queue.as_mut());
                    break;
                };
                match command {
                    RuntimeCommand::RefreshDevices => {
                        refresh_devices(
                            Arc::clone(&lister),
                            &event_tx,
                            DeviceRefreshState {
                                devices: &mut devices,
                                selected_device: &mut selected_device,
                                automation_state: &mut automation_state,
                                connection_state: &mut connection_state,
                                input_queue: &mut input_queue,
                                video_worker_stop: video_worker_stop.as_ref(),
                            },
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
                        if let Ok(mut latest_frame) = latest_frame.lock() {
                            *latest_frame = LatestFrameStore::default();
                        }
                        send_event(
                            &event_tx,
                            RuntimeEvent::ConnectionStateChanged(connection_state),
                        );
                        let factory = Arc::clone(&session_factory);
                        let session_serial = serial.clone();
                        match tokio::task::spawn_blocking(move || factory.start(&session_serial)).await {
                            Ok(Ok(mut session)) => {
                                let controller: Arc<dyn InputController> = Arc::new(
                                    AdbInputController::new(
                                        AdbClient::new(adb_path.clone()),
                                        serial,
                                    ),
                                );
                                let queue = match InputQueue::spawn(
                                    controller,
                                    worker_event_tx.clone(),
                                ) {
                                    Ok(queue) => queue,
                                    Err(error) => {
                                        let _ = session.stop();
                                        connection_state = ConnectionState::Disconnected;
                                        send_event(
                                            &event_tx,
                                            RuntimeEvent::ConnectionStateChanged(connection_state),
                                        );
                                        send_event(
                                            &event_tx,
                                            RuntimeEvent::Error(format!(
                                                "failed to start input worker: {error}"
                                            )),
                                        );
                                        continue;
                                    }
                                };
                                let stop = Arc::new(AtomicBool::new(false));
                                spawn_video_worker(
                                    session,
                                    Arc::clone(&decoder_factory),
                                    Arc::clone(&latest_frame),
                                    Arc::clone(&stop),
                                    worker_event_tx.clone(),
                                );
                                input_queue = Some(queue);
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
                            request_input_stop(input_queue.as_mut());
                            request_video_stop(video_worker_stop.as_ref());
                        }
                    }
                    RuntimeCommand::SubmitInput(command) => {
                        if connection_state != ConnectionState::Connected {
                            send_event(
                                &event_tx,
                                RuntimeEvent::Error("input is available only while connected".to_owned()),
                            );
                            continue;
                        }
                        let (width, height) = latest_frame
                            .lock()
                            .ok()
                            .and_then(|frame| frame.dimensions)
                            .unwrap_or((0, 0));
                        let Some(queue) = input_queue.as_ref() else {
                            send_event(
                                &event_tx,
                                RuntimeEvent::Error("input worker is not running".to_owned()),
                            );
                            continue;
                        };
                        match queue.submit(command, width, height) {
                            Ok(command) => {
                                send_event(&event_tx, RuntimeEvent::InputQueued(command));
                            }
                            Err(message) => {
                                send_event(&event_tx, RuntimeEvent::Error(message));
                            }
                        }
                    }
                    RuntimeCommand::Shutdown => {
                        request_input_stop(input_queue.as_mut());
                        request_video_stop(video_worker_stop.as_ref());
                        break;
                    }
                }
            }
        }
    }
}

struct DeviceRefreshState<'a> {
    devices: &'a mut Vec<AdbDevice>,
    selected_device: &'a mut Option<String>,
    automation_state: &'a mut AutomationState,
    connection_state: &'a mut ConnectionState,
    input_queue: &'a mut Option<InputQueue>,
    video_worker_stop: Option<&'a Arc<AtomicBool>>,
}

async fn refresh_devices(
    lister: Arc<dyn DeviceLister>,
    event_tx: &mpsc::UnboundedSender<RuntimeEvent>,
    mut state: DeviceRefreshState<'_>,
) {
    let result = tokio::task::spawn_blocking(move || lister.list_devices()).await;
    match result {
        Ok(Ok(updated_devices)) => {
            let selected_is_ready = state.selected_device.as_ref().is_some_and(|serial| {
                updated_devices
                    .iter()
                    .any(|device| &device.serial == serial && device.is_ready())
            });
            if !selected_is_ready && state.selected_device.take().is_some() {
                send_event(event_tx, RuntimeEvent::SelectedDeviceChanged(None));
                request_input_stop(state.input_queue.as_mut());
                request_video_stop(state.video_worker_stop);
                if *state.automation_state == AutomationState::Running {
                    *state.automation_state = AutomationState::Stopped;
                    *state.connection_state = ConnectionState::Disconnecting;
                    send_event(
                        event_tx,
                        RuntimeEvent::AutomationStateChanged(*state.automation_state),
                    );
                    send_event(
                        event_tx,
                        RuntimeEvent::ConnectionStateChanged(*state.connection_state),
                    );
                }
            }
            *state.devices = updated_devices;
            send_event(event_tx, RuntimeEvent::DevicesUpdated(state.devices.clone()));
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
    latest_frame: Arc<Mutex<LatestFrameStore>>,
    stop: Arc<AtomicBool>,
    worker_event_tx: mpsc::UnboundedSender<WorkerEvent>,
) {
    let _worker = tokio::task::spawn_blocking(move || {
        let mut buffer = vec![0_u8; VIDEO_BUFFER_SIZE];
        let mut total_bytes = 0_u64;
        let mut decoder = match decoder_factory.create() {
            Ok(decoder) => decoder,
            Err(error) => {
                let _ = session.stop();
                let _ = worker_event_tx.send(WorkerEvent::VideoEnded(Err(error.to_string())));
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
                    let _ = worker_event_tx.send(WorkerEvent::VideoProgress(total_bytes));
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
        let _ = worker_event_tx.send(WorkerEvent::VideoEnded(result));
    });
}

fn store_decoded_frames(
    decoder: &mut dyn VideoDecoder,
    latest_frame: &Mutex<LatestFrameStore>,
) -> Result<(), String> {
    while let Some(frame) = decoder
        .try_next_frame()
        .map_err(|error| error.to_string())?
    {
        let mut slot = latest_frame
            .lock()
            .map_err(|_| "latest frame store is unavailable".to_owned())?;
        slot.dimensions = Some((frame.width(), frame.height()));
        slot.pending = Some(frame);
    }
    Ok(())
}

struct InputQueue {
    command_tx: Option<std_mpsc::SyncSender<PixelInputCommand>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl InputQueue {
    fn spawn(
        controller: Arc<dyn InputController>,
        worker_event_tx: mpsc::UnboundedSender<WorkerEvent>,
    ) -> Result<Self, std::io::Error> {
        let (command_tx, command_rx) = std_mpsc::sync_channel(INPUT_QUEUE_SIZE);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("better-e7-input".to_owned())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    match command_rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(command) => {
                            if worker_stop.load(Ordering::Acquire) {
                                break;
                            }
                            match controller.submit(command) {
                                Ok(()) => {
                                    let _ =
                                        worker_event_tx.send(WorkerEvent::InputExecuted(command));
                                }
                                Err(error) => {
                                    let _ = worker_event_tx
                                        .send(WorkerEvent::InputFailed(error.to_string()));
                                }
                            }
                        }
                        Err(std_mpsc::RecvTimeoutError::Timeout) => {}
                        Err(std_mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })?;

        Ok(Self {
            command_tx: Some(command_tx),
            stop,
            worker: Some(worker),
        })
    }

    fn submit(
        &self,
        command: InputCommand,
        width: u32,
        height: u32,
    ) -> Result<PixelInputCommand, String> {
        if self.stop.load(Ordering::Acquire) {
            return Err("input queue has stopped".to_owned());
        }
        let command = command
            .to_pixels(width, height)
            .map_err(|error| error.to_string())?;
        let sender = self
            .command_tx
            .as_ref()
            .ok_or_else(|| "input queue has stopped".to_owned())?;
        sender.try_send(command).map_err(|error| match error {
            std_mpsc::TrySendError::Full(_) => "input queue is full".to_owned(),
            std_mpsc::TrySendError::Disconnected(_) => "input queue worker has stopped".to_owned(),
        })?;
        Ok(command)
    }

    fn request_stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.command_tx.take();
    }

    fn stop(&mut self) {
        self.request_stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for InputQueue {
    fn drop(&mut self) {
        self.stop();
    }
}

fn request_video_stop(stop: Option<&Arc<AtomicBool>>) {
    if let Some(stop) = stop {
        stop.store(true, Ordering::Release);
    }
}

fn request_input_stop(queue: Option<&mut InputQueue>) {
    if let Some(queue) = queue {
        queue.request_stop();
    }
}

fn send_event(event_tx: &mpsc::UnboundedSender<RuntimeEvent>, event: RuntimeEvent) {
    let _ = event_tx.send(event);
}

enum WorkerEvent {
    VideoProgress(u64),
    VideoEnded(Result<(), String>),
    InputExecuted(PixelInputCommand),
    InputFailed(String),
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
    use std::{
        collections::VecDeque,
        sync::mpsc::{Receiver, Sender},
        time::Instant,
    };

    use better_e7_core::{InputError, PixelFormat};
    use better_e7_video::VideoDecodeError;

    use super::*;

    struct MockDecoder {
        frames: VecDeque<Frame>,
    }

    struct BlockingInputController {
        commands: Mutex<Vec<PixelInputCommand>>,
        started_tx: Sender<()>,
        release_rx: Mutex<Receiver<()>>,
    }

    impl VideoDecoder for MockDecoder {
        fn push(&mut self, _data: &[u8]) -> Result<(), VideoDecodeError> {
            Ok(())
        }

        fn try_next_frame(&mut self) -> Result<Option<Frame>, VideoDecodeError> {
            Ok(self.frames.pop_front())
        }
    }

    impl InputController for BlockingInputController {
        fn submit(&self, command: PixelInputCommand) -> Result<(), InputError> {
            self.commands.lock().unwrap().push(command);
            let _ = self.started_tx.send(());
            let _ = self.release_rx.lock().unwrap().recv();
            Ok(())
        }
    }

    #[test]
    fn keeps_only_the_latest_decoded_frame() {
        let frames = [1_u64, 2]
            .into_iter()
            .map(|id| Frame::new(id, Instant::now(), 1, 1, PixelFormat::Rgb8, vec![0; 3]).unwrap())
            .collect();
        let mut decoder = MockDecoder { frames };
        let latest = Mutex::new(LatestFrameStore::default());

        store_decoded_frames(&mut decoder, &latest).unwrap();

        let latest = latest.lock().unwrap();
        assert_eq!(latest.pending.as_ref().unwrap().id(), 2);
        assert_eq!(latest.dimensions, Some((1, 1)));
    }

    #[test]
    fn discards_queued_input_when_stopped() {
        let (started_tx, started_rx) = std_mpsc::channel();
        let (release_tx, release_rx) = std_mpsc::channel();
        let controller = Arc::new(BlockingInputController {
            commands: Mutex::new(Vec::new()),
            started_tx,
            release_rx: Mutex::new(release_rx),
        });
        let (worker_event_tx, _worker_event_rx) = mpsc::unbounded_channel();
        let mut queue = InputQueue::spawn(controller.clone(), worker_event_tx).unwrap();

        queue
            .submit(
                InputCommand::Key {
                    android_key_code: 3,
                },
                0,
                0,
            )
            .unwrap();
        queue
            .submit(
                InputCommand::Key {
                    android_key_code: 4,
                },
                0,
                0,
            )
            .unwrap();
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        queue.request_stop();
        release_tx.send(()).unwrap();
        queue.stop();

        assert_eq!(controller.commands.lock().unwrap().len(), 1);
    }
}

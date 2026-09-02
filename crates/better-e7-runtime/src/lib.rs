use std::{
    error::Error,
    fmt,
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc as std_mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use better_e7_adb::{AdbClient, AdbDevice, AdbInputController, DeviceLister};
use better_e7_android::{ActiveVideoSession, ScrcpySessionFactory, VideoSessionFactory};
use better_e7_automation::{AutomationEngine, AutomationInput, AutomationProfile};
use better_e7_config::AppConfig;
use better_e7_core::{
    Detection, Frame, InputCommand, InputController, NormalizedRect, PixelInputCommand, Recognizer,
    VideoSource,
};
use better_e7_video::{FfmpegProcessDecoderFactory, VideoDecoder, VideoDecoderFactory};
use better_e7_vision::{ImageSequenceSource, RecognizerSet, TemplateMatcher};
use serde::Serialize;
use tokio::{
    runtime::{Builder, Runtime},
    sync::mpsc,
    time::MissedTickBehavior,
};

const VIDEO_BUFFER_SIZE: usize = 64 * 1_024;
const INPUT_QUEUE_SIZE: usize = 64;
const HISTORY_QUEUE_SIZE: usize = 256;
const OFFLINE_FRAME_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeCommand {
    RefreshDevices,
    SelectDevice(String),
    LoadAutomationProfile(PathBuf),
    SetAutomationDryRun(bool),
    StartAutomation,
    StopAutomation,
    StartOfflineAutomation {
        profile_path: PathBuf,
        frames_directory: PathBuf,
        history_path: Option<PathBuf>,
    },
    StopOfflineAutomation,
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

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeEvent {
    DevicesUpdated(Vec<AdbDevice>),
    SelectedDeviceChanged(Option<String>),
    AutomationStateChanged(AutomationState),
    ConnectionStateChanged(ConnectionState),
    VideoBytesReceived(u64),
    InputQueued(PixelInputCommand),
    InputExecuted(PixelInputCommand),
    DetectionsUpdated(Vec<Detection>),
    AutomationProfileChanged {
        name: String,
        path: PathBuf,
    },
    AutomationDryRunChanged(bool),
    AutomationRuleFired(String),
    AutomationLog(String),
    AutomationInputPlanned {
        rule_id: String,
        command: InputCommand,
    },
    OfflineAutomationStarted,
    OfflineAutomationFinished {
        processed_frames: usize,
        stopped: bool,
    },
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum OfflineAutomationEvent {
    RuleFired(String),
    Log(String),
    InputPlanned {
        rule_id: String,
        command: InputCommand,
    },
}

#[derive(Debug, Clone)]
pub struct OfflineAutomationReport {
    pub profile_name: String,
    pub processed_frames: usize,
    pub stopped: bool,
    pub last_frame: Option<Frame>,
    pub last_detections: Vec<Detection>,
    pub events: Vec<OfflineAutomationEvent>,
}

#[derive(Debug, Clone)]
pub struct OfflineAutomationOptions {
    pub frame_interval: Duration,
    pub history_path: Option<PathBuf>,
}

impl Default for OfflineAutomationOptions {
    fn default() -> Self {
        Self {
            frame_interval: OFFLINE_FRAME_INTERVAL,
            history_path: None,
        }
    }
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
        let automation = match build_automation(config) {
            Ok(automation) => automation,
            Err(error) => {
                let _ = event_tx.send(RuntimeEvent::Error(error.to_string()));
                BuiltAutomation {
                    recognizer: None,
                    engine: None,
                    profile_name: None,
                }
            }
        };
        if let (Some(name), Some(path)) = (
            automation.profile_name.clone(),
            config.automation_profile_path.clone(),
        ) {
            let _ = event_tx.send(RuntimeEvent::AutomationProfileChanged { name, path });
        }
        let latest_frame = Arc::new(Mutex::new(LatestFrameStore::default()));
        let refresh_interval = Duration::from_millis(config.device_refresh_interval_ms);

        let resources = CoordinatorResources {
            lister,
            session_factory,
            decoder_factory,
            latest_frame: Arc::clone(&latest_frame),
            adb_path: config.adb_path.clone(),
            recognizer: automation.recognizer,
            automation_engine: automation.engine,
            automation_profile_name: automation.profile_name,
            automation_dry_run: config.automation_dry_run,
            automation_history_path: config.automation_history_path.clone(),
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
    recognizer: Option<Arc<dyn Recognizer>>,
    automation_engine: Option<AutomationEngine>,
    automation_profile_name: Option<String>,
    automation_dry_run: bool,
    automation_history_path: Option<PathBuf>,
}

struct BuiltAutomation {
    recognizer: Option<Arc<dyn Recognizer>>,
    engine: Option<AutomationEngine>,
    profile_name: Option<String>,
}

fn build_automation(config: &AppConfig) -> Result<BuiltAutomation, RuntimeError> {
    let Some(profile_path) = config.automation_profile_path.as_ref() else {
        return Ok(BuiltAutomation {
            recognizer: build_legacy_recognizer(config)?,
            engine: None,
            profile_name: None,
        });
    };

    build_profile_automation(profile_path)
}

fn build_profile_automation(profile_path: &Path) -> Result<BuiltAutomation, RuntimeError> {
    let profile = AutomationProfile::load(profile_path)
        .map_err(|error| RuntimeError::AutomationProfile(error.to_string()))?;
    let mut recognizers = RecognizerSet::new();
    for template in &profile.templates {
        let path = resolve_profile_asset(profile_path, &template.path);
        let matcher = TemplateMatcher::from_path(
            template.id.clone(),
            &path,
            template.threshold,
            template
                .normalized_region()
                .map_err(|error| RuntimeError::AutomationProfile(error.to_string()))?,
        )
        .map_err(|error| RuntimeError::Recognition(error.to_string()))?;
        recognizers.add(matcher);
    }
    let profile_name = profile.name.clone();
    let engine = AutomationEngine::new(profile)
        .map_err(|error| RuntimeError::AutomationProfile(error.to_string()))?;
    Ok(BuiltAutomation {
        recognizer: Some(Arc::new(recognizers)),
        engine: Some(engine),
        profile_name: Some(profile_name),
    })
}

fn build_legacy_recognizer(
    config: &AppConfig,
) -> Result<Option<Arc<dyn Recognizer>>, RuntimeError> {
    let Some(path) = config.recognition_template_path.as_ref() else {
        return Ok(None);
    };
    let label = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("template");
    let matcher = TemplateMatcher::from_path(
        label,
        path,
        config.recognition_threshold,
        NormalizedRect::full(),
    )
    .map_err(|error| RuntimeError::Recognition(error.to_string()))?;
    Ok(Some(Arc::new(matcher)))
}

fn resolve_profile_asset(profile_path: &Path, asset_path: &Path) -> PathBuf {
    if asset_path.is_absolute() {
        asset_path.to_owned()
    } else {
        profile_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(asset_path)
    }
}

pub fn discover_offline_frames(directory: impl AsRef<Path>) -> Result<Vec<PathBuf>, RuntimeError> {
    let directory = directory.as_ref();
    let entries = fs::read_dir(directory).map_err(|error| {
        RuntimeError::Offline(format!(
            "failed to read offline frame directory {}: {error}",
            directory.display()
        ))
    })?;
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_supported_image(path))
        .collect::<Vec<_>>();
    paths.sort();
    if paths.is_empty() {
        return Err(RuntimeError::Offline(format!(
            "offline frame directory contains no PNG or JPEG images: {}",
            directory.display()
        )));
    }
    Ok(paths)
}

fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("png")
                || extension.eq_ignore_ascii_case("jpg")
                || extension.eq_ignore_ascii_case("jpeg")
        })
}

pub fn run_offline_automation(
    profile_path: impl AsRef<Path>,
    frame_paths: &[PathBuf],
    options: OfflineAutomationOptions,
) -> Result<OfflineAutomationReport, RuntimeError> {
    run_offline_automation_with_stop(profile_path.as_ref(), frame_paths, options, None)
}

fn run_offline_automation_with_stop(
    profile_path: &Path,
    frame_paths: &[PathBuf],
    options: OfflineAutomationOptions,
    stop: Option<&AtomicBool>,
) -> Result<OfflineAutomationReport, RuntimeError> {
    if frame_paths.is_empty() {
        return Err(RuntimeError::Offline(
            "offline automation requires at least one frame".to_owned(),
        ));
    }

    let BuiltAutomation {
        recognizer,
        engine,
        profile_name,
    } = build_profile_automation(profile_path)?;
    let recognizer = recognizer.expect("profile automation always has a recognizer");
    let mut engine = engine.expect("profile automation always has an engine");
    let profile_name = profile_name.expect("profile automation always has a name");
    engine.reset();

    let (history_event_tx, mut history_event_rx) = mpsc::unbounded_channel();
    let mut history_writer = options
        .history_path
        .map(|path| AutomationHistoryWriter::spawn(path, history_event_tx))
        .transpose()
        .map_err(|error| RuntimeError::Offline(format!("failed to start history writer: {error}")))?;
    let mut source = ImageSequenceSource::new(frame_paths.iter().cloned());
    source
        .start()
        .map_err(|error| RuntimeError::Offline(error.to_string()))?;

    let result = (|| {
        let mut report = OfflineAutomationReport {
            profile_name: profile_name.clone(),
            processed_frames: 0,
            stopped: false,
            last_frame: None,
            last_detections: Vec::new(),
            events: Vec::new(),
        };

        loop {
            if stop.is_some_and(|stop| stop.load(Ordering::Acquire)) {
                report.stopped = true;
                break;
            }
            let Some(frame) = source
                .try_latest_frame()
                .map_err(|error| RuntimeError::Offline(error.to_string()))?
            else {
                break;
            };
            let elapsed = options.frame_interval.saturating_mul(
                report
                    .processed_frames
                    .try_into()
                    .unwrap_or(u32::MAX),
            );
            let detections = recognizer
                .recognize(&frame)
                .map_err(|error| RuntimeError::Offline(error.to_string()))?;
            let automation = engine
                .tick(&detections, elapsed)
                .map_err(|error| RuntimeError::Offline(error.to_string()))?;

            for rule_id in automation.fired_rules {
                submit_offline_history(
                    history_writer.as_ref(),
                    AutomationHistoryRecord::new(
                        Some(&profile_name),
                        Some(elapsed),
                        &rule_id,
                        AutomationHistoryEvent::RuleFired,
                        None,
                    ),
                )?;
                report
                    .events
                    .push(OfflineAutomationEvent::RuleFired(rule_id));
            }
            for message in automation.logs {
                report.events.push(OfflineAutomationEvent::Log(message));
            }
            if let Some(input) = automation.input {
                submit_offline_history(
                    history_writer.as_ref(),
                    AutomationHistoryRecord::new(
                        Some(&profile_name),
                        Some(elapsed),
                        &input.rule_id,
                        AutomationHistoryEvent::InputPlanned,
                        Some(describe_normalized_input(&input.command)),
                    ),
                )?;
                report.events.push(OfflineAutomationEvent::InputPlanned {
                    rule_id: input.rule_id,
                    command: input.command,
                });
            }

            report.processed_frames = report.processed_frames.saturating_add(1);
            report.last_detections = detections;
            report.last_frame = Some(frame);
        }
        Ok(report)
    })();

    let stop_result = source
        .stop()
        .map_err(|error| RuntimeError::Offline(error.to_string()));
    if let Some(writer) = history_writer.as_mut() {
        writer.stop();
    }
    while let Ok(event) = history_event_rx.try_recv() {
        if let WorkerEvent::HistoryFailed(message) = event {
            return Err(RuntimeError::Offline(message));
        }
    }
    stop_result?;
    result
}

fn submit_offline_history(
    writer: Option<&AutomationHistoryWriter>,
    record: AutomationHistoryRecord,
) -> Result<(), RuntimeError> {
    if let Some(writer) = writer {
        writer
            .submit(record)
            .map_err(|message| RuntimeError::Offline(format!("failed to save history: {message}")))?;
    }
    Ok(())
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
        mut recognizer,
        mut automation_engine,
        mut automation_profile_name,
        mut automation_dry_run,
        automation_history_path,
    } = resources;
    let mut interval = tokio::time::interval(refresh_interval);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let (worker_event_tx, mut worker_event_rx) = mpsc::unbounded_channel();
    let mut history_writer = automation_history_path.and_then(|path| {
        match AutomationHistoryWriter::spawn(path, worker_event_tx.clone()) {
            Ok(writer) => Some(writer),
            Err(error) => {
                send_event(
                    &event_tx,
                    RuntimeEvent::Error(format!("failed to start history writer: {error}")),
                );
                None
            }
        }
    });
    let mut devices = Vec::new();
    let mut selected_device: Option<String> = None;
    let mut automation_state = AutomationState::Stopped;
    let mut connection_state = ConnectionState::Disconnected;
    let mut video_worker_stop: Option<Arc<AtomicBool>> = None;
    let mut input_queue: Option<InputQueue> = None;
    let mut recognition_worker: Option<RecognitionWorker> = None;
    let mut automation_started_at: Option<Instant> = None;
    let mut offline_worker_stop: Option<Arc<AtomicBool>> = None;

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
                        recognition_worker: &mut recognition_worker,
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
                            automation_started_at = None;
                            if let Some(mut worker) = recognition_worker.take() {
                                worker.stop();
                            }
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
                            send_event(&event_tx, RuntimeEvent::DetectionsUpdated(Vec::new()));
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
                        WorkerEvent::DetectionsUpdated(detections) => {
                            if connection_state == ConnectionState::Connected
                                && let (Some(engine), Some(started_at)) =
                                (automation_engine.as_mut(), automation_started_at)
                            {
                                match engine.tick(&detections, started_at.elapsed()) {
                                    Ok(report) => {
                                        for rule_id in report.fired_rules {
                                            submit_history(
                                                history_writer.as_ref(),
                                                AutomationHistoryRecord::new(
                                                    automation_profile_name.as_deref(),
                                                    Some(started_at.elapsed()),
                                                    &rule_id,
                                                    AutomationHistoryEvent::RuleFired,
                                                    None,
                                                ),
                                                &event_tx,
                                            );
                                            send_event(
                                                &event_tx,
                                                RuntimeEvent::AutomationRuleFired(rule_id),
                                            );
                                        }
                                        for message in report.logs {
                                            send_event(
                                                &event_tx,
                                                RuntimeEvent::AutomationLog(message),
                                            );
                                        }
                                        if let Some(input) = report.input {
                                            match dispatch_automation_input(
                                                input,
                                                automation_dry_run,
                                                &latest_frame,
                                                input_queue.as_ref(),
                                            ) {
                                                Ok(AutomationInputDispatch::Queued {
                                                    rule_id,
                                                    command,
                                                }) => {
                                                    let detail = describe_pixel_input(command);
                                                    submit_history(
                                                        history_writer.as_ref(),
                                                        AutomationHistoryRecord::new(
                                                            automation_profile_name.as_deref(),
                                                            Some(started_at.elapsed()),
                                                            &rule_id,
                                                            AutomationHistoryEvent::InputQueued,
                                                            Some(detail),
                                                        ),
                                                        &event_tx,
                                                    );
                                                    send_event(
                                                        &event_tx,
                                                        RuntimeEvent::InputQueued(command),
                                                    );
                                                }
                                                Ok(AutomationInputDispatch::Planned {
                                                    rule_id,
                                                    command,
                                                }) => {
                                                    let detail = describe_normalized_input(&command);
                                                    submit_history(
                                                        history_writer.as_ref(),
                                                        AutomationHistoryRecord::new(
                                                            automation_profile_name.as_deref(),
                                                            Some(started_at.elapsed()),
                                                            &rule_id,
                                                            AutomationHistoryEvent::InputPlanned,
                                                            Some(detail),
                                                        ),
                                                        &event_tx,
                                                    );
                                                    send_event(
                                                        &event_tx,
                                                        RuntimeEvent::AutomationInputPlanned {
                                                            rule_id,
                                                            command,
                                                        },
                                                    );
                                                }
                                                Err(message) => send_event(
                                                    &event_tx,
                                                    RuntimeEvent::Error(message),
                                                ),
                                            }
                                        }
                                    }
                                    Err(error) => send_event(
                                        &event_tx,
                                        RuntimeEvent::Error(format!(
                                            "automation engine failed: {error}"
                                        )),
                                    ),
                                }
                            }
                            send_event(&event_tx, RuntimeEvent::DetectionsUpdated(detections));
                        }
                        WorkerEvent::RecognitionFailed(message) => {
                            send_event(&event_tx, RuntimeEvent::Error(message));
                        }
                        WorkerEvent::OfflineEnded(result) => {
                            offline_worker_stop = None;
                            match result {
                                Ok(mut report) => {
                                    if let Some(frame) = report.last_frame.take()
                                        && let Ok(mut latest_frame) = latest_frame.lock()
                                    {
                                        latest_frame.dimensions =
                                            Some((frame.width(), frame.height()));
                                        latest_frame.pending = Some(frame);
                                    }
                                    send_event(
                                        &event_tx,
                                        RuntimeEvent::DetectionsUpdated(report.last_detections),
                                    );
                                    for event in report.events {
                                        match event {
                                            OfflineAutomationEvent::RuleFired(rule_id) => {
                                                send_event(
                                                    &event_tx,
                                                    RuntimeEvent::AutomationRuleFired(rule_id),
                                                );
                                            }
                                            OfflineAutomationEvent::Log(message) => {
                                                send_event(
                                                    &event_tx,
                                                    RuntimeEvent::AutomationLog(message),
                                                );
                                            }
                                            OfflineAutomationEvent::InputPlanned {
                                                rule_id,
                                                command,
                                            } => {
                                                send_event(
                                                    &event_tx,
                                                    RuntimeEvent::AutomationInputPlanned {
                                                        rule_id,
                                                        command,
                                                    },
                                                );
                                            }
                                        }
                                    }
                                    send_event(
                                        &event_tx,
                                        RuntimeEvent::OfflineAutomationFinished {
                                            processed_frames: report.processed_frames,
                                            stopped: report.stopped,
                                        },
                                    );
                                }
                                Err(message) => {
                                    send_event(
                                        &event_tx,
                                        RuntimeEvent::OfflineAutomationFinished {
                                            processed_frames: 0,
                                            stopped: false,
                                        },
                                    );
                                    send_event(&event_tx, RuntimeEvent::Error(message));
                                }
                            }
                        }
                        WorkerEvent::HistoryFailed(message) => {
                            if let Some(mut writer) = history_writer.take() {
                                writer.stop();
                            }
                            send_event(&event_tx, RuntimeEvent::Error(message));
                        }
                    }
                }
            }
            command = command_rx.recv() => {
                let Some(command) = command else {
                    request_video_stop(video_worker_stop.as_ref());
                    request_input_stop(input_queue.as_mut());
                    request_recognition_stop(recognition_worker.as_mut());
                    request_video_stop(offline_worker_stop.as_ref());
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
                                recognition_worker: &mut recognition_worker,
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
                    RuntimeCommand::LoadAutomationProfile(path) => {
                        if connection_state != ConnectionState::Disconnected
                            || offline_worker_stop.is_some()
                        {
                            send_event(&event_tx, RuntimeEvent::Error(
                                "stop automation before loading a profile".to_owned(),
                            ));
                            continue;
                        }
                        let result = tokio::task::spawn_blocking(move || {
                            let automation = build_profile_automation(&path);
                            (path, automation)
                        }).await;
                        match result {
                            Ok((path, Ok(automation))) => {
                                recognizer = automation.recognizer;
                                automation_engine = automation.engine;
                                let profile_name = automation
                                    .profile_name
                                    .expect("loaded profiles always have a name");
                                automation_profile_name = Some(profile_name.clone());
                                send_event(
                                    &event_tx,
                                    RuntimeEvent::AutomationProfileChanged {
                                        name: profile_name,
                                        path,
                                    },
                                );
                            }
                            Ok((_, Err(error))) => {
                                send_event(&event_tx, RuntimeEvent::Error(error.to_string()));
                            }
                            Err(error) => send_event(
                                &event_tx,
                                RuntimeEvent::Error(format!(
                                    "profile loader failed: {error}"
                                )),
                            ),
                        }
                    }
                    RuntimeCommand::SetAutomationDryRun(enabled) => {
                        if connection_state != ConnectionState::Disconnected
                            || offline_worker_stop.is_some()
                        {
                            send_event(&event_tx, RuntimeEvent::Error(
                                "stop automation before changing dry-run".to_owned(),
                            ));
                            continue;
                        }
                        automation_dry_run = enabled;
                        send_event(
                            &event_tx,
                            RuntimeEvent::AutomationDryRunChanged(enabled),
                        );
                    }
                    RuntimeCommand::StartAutomation => {
                        if connection_state != ConnectionState::Disconnected
                            || offline_worker_stop.is_some()
                        {
                            send_event(&event_tx, RuntimeEvent::Error(
                                "another automation session is already active".to_owned(),
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
                                let mut new_recognition_worker = match recognizer.as_ref() {
                                    Some(recognizer) => match RecognitionWorker::spawn(
                                        Arc::clone(recognizer),
                                        worker_event_tx.clone(),
                                    ) {
                                        Ok(worker) => Some(worker),
                                        Err(error) => {
                                            let mut queue = queue;
                                            queue.stop();
                                            let _ = session.stop();
                                            connection_state = ConnectionState::Disconnected;
                                            send_event(
                                                &event_tx,
                                                RuntimeEvent::ConnectionStateChanged(connection_state),
                                            );
                                            send_event(
                                                &event_tx,
                                                RuntimeEvent::Error(format!(
                                                    "failed to start recognition worker: {error}"
                                                )),
                                            );
                                            continue;
                                        }
                                    },
                                    None => None,
                                };
                                let recognition_sink = new_recognition_worker
                                    .as_ref()
                                    .map(RecognitionWorker::sink);
                                let stop = Arc::new(AtomicBool::new(false));
                                spawn_video_worker(
                                    session,
                                    Arc::clone(&decoder_factory),
                                    Arc::clone(&latest_frame),
                                    recognition_sink,
                                    Arc::clone(&stop),
                                    worker_event_tx.clone(),
                                );
                                input_queue = Some(queue);
                                recognition_worker = new_recognition_worker.take();
                                video_worker_stop = Some(stop);
                                if let Some(engine) = automation_engine.as_mut() {
                                    engine.reset();
                                }
                                automation_started_at = Some(Instant::now());
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
                                send_event(&event_tx, RuntimeEvent::DetectionsUpdated(Vec::new()));
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
                            request_recognition_stop(recognition_worker.as_mut());
                            request_video_stop(video_worker_stop.as_ref());
                        }
                    }
                    RuntimeCommand::StartOfflineAutomation {
                        profile_path,
                        frames_directory,
                        history_path,
                    } => {
                        if connection_state != ConnectionState::Disconnected
                            || offline_worker_stop.is_some()
                        {
                            send_event(
                                &event_tx,
                                RuntimeEvent::Error(
                                    "another automation session is already active".to_owned(),
                                ),
                            );
                            continue;
                        }
                        let stop = Arc::new(AtomicBool::new(false));
                        offline_worker_stop = Some(Arc::clone(&stop));
                        if let Ok(mut latest_frame) = latest_frame.lock() {
                            *latest_frame = LatestFrameStore::default();
                        }
                        send_event(&event_tx, RuntimeEvent::DetectionsUpdated(Vec::new()));
                        send_event(&event_tx, RuntimeEvent::OfflineAutomationStarted);
                        let completion_tx = worker_event_tx.clone();
                        let _offline_task = tokio::spawn(async move {
                            let result = tokio::task::spawn_blocking(move || {
                                let paths = discover_offline_frames(frames_directory)?;
                                run_offline_automation_with_stop(
                                    &profile_path,
                                    &paths,
                                    OfflineAutomationOptions {
                                        history_path,
                                        ..OfflineAutomationOptions::default()
                                    },
                                    Some(&stop),
                                )
                            })
                            .await
                            .map_err(|error| RuntimeError::Offline(format!(
                                "offline automation worker failed: {error}"
                            )))
                            .and_then(|result| result)
                            .map_err(|error| error.to_string());
                            let _ = completion_tx.send(WorkerEvent::OfflineEnded(result));
                        });
                    }
                    RuntimeCommand::StopOfflineAutomation => {
                        request_video_stop(offline_worker_stop.as_ref());
                    }
                    RuntimeCommand::SubmitInput(command) => {
                        if connection_state != ConnectionState::Connected {
                            send_event(
                                &event_tx,
                                RuntimeEvent::Error("input is available only while connected".to_owned()),
                            );
                            continue;
                        }
                        match queue_input(command, &latest_frame, input_queue.as_ref()) {
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
                        request_recognition_stop(recognition_worker.as_mut());
                        request_video_stop(video_worker_stop.as_ref());
                        request_video_stop(offline_worker_stop.as_ref());
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
    recognition_worker: &'a mut Option<RecognitionWorker>,
    video_worker_stop: Option<&'a Arc<AtomicBool>>,
}

async fn refresh_devices(
    lister: Arc<dyn DeviceLister>,
    event_tx: &mpsc::UnboundedSender<RuntimeEvent>,
    state: DeviceRefreshState<'_>,
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
                request_recognition_stop(state.recognition_worker.as_mut());
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
            send_event(
                event_tx,
                RuntimeEvent::DevicesUpdated(state.devices.clone()),
            );
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
    recognition_sink: Option<RecognitionSink>,
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
            if let Err(error) =
                store_decoded_frames(decoder.as_mut(), &latest_frame, recognition_sink.as_ref())
            {
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
    recognition_sink: Option<&RecognitionSink>,
) -> Result<(), String> {
    while let Some(frame) = decoder
        .try_next_frame()
        .map_err(|error| error.to_string())?
    {
        if let Some(sink) = recognition_sink {
            sink.submit(frame.clone());
        }
        let mut slot = latest_frame
            .lock()
            .map_err(|_| "latest frame store is unavailable".to_owned())?;
        slot.dimensions = Some((frame.width(), frame.height()));
        slot.pending = Some(frame);
    }
    Ok(())
}

#[derive(Default)]
struct RecognitionState {
    pending: Option<Frame>,
    stopped: bool,
}

#[derive(Clone)]
struct RecognitionSink {
    shared: Arc<(Mutex<RecognitionState>, Condvar)>,
}

impl RecognitionSink {
    fn submit(&self, frame: Frame) {
        let (state, ready) = &*self.shared;
        if let Ok(mut state) = state.lock()
            && !state.stopped
        {
            state.pending = Some(frame);
            ready.notify_one();
        }
    }

    fn request_stop(&self) {
        let (state, ready) = &*self.shared;
        if let Ok(mut state) = state.lock() {
            state.stopped = true;
            state.pending = None;
            ready.notify_all();
        }
    }
}

struct RecognitionWorker {
    sink: RecognitionSink,
    worker: Option<JoinHandle<()>>,
}

impl RecognitionWorker {
    fn spawn(
        recognizer: Arc<dyn Recognizer>,
        worker_event_tx: mpsc::UnboundedSender<WorkerEvent>,
    ) -> Result<Self, std::io::Error> {
        let sink = RecognitionSink {
            shared: Arc::new((Mutex::new(RecognitionState::default()), Condvar::new())),
        };
        let worker_sink = sink.clone();
        let worker = thread::Builder::new()
            .name("better-e7-recognition".to_owned())
            .spawn(move || {
                loop {
                    let frame = {
                        let (state, ready) = &*worker_sink.shared;
                        let mut state = match state.lock() {
                            Ok(state) => state,
                            Err(_) => break,
                        };
                        while state.pending.is_none() && !state.stopped {
                            state = match ready.wait(state) {
                                Ok(state) => state,
                                Err(_) => return,
                            };
                        }
                        if state.stopped {
                            break;
                        }
                        state.pending.take()
                    };
                    let Some(frame) = frame else {
                        continue;
                    };
                    match recognizer.recognize(&frame) {
                        Ok(detections) => {
                            let _ =
                                worker_event_tx.send(WorkerEvent::DetectionsUpdated(detections));
                        }
                        Err(error) => {
                            let _ = worker_event_tx
                                .send(WorkerEvent::RecognitionFailed(error.to_string()));
                        }
                    }
                }
            })?;
        Ok(Self {
            sink,
            worker: Some(worker),
        })
    }

    fn sink(&self) -> RecognitionSink {
        self.sink.clone()
    }

    fn request_stop(&self) {
        self.sink.request_stop();
    }

    fn stop(&mut self) {
        self.request_stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for RecognitionWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum AutomationHistoryEvent {
    RuleFired,
    InputQueued,
    InputPlanned,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct AutomationHistoryRecord {
    version: u8,
    timestamp_unix_ms: u64,
    session_elapsed_ms: Option<u64>,
    profile: Option<String>,
    rule_id: String,
    event: AutomationHistoryEvent,
    detail: Option<String>,
}

impl AutomationHistoryRecord {
    fn new(
        profile: Option<&str>,
        session_elapsed: Option<Duration>,
        rule_id: &str,
        event: AutomationHistoryEvent,
        detail: Option<String>,
    ) -> Self {
        Self {
            version: 1,
            timestamp_unix_ms: milliseconds_since_epoch(),
            session_elapsed_ms: session_elapsed.map(duration_milliseconds),
            profile: profile.map(str::to_owned),
            rule_id: rule_id.to_owned(),
            event,
            detail,
        }
    }
}

fn milliseconds_since_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(duration_milliseconds)
        .unwrap_or(0)
}

fn duration_milliseconds(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

struct AutomationHistoryWriter {
    record_tx: Option<std_mpsc::SyncSender<AutomationHistoryRecord>>,
    worker: Option<JoinHandle<()>>,
}

impl AutomationHistoryWriter {
    fn spawn(
        path: PathBuf,
        worker_event_tx: mpsc::UnboundedSender<WorkerEvent>,
    ) -> Result<Self, std::io::Error> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let (record_tx, record_rx) = std_mpsc::sync_channel(HISTORY_QUEUE_SIZE);
        let worker = thread::Builder::new()
            .name("better-e7-history".to_owned())
            .spawn(move || {
                let mut writer = BufWriter::new(file);
                while let Ok(record) = record_rx.recv() {
                    if let Err(error) = write_history_record(&mut writer, &record) {
                        let _ = worker_event_tx.send(WorkerEvent::HistoryFailed(format!(
                            "history writer failed: {error}"
                        )));
                        break;
                    }
                }
            })?;
        Ok(Self {
            record_tx: Some(record_tx),
            worker: Some(worker),
        })
    }

    fn submit(&self, record: AutomationHistoryRecord) -> Result<(), String> {
        let sender = self
            .record_tx
            .as_ref()
            .ok_or_else(|| "history writer has stopped".to_owned())?;
        sender.try_send(record).map_err(|error| match error {
            std_mpsc::TrySendError::Full(_) => "history queue is full".to_owned(),
            std_mpsc::TrySendError::Disconnected(_) => "history writer has stopped".to_owned(),
        })
    }

    fn stop(&mut self) {
        self.record_tx.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for AutomationHistoryWriter {
    fn drop(&mut self) {
        self.stop();
    }
}

fn write_history_record(
    writer: &mut impl Write,
    record: &AutomationHistoryRecord,
) -> Result<(), std::io::Error> {
    serde_json::to_writer(&mut *writer, record).map_err(std::io::Error::other)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

fn submit_history(
    writer: Option<&AutomationHistoryWriter>,
    record: AutomationHistoryRecord,
    event_tx: &mpsc::UnboundedSender<RuntimeEvent>,
) {
    if let Some(writer) = writer
        && let Err(message) = writer.submit(record)
    {
        send_event(
            event_tx,
            RuntimeEvent::Error(format!("failed to save automation history: {message}")),
        );
    }
}

fn describe_normalized_input(command: &InputCommand) -> String {
    match command {
        InputCommand::Tap { point } => format!("tap {:.3} {:.3}", point.x(), point.y()),
        InputCommand::Swipe { from, to, duration } => format!(
            "swipe {:.3} {:.3} {:.3} {:.3} {}ms",
            from.x(),
            from.y(),
            to.x(),
            to.y(),
            duration.as_millis()
        ),
        InputCommand::Key { android_key_code } => format!("keyevent {android_key_code}"),
    }
}

fn describe_pixel_input(command: PixelInputCommand) -> String {
    match command {
        PixelInputCommand::Tap { x, y } => format!("tap {x} {y}"),
        PixelInputCommand::Swipe {
            from_x,
            from_y,
            to_x,
            to_y,
            duration,
        } => format!(
            "swipe {from_x} {from_y} {to_x} {to_y} {}ms",
            duration.as_millis()
        ),
        PixelInputCommand::Key { android_key_code } => format!("keyevent {android_key_code}"),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum AutomationInputDispatch {
    Queued {
        rule_id: String,
        command: PixelInputCommand,
    },
    Planned {
        rule_id: String,
        command: InputCommand,
    },
}

fn dispatch_automation_input(
    input: AutomationInput,
    dry_run: bool,
    latest_frame: &Mutex<LatestFrameStore>,
    input_queue: Option<&InputQueue>,
) -> Result<AutomationInputDispatch, String> {
    if dry_run {
        return Ok(AutomationInputDispatch::Planned {
            rule_id: input.rule_id,
            command: input.command,
        });
    }
    let rule_id = input.rule_id;
    queue_input(input.command, latest_frame, input_queue)
        .map(|command| AutomationInputDispatch::Queued {
            rule_id: rule_id.clone(),
            command,
        })
        .map_err(|message| format!("automation rule {rule_id} failed to queue input: {message}"))
}

fn queue_input(
    command: InputCommand,
    latest_frame: &Mutex<LatestFrameStore>,
    input_queue: Option<&InputQueue>,
) -> Result<PixelInputCommand, String> {
    let (width, height) = latest_frame
        .lock()
        .map_err(|_| "latest frame store is unavailable".to_owned())?
        .dimensions
        .unwrap_or((0, 0));
    let queue = input_queue.ok_or_else(|| "input worker is not running".to_owned())?;
    queue.submit(command, width, height)
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

fn request_recognition_stop(worker: Option<&mut RecognitionWorker>) {
    if let Some(worker) = worker {
        worker.request_stop();
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
    DetectionsUpdated(Vec<Detection>),
    RecognitionFailed(String),
    OfflineEnded(Result<OfflineAutomationReport, String>),
    HistoryFailed(String),
}

#[derive(Debug)]
pub enum RuntimeError {
    Build(std::io::Error),
    AutomationProfile(String),
    Recognition(String),
    Offline(String),
    Stopped,
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Build(error) => write!(formatter, "failed to build runtime: {error}"),
            Self::AutomationProfile(message) => {
                write!(
                    formatter,
                    "failed to configure automation profile: {message}"
                )
            }
            Self::Recognition(message) => {
                write!(formatter, "failed to configure recognition: {message}")
            }
            Self::Offline(message) => write!(formatter, "offline automation failed: {message}"),
            Self::Stopped => formatter.write_str("runtime has stopped"),
        }
    }
}

impl Error for RuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Build(error) => Some(error),
            Self::AutomationProfile(_)
            | Self::Recognition(_)
            | Self::Offline(_)
            | Self::Stopped => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        fs,
        sync::mpsc::{Receiver, Sender},
        time::{Instant, SystemTime},
    };

    use better_e7_core::{InputError, PixelFormat};
    use better_e7_video::VideoDecodeError;
    use image::RgbImage;

    use super::*;

    struct MockDecoder {
        frames: VecDeque<Frame>,
    }

    struct BlockingInputController {
        commands: Mutex<Vec<PixelInputCommand>>,
        started_tx: Sender<()>,
        release_rx: Mutex<Receiver<()>>,
    }

    struct BlockingRecognizer {
        frame_ids: Mutex<Vec<u64>>,
        started_tx: Sender<u64>,
        release_first_rx: Mutex<Receiver<()>>,
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

    impl Recognizer for BlockingRecognizer {
        fn recognize(
            &self,
            frame: &Frame,
        ) -> Result<Vec<Detection>, better_e7_core::RecognitionError> {
            self.frame_ids.lock().unwrap().push(frame.id());
            let _ = self.started_tx.send(frame.id());
            if frame.id() == 1 {
                let _ = self.release_first_rx.lock().unwrap().recv();
            }
            Ok(Vec::new())
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

        store_decoded_frames(&mut decoder, &latest, None).unwrap();

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

    #[test]
    fn recognition_worker_replaces_an_unprocessed_frame() {
        let (started_tx, started_rx) = std_mpsc::channel();
        let (release_tx, release_rx) = std_mpsc::channel();
        let recognizer = Arc::new(BlockingRecognizer {
            frame_ids: Mutex::new(Vec::new()),
            started_tx,
            release_first_rx: Mutex::new(release_rx),
        });
        let (worker_event_tx, _worker_event_rx) = mpsc::unbounded_channel();
        let mut worker = RecognitionWorker::spawn(recognizer.clone(), worker_event_tx).unwrap();
        let sink = worker.sink();
        let frame =
            |id| Frame::new(id, Instant::now(), 1, 1, PixelFormat::Rgb8, vec![0; 3]).unwrap();

        sink.submit(frame(1));
        assert_eq!(started_rx.recv_timeout(Duration::from_secs(1)).unwrap(), 1);
        sink.submit(frame(2));
        sink.submit(frame(3));
        release_tx.send(()).unwrap();
        assert_eq!(started_rx.recv_timeout(Duration::from_secs(1)).unwrap(), 3);
        worker.stop();

        assert_eq!(*recognizer.frame_ids.lock().unwrap(), [1, 3]);
    }

    #[test]
    fn builds_a_profile_and_ticks_it_without_android() {
        let profile_path = std::env::temp_dir().join(format!(
            "better-e7-runtime-profile-{}.toml",
            std::process::id()
        ));
        fs::write(
            &profile_path,
            r#"
                name = "mock-profile"

                [[rules]]
                id = "go-home"

                [rules.condition]
                type = "always"

                [rules.action]
                type = "key"
                android_key_code = 3
            "#,
        )
        .unwrap();
        let config = AppConfig {
            automation_profile_path: Some(profile_path.clone()),
            ..AppConfig::default()
        };

        let mut automation = build_automation(&config).unwrap();
        let frame = Frame::new(1, Instant::now(), 1, 1, PixelFormat::Rgb8, vec![0; 3]).unwrap();
        let detections = automation
            .recognizer
            .as_ref()
            .unwrap()
            .recognize(&frame)
            .unwrap();
        let report = automation
            .engine
            .as_mut()
            .unwrap()
            .tick(&detections, Duration::ZERO)
            .unwrap();
        let input = report.input.unwrap();
        let (started_tx, started_rx) = std_mpsc::channel();
        let (release_tx, release_rx) = std_mpsc::channel();
        let controller = Arc::new(BlockingInputController {
            commands: Mutex::new(Vec::new()),
            started_tx,
            release_rx: Mutex::new(release_rx),
        });
        let (worker_event_tx, _worker_event_rx) = mpsc::unbounded_channel();
        let mut queue = InputQueue::spawn(controller.clone(), worker_event_tx).unwrap();
        let latest_frame = Mutex::new(LatestFrameStore {
            pending: None,
            dimensions: Some((1, 1)),
        });
        let queued = dispatch_automation_input(input, false, &latest_frame, Some(&queue)).unwrap();
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        assert_eq!(automation.profile_name.as_deref(), Some("mock-profile"));
        assert!(detections.is_empty());
        assert_eq!(
            queued,
            AutomationInputDispatch::Queued {
                rule_id: "go-home".to_owned(),
                command: PixelInputCommand::Key {
                    android_key_code: 3
                }
            }
        );
        release_tx.send(()).unwrap();
        queue.stop();
        let _ = fs::remove_file(profile_path);
    }

    #[test]
    fn dry_run_plans_input_without_an_input_queue() {
        let input = AutomationInput {
            rule_id: "go-home".to_owned(),
            command: InputCommand::Key {
                android_key_code: 3,
            },
        };
        let latest_frame = Mutex::new(LatestFrameStore::default());

        let dispatch = dispatch_automation_input(input, true, &latest_frame, None).unwrap();

        assert_eq!(
            dispatch,
            AutomationInputDispatch::Planned {
                rule_id: "go-home".to_owned(),
                command: InputCommand::Key {
                    android_key_code: 3
                }
            }
        );
    }

    #[test]
    fn runs_a_sorted_image_sequence_without_android() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("better-e7-offline-{suffix}"));
        let frames_directory = root.join("frames");
        let profile_path = root.join("automation.toml");
        let history_path = root.join("history.jsonl");
        fs::create_dir_all(&frames_directory).unwrap();
        fs::write(
            &profile_path,
            r#"
                name = "offline-profile"

                [[rules]]
                id = "go-home"

                [rules.condition]
                type = "always"

                [rules.action]
                type = "key"
                android_key_code = 3
            "#,
        )
        .unwrap();
        RgbImage::from_raw(1, 1, vec![20, 20, 20])
            .unwrap()
            .save(frames_directory.join("02.png"))
            .unwrap();
        RgbImage::from_raw(1, 1, vec![10, 10, 10])
            .unwrap()
            .save(frames_directory.join("01.png"))
            .unwrap();
        fs::write(frames_directory.join("ignored.txt"), "not an image").unwrap();

        let paths = discover_offline_frames(&frames_directory).unwrap();
        let report = run_offline_automation(
            &profile_path,
            &paths,
            OfflineAutomationOptions {
                history_path: Some(history_path.clone()),
                ..OfflineAutomationOptions::default()
            },
        )
        .unwrap();

        assert_eq!(paths[0].file_name().unwrap().to_string_lossy(), "01.png");
        assert_eq!(paths[1].file_name().unwrap().to_string_lossy(), "02.png");
        assert_eq!(report.profile_name, "offline-profile");
        assert_eq!(report.processed_frames, 2);
        assert!(!report.stopped);
        assert_eq!(report.last_frame.as_ref().unwrap().id(), 1);
        assert!(report.last_detections.is_empty());
        assert_eq!(report.events.len(), 4);
        assert_eq!(
            report.events[0],
            OfflineAutomationEvent::RuleFired("go-home".to_owned())
        );
        assert_eq!(
            report.events[1],
            OfflineAutomationEvent::InputPlanned {
                rule_id: "go-home".to_owned(),
                command: InputCommand::Key {
                    android_key_code: 3
                }
            }
        );

        let history = fs::read_to_string(history_path).unwrap();
        let records = history
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 4);
        assert_eq!(records[0]["event"], "rule_fired");
        assert_eq!(records[1]["event"], "input_planned");
        assert_eq!(records[2]["session_elapsed_ms"], 100);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn writes_automation_history_as_ordered_json_lines() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("better-e7-automation-history-{suffix}.jsonl"));
        let (worker_event_tx, mut worker_event_rx) = mpsc::unbounded_channel();
        let mut writer = AutomationHistoryWriter::spawn(path.clone(), worker_event_tx).unwrap();
        writer
            .submit(AutomationHistoryRecord::new(
                Some("mock-profile"),
                Some(Duration::from_millis(10)),
                "go-home",
                AutomationHistoryEvent::RuleFired,
                None,
            ))
            .unwrap();
        writer
            .submit(AutomationHistoryRecord::new(
                Some("mock-profile"),
                Some(Duration::from_millis(11)),
                "go-home",
                AutomationHistoryEvent::InputQueued,
                Some("keyevent 3".to_owned()),
            ))
            .unwrap();
        writer.stop();

        let contents = fs::read_to_string(&path).unwrap();
        let records = contents
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["version"], 1);
        assert_eq!(records[0]["profile"], "mock-profile");
        assert_eq!(records[0]["rule_id"], "go-home");
        assert_eq!(records[0]["event"], "rule_fired");
        assert_eq!(records[0]["session_elapsed_ms"], 10);
        assert_eq!(records[1]["event"], "input_queued");
        assert_eq!(records[1]["detail"], "keyevent 3");
        assert!(worker_event_rx.try_recv().is_err());
        fs::remove_file(path).unwrap();
    }
}

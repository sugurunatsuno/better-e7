use std::{error::Error, fmt, sync::Arc, time::Duration};

use better_e7_adb::{AdbClient, AdbDevice, DeviceLister};
use better_e7_config::AppConfig;
use tokio::{
    runtime::{Builder, Runtime},
    sync::mpsc,
    time::MissedTickBehavior,
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEvent {
    DevicesUpdated(Vec<AdbDevice>),
    SelectedDeviceChanged(Option<String>),
    AutomationStateChanged(AutomationState),
    Error(String),
}

pub struct AppRuntime {
    command_tx: mpsc::UnboundedSender<RuntimeCommand>,
    event_rx: mpsc::UnboundedReceiver<RuntimeEvent>,
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
        let refresh_interval = Duration::from_millis(config.device_refresh_interval_ms);

        let _coordinator = runtime.spawn(run_coordinator(
            lister,
            refresh_interval,
            command_rx,
            event_tx,
        ));

        Ok(Self {
            command_tx,
            event_rx,
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
}

impl Drop for AppRuntime {
    fn drop(&mut self) {
        let _ = self.command_tx.send(RuntimeCommand::Shutdown);
    }
}

async fn run_coordinator(
    lister: Arc<dyn DeviceLister>,
    refresh_interval: Duration,
    mut command_rx: mpsc::UnboundedReceiver<RuntimeCommand>,
    event_tx: mpsc::UnboundedSender<RuntimeEvent>,
) {
    let mut interval = tokio::time::interval(refresh_interval);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut devices = Vec::new();
    let mut selected_device: Option<String> = None;
    let mut automation_state = AutomationState::Stopped;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                refresh_devices(
                    Arc::clone(&lister),
                    &event_tx,
                    &mut devices,
                    &mut selected_device,
                    &mut automation_state,
                ).await;
            }
            command = command_rx.recv() => {
                let Some(command) = command else {
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
                        ).await;
                    }
                    RuntimeCommand::SelectDevice(serial) => {
                        if automation_state == AutomationState::Running {
                            send_event(&event_tx, RuntimeEvent::Error(
                                "stop automation before changing the device".to_owned(),
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
                        let ready = selected_device.as_ref().is_some_and(|serial| {
                            devices.iter().any(|device| &device.serial == serial && device.is_ready())
                        });
                        if ready {
                            automation_state = AutomationState::Running;
                            send_event(
                                &event_tx,
                                RuntimeEvent::AutomationStateChanged(automation_state),
                            );
                        } else {
                            send_event(&event_tx, RuntimeEvent::Error(
                                "select a ready device before starting automation".to_owned(),
                            ));
                        }
                    }
                    RuntimeCommand::StopAutomation => {
                        automation_state = AutomationState::Stopped;
                        send_event(
                            &event_tx,
                            RuntimeEvent::AutomationStateChanged(automation_state),
                        );
                    }
                    RuntimeCommand::Shutdown => break,
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
                if *automation_state == AutomationState::Running {
                    *automation_state = AutomationState::Stopped;
                    send_event(
                        event_tx,
                        RuntimeEvent::AutomationStateChanged(*automation_state),
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

fn send_event(event_tx: &mpsc::UnboundedSender<RuntimeEvent>, event: RuntimeEvent) {
    let _ = event_tx.send(event);
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

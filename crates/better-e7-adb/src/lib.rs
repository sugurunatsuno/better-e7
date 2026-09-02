use std::{error::Error, fmt, io, path::PathBuf, process::Command, sync::Arc};

use better_e7_core::{InputController, InputError, PixelInputCommand};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbDevice {
    pub serial: String,
    pub state: AdbDeviceState,
    pub product: Option<String>,
    pub model: Option<String>,
    pub device: Option<String>,
    pub transport_id: Option<String>,
}

impl AdbDevice {
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        matches!(self.state, AdbDeviceState::Device)
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        self.model.as_deref().unwrap_or(&self.serial)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdbDeviceState {
    Device,
    Offline,
    Unauthorized,
    Unknown(String),
}

impl fmt::Display for AdbDeviceState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device => formatter.write_str("device"),
            Self::Offline => formatter.write_str("offline"),
            Self::Unauthorized => formatter.write_str("unauthorized"),
            Self::Unknown(state) => formatter.write_str(state),
        }
    }
}

pub trait DeviceLister: Send + Sync {
    fn list_devices(&self) -> Result<Vec<AdbDevice>, AdbError>;
}

#[derive(Debug, Clone)]
pub struct AdbClient {
    executable: PathBuf,
}

impl AdbClient {
    #[must_use]
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }
}

impl DeviceLister for AdbClient {
    fn list_devices(&self) -> Result<Vec<AdbDevice>, AdbError> {
        let output = Command::new(&self.executable)
            .args(["devices", "-l"])
            .output()
            .map_err(AdbError::Spawn)?;

        if !output.status.success() {
            return Err(AdbError::CommandFailed {
                status: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }

        let stdout = String::from_utf8(output.stdout).map_err(AdbError::InvalidUtf8)?;
        parse_devices_output(&stdout)
    }
}

pub trait AdbCommandRunner: Send + Sync {
    fn run_shell_input(&self, serial: &str, arguments: &[String]) -> Result<(), AdbError>;
}

impl AdbCommandRunner for AdbClient {
    fn run_shell_input(&self, serial: &str, arguments: &[String]) -> Result<(), AdbError> {
        let output = Command::new(&self.executable)
            .args(["-s", serial, "shell", "input"])
            .args(arguments)
            .output()
            .map_err(AdbError::Spawn)?;

        if output.status.success() {
            Ok(())
        } else {
            Err(AdbError::CommandFailed {
                status: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            })
        }
    }
}

pub struct AdbInputController {
    runner: Arc<dyn AdbCommandRunner>,
    serial: String,
}

impl AdbInputController {
    #[must_use]
    pub fn new(client: AdbClient, serial: impl Into<String>) -> Self {
        Self::with_runner(Arc::new(client), serial)
    }

    #[must_use]
    pub fn with_runner(runner: Arc<dyn AdbCommandRunner>, serial: impl Into<String>) -> Self {
        Self {
            runner,
            serial: serial.into(),
        }
    }
}

impl InputController for AdbInputController {
    fn submit(&self, command: PixelInputCommand) -> Result<(), InputError> {
        self.runner
            .run_shell_input(&self.serial, &input_arguments(command))
            .map_err(|error| InputError(error.to_string()))
    }
}

fn input_arguments(command: PixelInputCommand) -> Vec<String> {
    match command {
        PixelInputCommand::Tap { x, y } => vec!["tap".to_owned(), x.to_string(), y.to_string()],
        PixelInputCommand::Swipe {
            from_x,
            from_y,
            to_x,
            to_y,
            duration,
        } => vec![
            "swipe".to_owned(),
            from_x.to_string(),
            from_y.to_string(),
            to_x.to_string(),
            to_y.to_string(),
            duration.as_millis().min(u128::from(u64::MAX)).to_string(),
        ],
        PixelInputCommand::Key { android_key_code } => {
            vec!["keyevent".to_owned(), android_key_code.to_string()]
        }
    }
}

pub fn parse_devices_output(output: &str) -> Result<Vec<AdbDevice>, AdbError> {
    let mut lines = output.lines();
    let Some(header) = lines.next() else {
        return Err(AdbError::UnexpectedOutput("output was empty".to_owned()));
    };
    if !header.starts_with("List of devices attached") {
        return Err(AdbError::UnexpectedOutput(header.to_owned()));
    }

    let mut devices = Vec::new();
    for line in lines.map(str::trim).filter(|line| !line.is_empty()) {
        let mut fields = line.split_whitespace();
        let Some(serial) = fields.next() else {
            continue;
        };
        let Some(state) = fields.next() else {
            return Err(AdbError::UnexpectedOutput(line.to_owned()));
        };

        let mut parsed = AdbDevice {
            serial: serial.to_owned(),
            state: parse_state(state),
            product: None,
            model: None,
            device: None,
            transport_id: None,
        };

        for field in fields {
            let Some((key, value)) = field.split_once(':') else {
                continue;
            };
            match key {
                "product" => parsed.product = Some(value.to_owned()),
                "model" => parsed.model = Some(value.replace('_', " ")),
                "device" => parsed.device = Some(value.to_owned()),
                "transport_id" => parsed.transport_id = Some(value.to_owned()),
                _ => {}
            }
        }
        devices.push(parsed);
    }

    Ok(devices)
}

fn parse_state(state: &str) -> AdbDeviceState {
    match state {
        "device" => AdbDeviceState::Device,
        "offline" => AdbDeviceState::Offline,
        "unauthorized" => AdbDeviceState::Unauthorized,
        other => AdbDeviceState::Unknown(other.to_owned()),
    }
}

#[derive(Debug)]
pub enum AdbError {
    Spawn(io::Error),
    CommandFailed { status: Option<i32>, stderr: String },
    InvalidUtf8(std::string::FromUtf8Error),
    UnexpectedOutput(String),
}

impl fmt::Display for AdbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(formatter, "failed to start adb: {error}"),
            Self::CommandFailed { status, stderr } => {
                write!(formatter, "adb exited with {status:?}: {stderr}")
            }
            Self::InvalidUtf8(error) => write!(formatter, "adb returned invalid UTF-8: {error}"),
            Self::UnexpectedOutput(line) => write!(formatter, "unexpected adb output: {line}"),
        }
    }
}

impl Error for AdbError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Spawn(error) => Some(error),
            Self::InvalidUtf8(error) => Some(error),
            Self::CommandFailed { .. } | Self::UnexpectedOutput(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Mutex, time::Duration};

    use super::*;

    #[derive(Default)]
    struct MockRunner {
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl AdbCommandRunner for MockRunner {
        fn run_shell_input(&self, serial: &str, arguments: &[String]) -> Result<(), AdbError> {
            self.calls
                .lock()
                .unwrap()
                .push((serial.to_owned(), arguments.to_vec()));
            Ok(())
        }
    }

    #[test]
    fn parses_adb_devices() {
        let output = "List of devices attached\nR58M123 device product:beyond model:Galaxy_S10 device:beyond transport_id:1\nemulator-5554 offline transport_id:2\n";
        let devices = parse_devices_output(output).unwrap();

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].serial, "R58M123");
        assert_eq!(devices[0].model.as_deref(), Some("Galaxy S10"));
        assert!(devices[0].is_ready());
        assert_eq!(devices[1].state, AdbDeviceState::Offline);
    }

    #[test]
    fn rejects_an_invalid_header() {
        let result = parse_devices_output("adb server is out of date\n");
        assert!(matches!(result, Err(AdbError::UnexpectedOutput(_))));
    }

    #[test]
    fn submits_tap_swipe_and_key_commands() {
        let runner = Arc::new(MockRunner::default());
        let controller = AdbInputController::with_runner(runner.clone(), "device-1");

        controller
            .submit(PixelInputCommand::Tap { x: 120, y: 340 })
            .unwrap();
        controller
            .submit(PixelInputCommand::Swipe {
                from_x: 10,
                from_y: 20,
                to_x: 30,
                to_y: 40,
                duration: Duration::from_millis(250),
            })
            .unwrap();
        controller
            .submit(PixelInputCommand::Key {
                android_key_code: 4,
            })
            .unwrap();

        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls[0], ("device-1".to_owned(), vec!["tap", "120", "340"]));
        assert_eq!(
            calls[1],
            (
                "device-1".to_owned(),
                vec!["swipe", "10", "20", "30", "40", "250"]
            )
        );
        assert_eq!(calls[2], ("device-1".to_owned(), vec!["keyevent", "4"]));
    }
}

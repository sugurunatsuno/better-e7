use std::{
    error::Error,
    fmt,
    io::{self, Read},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    thread,
    time::Duration,
};

use better_e7_config::AppConfig;

pub const SCRCPY_VERSION: &str = "4.1";
pub const REMOTE_SERVER_PATH: &str = "/data/local/tmp/better-e7-scrcpy-server.jar";
const DEVICE_SOCKET: &str = "localabstract:scrcpy";

pub trait ActiveVideoSession: Send {
    fn read_video(&mut self, buffer: &mut [u8]) -> Result<usize, SessionError>;
    fn stop(&mut self) -> Result<(), SessionError>;
}

pub trait VideoSessionFactory: Send + Sync {
    fn start(&self, serial: &str) -> Result<Box<dyn ActiveVideoSession>, SessionError>;
}

pub trait ServerProcess: Send {
    fn stop(&mut self) -> Result<(), String>;
}

pub trait ScrcpyBackend: Send + Sync {
    fn push_server(
        &self,
        serial: &str,
        local_path: &Path,
        remote_path: &str,
    ) -> Result<(), String>;
    fn create_forward(&self, serial: &str, local_port: u16) -> Result<(), String>;
    fn start_server(
        &self,
        serial: &str,
        options: &ScrcpySessionOptions,
    ) -> Result<Box<dyn ServerProcess>, String>;
    fn connect_video(&self, local_port: u16) -> Result<Box<dyn Read + Send>, String>;
    fn remove_forward(&self, serial: &str, local_port: u16) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrcpySessionOptions {
    pub server_path: PathBuf,
    pub local_port: u16,
    pub max_size: u32,
}

impl From<&AppConfig> for ScrcpySessionOptions {
    fn from(config: &AppConfig) -> Self {
        Self {
            server_path: config.scrcpy_server_path.clone(),
            local_port: config.scrcpy_local_port,
            max_size: config.scrcpy_max_size,
        }
    }
}

pub struct ScrcpySessionFactory {
    backend: Arc<dyn ScrcpyBackend>,
    options: ScrcpySessionOptions,
}

impl ScrcpySessionFactory {
    #[must_use]
    pub fn new(config: &AppConfig) -> Self {
        Self::with_backend(
            Arc::new(AdbScrcpyBackend::new(config.adb_path.clone())),
            ScrcpySessionOptions::from(config),
        )
    }

    #[must_use]
    pub fn with_backend(
        backend: Arc<dyn ScrcpyBackend>,
        options: ScrcpySessionOptions,
    ) -> Self {
        Self { backend, options }
    }
}

impl VideoSessionFactory for ScrcpySessionFactory {
    fn start(&self, serial: &str) -> Result<Box<dyn ActiveVideoSession>, SessionError> {
        self.backend
            .push_server(serial, &self.options.server_path, REMOTE_SERVER_PATH)
            .map_err(|message| SessionError::new(SessionStage::PushServer, message))?;
        self.backend
            .create_forward(serial, self.options.local_port)
            .map_err(|message| SessionError::new(SessionStage::CreateForward, message))?;

        let mut process = match self.backend.start_server(serial, &self.options) {
            Ok(process) => process,
            Err(message) => {
                let _ = self.backend.remove_forward(serial, self.options.local_port);
                return Err(SessionError::new(SessionStage::StartServer, message));
            }
        };

        let video = match self.backend.connect_video(self.options.local_port) {
            Ok(video) => video,
            Err(message) => {
                let _ = process.stop();
                let _ = self.backend.remove_forward(serial, self.options.local_port);
                return Err(SessionError::new(SessionStage::ConnectVideo, message));
            }
        };

        Ok(Box::new(ScrcpySession {
            backend: Arc::clone(&self.backend),
            serial: serial.to_owned(),
            local_port: self.options.local_port,
            video: Some(video),
            process: Some(process),
            stopped: false,
        }))
    }
}

struct ScrcpySession {
    backend: Arc<dyn ScrcpyBackend>,
    serial: String,
    local_port: u16,
    video: Option<Box<dyn Read + Send>>,
    process: Option<Box<dyn ServerProcess>>,
    stopped: bool,
}

impl ActiveVideoSession for ScrcpySession {
    fn read_video(&mut self, buffer: &mut [u8]) -> Result<usize, SessionError> {
        let video = self
            .video
            .as_mut()
            .ok_or_else(|| SessionError::new(SessionStage::ReadVideo, "session is stopped"))?;
        video
            .read(buffer)
            .map_err(|error| {
                let retryable = matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                );
                SessionError::with_retryable(
                    SessionStage::ReadVideo,
                    error.to_string(),
                    retryable,
                )
            })
    }

    fn stop(&mut self) -> Result<(), SessionError> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        self.video.take();

        let process_result = self.process.as_mut().map_or(Ok(()), |process| {
            process
                .stop()
                .map_err(|message| SessionError::new(SessionStage::StopServer, message))
        });
        self.process.take();
        let forward_result = self
            .backend
            .remove_forward(&self.serial, self.local_port)
            .map_err(|message| SessionError::new(SessionStage::RemoveForward, message));

        process_result.and(forward_result)
    }
}

impl Drop for ScrcpySession {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[derive(Debug, Clone)]
pub struct AdbScrcpyBackend {
    adb_path: PathBuf,
    connect_attempts: usize,
    retry_delay: Duration,
}

impl AdbScrcpyBackend {
    #[must_use]
    pub fn new(adb_path: impl Into<PathBuf>) -> Self {
        Self {
            adb_path: adb_path.into(),
            connect_attempts: 100,
            retry_delay: Duration::from_millis(100),
        }
    }

    fn run_adb(&self, serial: &str, arguments: &[String]) -> Result<(), String> {
        let output = Command::new(&self.adb_path)
            .arg("-s")
            .arg(serial)
            .args(arguments)
            .output()
            .map_err(|error| format!("failed to start adb: {error}"))?;
        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "adb exited with {:?}: {}",
            output.status.code(),
            stderr.trim()
        ))
    }
}

impl ScrcpyBackend for AdbScrcpyBackend {
    fn push_server(
        &self,
        serial: &str,
        local_path: &Path,
        remote_path: &str,
    ) -> Result<(), String> {
        self.run_adb(
            serial,
            &[
                "push".to_owned(),
                local_path.to_string_lossy().into_owned(),
                remote_path.to_owned(),
            ],
        )
    }

    fn create_forward(&self, serial: &str, local_port: u16) -> Result<(), String> {
        self.run_adb(
            serial,
            &[
                "forward".to_owned(),
                format!("tcp:{local_port}"),
                DEVICE_SOCKET.to_owned(),
            ],
        )
    }

    fn start_server(
        &self,
        serial: &str,
        options: &ScrcpySessionOptions,
    ) -> Result<Box<dyn ServerProcess>, String> {
        let child = Command::new(&self.adb_path)
            .arg("-s")
            .arg(serial)
            .arg("shell")
            .arg(format!("CLASSPATH={REMOTE_SERVER_PATH}"))
            .arg("app_process")
            .arg("/")
            .arg("com.genymobile.scrcpy.Server")
            .arg(SCRCPY_VERSION)
            .arg("tunnel_forward=true")
            .arg("audio=false")
            .arg("control=false")
            .arg("cleanup=false")
            .arg("raw_stream=true")
            .arg("video_codec=h264")
            .arg(format!("max_size={}", options.max_size))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("failed to start scrcpy-server: {error}"))?;
        Ok(Box::new(ChildServerProcess { child }))
    }

    fn connect_video(&self, local_port: u16) -> Result<Box<dyn Read + Send>, String> {
        let mut last_error = None;
        for _ in 0..self.connect_attempts {
            match TcpStream::connect(("127.0.0.1", local_port)) {
                Ok(stream) => {
                    stream
                        .set_read_timeout(Some(Duration::from_millis(500)))
                        .map_err(|error| format!("failed to set video timeout: {error}"))?;
                    return Ok(Box::new(stream));
                }
                Err(error) => {
                    last_error = Some(error);
                    thread::sleep(self.retry_delay);
                }
            }
        }
        Err(format!(
            "failed to connect to video socket: {}",
            last_error.map_or_else(|| "unknown error".to_owned(), |error| error.to_string())
        ))
    }

    fn remove_forward(&self, serial: &str, local_port: u16) -> Result<(), String> {
        self.run_adb(
            serial,
            &[
                "forward".to_owned(),
                "--remove".to_owned(),
                format!("tcp:{local_port}"),
            ],
        )
    }
}

struct ChildServerProcess {
    child: Child,
}

impl ServerProcess for ChildServerProcess {
    fn stop(&mut self) -> Result<(), String> {
        if self
            .child
            .try_wait()
            .map_err(|error| format!("failed to inspect scrcpy-server: {error}"))?
            .is_some()
        {
            return Ok(());
        }
        self.child
            .kill()
            .map_err(|error| format!("failed to stop scrcpy-server: {error}"))?;
        self.child
            .wait()
            .map_err(|error| format!("failed to wait for scrcpy-server: {error}"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStage {
    PushServer,
    CreateForward,
    StartServer,
    ConnectVideo,
    ReadVideo,
    StopServer,
    RemoveForward,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionError {
    pub stage: SessionStage,
    pub message: String,
    retryable: bool,
}

impl SessionError {
    fn new(stage: SessionStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
            retryable: false,
        }
    }

    fn with_retryable(
        stage: SessionStage,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            stage,
            message: message.into(),
            retryable,
        }
    }

    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        self.retryable
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.stage, self.message)
    }
}

impl Error for SessionError {}

#[cfg(test)]
mod tests {
    use std::{
        io::Cursor,
        sync::{Arc, Mutex},
    };

    use super::*;

    struct MockBackend {
        calls: Arc<Mutex<Vec<&'static str>>>,
        video: Vec<u8>,
        fail_connect: bool,
    }

    impl ScrcpyBackend for MockBackend {
        fn push_server(
            &self,
            _serial: &str,
            _local_path: &Path,
            _remote_path: &str,
        ) -> Result<(), String> {
            self.calls.lock().unwrap().push("push");
            Ok(())
        }

        fn create_forward(&self, _serial: &str, _local_port: u16) -> Result<(), String> {
            self.calls.lock().unwrap().push("forward");
            Ok(())
        }

        fn start_server(
            &self,
            _serial: &str,
            _options: &ScrcpySessionOptions,
        ) -> Result<Box<dyn ServerProcess>, String> {
            self.calls.lock().unwrap().push("start");
            Ok(Box::new(MockProcess {
                calls: Arc::clone(&self.calls),
            }))
        }

        fn connect_video(&self, _local_port: u16) -> Result<Box<dyn Read + Send>, String> {
            self.calls.lock().unwrap().push("connect");
            if self.fail_connect {
                return Err("mock connection failed".to_owned());
            }
            Ok(Box::new(Cursor::new(self.video.clone())))
        }

        fn remove_forward(&self, _serial: &str, _local_port: u16) -> Result<(), String> {
            self.calls.lock().unwrap().push("remove");
            Ok(())
        }
    }

    struct MockProcess {
        calls: Arc<Mutex<Vec<&'static str>>>,
    }

    impl ServerProcess for MockProcess {
        fn stop(&mut self) -> Result<(), String> {
            self.calls.lock().unwrap().push("stop");
            Ok(())
        }
    }

    fn options() -> ScrcpySessionOptions {
        ScrcpySessionOptions {
            server_path: PathBuf::from("mock-server"),
            local_port: 27_183,
            max_size: 1_920,
        }
    }

    #[test]
    fn runs_the_session_lifecycle_and_reads_video() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = Arc::new(MockBackend {
            calls: Arc::clone(&calls),
            video: vec![0, 0, 0, 1, 0x67],
            fail_connect: false,
        });
        let factory = ScrcpySessionFactory::with_backend(backend, options());
        let mut session = factory.start("mock-device").unwrap();
        let mut buffer = [0_u8; 8];

        assert_eq!(session.read_video(&mut buffer).unwrap(), 5);
        assert_eq!(&buffer[..5], &[0, 0, 0, 1, 0x67]);
        session.stop().unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            ["push", "forward", "start", "connect", "stop", "remove"]
        );
    }

    #[test]
    fn cleans_up_when_video_connection_fails() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = Arc::new(MockBackend {
            calls: Arc::clone(&calls),
            video: Vec::new(),
            fail_connect: true,
        });
        let factory = ScrcpySessionFactory::with_backend(backend, options());
        let error = match factory.start("mock-device") {
            Ok(_) => panic!("session unexpectedly started"),
            Err(error) => error,
        };

        assert_eq!(error.stage, SessionStage::ConnectVideo);
        assert_eq!(
            *calls.lock().unwrap(),
            ["push", "forward", "start", "connect", "stop", "remove"]
        );
    }
}

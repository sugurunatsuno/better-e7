use std::{error::Error, fmt, fs, io, path::Path, path::PathBuf};

use serde::{Deserialize, Serialize};

const MINIMUM_REFRESH_INTERVAL_MS: u64 = 250;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub adb_path: PathBuf,
    pub ffmpeg_path: PathBuf,
    pub device_refresh_interval_ms: u64,
    pub scrcpy_server_path: PathBuf,
    pub scrcpy_local_port: u16,
    pub scrcpy_max_size: u32,
    pub automation_profile_path: Option<PathBuf>,
    pub automation_dry_run: bool,
    pub recognition_template_path: Option<PathBuf>,
    pub recognition_threshold: f32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            adb_path: PathBuf::from("adb"),
            ffmpeg_path: PathBuf::from("ffmpeg"),
            device_refresh_interval_ms: 2_000,
            scrcpy_server_path: PathBuf::from("third_party/scrcpy/scrcpy-server-v4.1"),
            scrcpy_local_port: 27_183,
            scrcpy_max_size: 1_920,
            automation_profile_path: None,
            automation_dry_run: false,
            recognition_template_path: None,
            recognition_threshold: 0.9,
        }
    }
}

impl AppConfig {
    pub fn load_or_create(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        match fs::read_to_string(path) {
            Ok(contents) => Self::from_toml(&contents),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let config = Self::default();
                config.save(path)?;
                Ok(config)
            }
            Err(error) => Err(ConfigError::Io(error)),
        }
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        self.validate()?;
        let contents = toml::to_string_pretty(self).map_err(ConfigError::Serialize)?;
        fs::write(path, contents).map_err(ConfigError::Io)
    }

    pub fn from_toml(contents: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(contents).map_err(ConfigError::Parse)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.ffmpeg_path.as_os_str().is_empty() {
            return Err(ConfigError::Invalid(
                "ffmpeg_path must not be empty".to_owned(),
            ));
        }
        if self.device_refresh_interval_ms < MINIMUM_REFRESH_INTERVAL_MS {
            return Err(ConfigError::Invalid(format!(
                "device_refresh_interval_ms must be at least {MINIMUM_REFRESH_INTERVAL_MS}"
            )));
        }
        if self.scrcpy_local_port == 0 {
            return Err(ConfigError::Invalid(
                "scrcpy_local_port must not be zero".to_owned(),
            ));
        }
        if !(0.0..=1.0).contains(&self.recognition_threshold) {
            return Err(ConfigError::Invalid(
                "recognition_threshold must be within 0.0..=1.0".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(io::Error),
    Parse(toml::de::Error),
    Serialize(toml::ser::Error),
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "configuration I/O failed: {error}"),
            Self::Parse(error) => write!(formatter, "configuration parse failed: {error}"),
            Self::Serialize(error) => {
                write!(formatter, "configuration serialization failed: {error}")
            }
            Self::Invalid(message) => write!(formatter, "invalid configuration: {message}"),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Parse(error) => Some(error),
            Self::Serialize(error) => Some(error),
            Self::Invalid(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_configuration() {
        let config = AppConfig::from_toml(
            r#"
                adb_path = "/opt/android/adb"
                device_refresh_interval_ms = 1500
            "#,
        )
        .unwrap();

        assert_eq!(config.adb_path, PathBuf::from("/opt/android/adb"));
        assert_eq!(config.ffmpeg_path, PathBuf::from("ffmpeg"));
        assert_eq!(config.automation_profile_path, None);
        assert!(!config.automation_dry_run);
        assert_eq!(config.recognition_template_path, None);
        assert_eq!(config.recognition_threshold, 0.9);
        assert_eq!(config.device_refresh_interval_ms, 1_500);
        assert_eq!(
            config.scrcpy_server_path,
            PathBuf::from("third_party/scrcpy/scrcpy-server-v4.1")
        );
    }

    #[test]
    fn rejects_an_interval_that_is_too_short() {
        let result = AppConfig::from_toml("device_refresh_interval_ms = 100");
        assert!(matches!(result, Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn rejects_an_empty_ffmpeg_path() {
        let result = AppConfig::from_toml("ffmpeg_path = \"\"");
        assert!(matches!(result, Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn rejects_an_invalid_recognition_threshold() {
        let result = AppConfig::from_toml("recognition_threshold = 1.1");
        assert!(matches!(result, Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn parses_automation_dry_run() {
        let config = AppConfig::from_toml("automation_dry_run = true").unwrap();
        assert!(config.automation_dry_run);
    }
}

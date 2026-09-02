use std::{collections::BTreeSet, error::Error, fmt, fs, io, path::Path};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationProfile {
    pub name: String,
    #[serde(default)]
    pub rules: Vec<AutomationRule>,
}

impl AutomationProfile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ProfileError> {
        let contents = fs::read_to_string(path).map_err(ProfileError::Io)?;
        Self::from_toml(&contents)
    }

    pub fn from_toml(contents: &str) -> Result<Self, ProfileError> {
        let profile: Self = toml::from_str(contents).map_err(ProfileError::Parse)?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ProfileError> {
        self.validate()?;
        let contents = toml::to_string_pretty(self).map_err(ProfileError::Serialize)?;
        fs::write(path, contents).map_err(ProfileError::Io)
    }

    pub fn validate(&self) -> Result<(), ProfileError> {
        if self.name.trim().is_empty() {
            return Err(ProfileError::Invalid(
                "profile name must not be empty".to_owned(),
            ));
        }
        let mut ids = BTreeSet::new();
        for rule in &self.rules {
            validate_id(&rule.id)?;
            if !ids.insert(rule.id.as_str()) {
                return Err(ProfileError::Invalid(format!(
                    "rule id is duplicated: {}",
                    rule.id
                )));
            }
            rule.condition.validate()?;
            rule.action.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationRule {
    pub id: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub cooldown_ms: u64,
    #[serde(default = "default_consume")]
    pub consume: bool,
    pub condition: Condition,
    pub action: Action,
}

const fn default_enabled() -> bool {
    true
}

const fn default_consume() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Condition {
    Always,
    DetectionPresent {
        label: String,
        #[serde(default = "default_minimum_confidence")]
        minimum_confidence: f32,
    },
    DetectionAbsent {
        label: String,
        #[serde(default = "default_minimum_confidence")]
        minimum_confidence: f32,
    },
    All {
        conditions: Vec<Condition>,
    },
    Any {
        conditions: Vec<Condition>,
    },
    Not {
        condition: Box<Condition>,
    },
}

const fn default_minimum_confidence() -> f32 {
    0.9
}

impl Condition {
    fn validate(&self) -> Result<(), ProfileError> {
        match self {
            Self::Always => Ok(()),
            Self::DetectionPresent {
                label,
                minimum_confidence,
            }
            | Self::DetectionAbsent {
                label,
                minimum_confidence,
            } => {
                validate_label(label)?;
                validate_confidence(*minimum_confidence)
            }
            Self::All { conditions } | Self::Any { conditions } => {
                if conditions.is_empty() {
                    return Err(ProfileError::Invalid(
                        "condition group must not be empty".to_owned(),
                    ));
                }
                conditions.iter().try_for_each(Self::validate)
            }
            Self::Not { condition } => condition.validate(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    TapDetection {
        label: String,
    },
    Tap {
        x: f32,
        y: f32,
    },
    Swipe {
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        duration_ms: u64,
    },
    Key {
        android_key_code: u32,
    },
    Log {
        message: String,
    },
}

impl Action {
    fn validate(&self) -> Result<(), ProfileError> {
        match self {
            Self::TapDetection { label } => validate_label(label),
            Self::Tap { x, y } => validate_point(*x, *y),
            Self::Swipe {
                from_x,
                from_y,
                to_x,
                to_y,
                duration_ms,
            } => {
                validate_point(*from_x, *from_y)?;
                validate_point(*to_x, *to_y)?;
                if *duration_ms == 0 {
                    return Err(ProfileError::Invalid(
                        "swipe duration_ms must not be zero".to_owned(),
                    ));
                }
                Ok(())
            }
            Self::Key { .. } => Ok(()),
            Self::Log { message } => {
                if message.trim().is_empty() {
                    Err(ProfileError::Invalid(
                        "log message must not be empty".to_owned(),
                    ))
                } else {
                    Ok(())
                }
            }
        }
    }
}

fn validate_id(value: &str) -> Result<(), ProfileError> {
    if value.is_empty()
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_.".contains(&byte)
        })
    {
        return Err(ProfileError::Invalid(format!(
            "rule id must use lowercase ASCII letters, digits, '-', '_' or '.': {value}"
        )));
    }
    Ok(())
}

fn validate_label(label: &str) -> Result<(), ProfileError> {
    if label.trim().is_empty() {
        Err(ProfileError::Invalid(
            "detection label must not be empty".to_owned(),
        ))
    } else {
        Ok(())
    }
}

fn validate_confidence(value: f32) -> Result<(), ProfileError> {
    if (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(ProfileError::Invalid(format!(
            "minimum_confidence must be within 0.0..=1.0: {value}"
        )))
    }
}

fn validate_point(x: f32, y: f32) -> Result<(), ProfileError> {
    if (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y) {
        Ok(())
    } else {
        Err(ProfileError::Invalid(format!(
            "normalized point must be within 0.0..=1.0: {x}, {y}"
        )))
    }
}

#[derive(Debug)]
pub enum ProfileError {
    Io(io::Error),
    Parse(toml::de::Error),
    Serialize(toml::ser::Error),
    Invalid(String),
}

impl fmt::Display for ProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "profile I/O failed: {error}"),
            Self::Parse(error) => write!(formatter, "profile parse failed: {error}"),
            Self::Serialize(error) => write!(formatter, "profile serialization failed: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid profile: {message}"),
        }
    }
}

impl Error for ProfileError {
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
    fn parses_the_example_profile() {
        let profile =
            AutomationProfile::from_toml(include_str!("../../../automation.example.toml")).unwrap();

        assert_eq!(profile.name, "confirm-and-recover");
        assert_eq!(profile.rules.len(), 2);
        assert_eq!(profile.rules[0].id, "confirm");
        assert!(matches!(
            profile.rules[0].action,
            Action::TapDetection { .. }
        ));
    }

    #[test]
    fn rejects_duplicate_rule_ids() {
        let profile = AutomationProfile {
            name: "duplicate".to_owned(),
            rules: vec![rule("same"), rule("same")],
        };

        assert!(matches!(profile.validate(), Err(ProfileError::Invalid(_))));
    }

    #[test]
    fn rejects_invalid_normalized_coordinates() {
        let mut invalid = rule("invalid");
        invalid.action = Action::Tap { x: 1.1, y: 0.5 };
        let profile = AutomationProfile {
            name: "invalid".to_owned(),
            rules: vec![invalid],
        };

        assert!(matches!(profile.validate(), Err(ProfileError::Invalid(_))));
    }

    #[test]
    fn serializes_and_parses_a_profile() {
        let profile = AutomationProfile {
            name: "roundtrip".to_owned(),
            rules: vec![rule("log")],
        };

        let serialized = toml::to_string(&profile).unwrap();
        let parsed = AutomationProfile::from_toml(&serialized).unwrap();

        assert_eq!(parsed, profile);
    }

    fn rule(id: &str) -> AutomationRule {
        AutomationRule {
            id: id.to_owned(),
            enabled: true,
            priority: 0,
            cooldown_ms: 0,
            consume: true,
            condition: Condition::Always,
            action: Action::Log {
                message: "matched".to_owned(),
            },
        }
    }
}

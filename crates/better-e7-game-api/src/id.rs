use std::{error::Error, fmt};

macro_rules! identifier {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                validate_id(&value)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

identifier!(GameId);
identifier!(StateId);
identifier!(TaskId);
identifier!(TriggerId);

fn validate_id(value: &str) -> Result<(), IdError> {
    if value.is_empty() {
        return Err(IdError::Empty);
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_.".contains(&byte))
    {
        return Err(IdError::InvalidCharacter(value.to_owned()));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdError {
    Empty,
    InvalidCharacter(String),
}

impl fmt::Display for IdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("identifier must not be empty"),
            Self::InvalidCharacter(value) => write!(
                formatter,
                "identifier may contain lowercase ASCII letters, digits, '-', '_' and '.': {value}"
            ),
        }
    }
}

impl Error for IdError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_stable_namespaced_ids() {
        assert_eq!(
            GameId::new("epic-seven.global").unwrap().as_str(),
            "epic-seven.global"
        );
    }

    #[test]
    fn rejects_display_text_as_an_id() {
        assert!(GameId::new("Epic Seven").is_err());
        assert!(TaskId::new("").is_err());
    }
}

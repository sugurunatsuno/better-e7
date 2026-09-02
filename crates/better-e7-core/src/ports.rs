use std::{error::Error, fmt, time::Duration};

use crate::{Frame, NormalizedPoint, NormalizedRect};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputCommand {
    Tap {
        point: NormalizedPoint,
    },
    Swipe {
        from: NormalizedPoint,
        to: NormalizedPoint,
        duration: Duration,
    },
    Key {
        android_key_code: u32,
    },
}

impl InputCommand {
    pub fn to_pixels(self, width: u32, height: u32) -> Result<PixelInputCommand, InputError> {
        match self {
            Self::Tap { point } => {
                validate_input_dimensions(width, height)?;
                let (x, y) = point.to_pixels(width, height);
                Ok(PixelInputCommand::Tap { x, y })
            }
            Self::Swipe { from, to, duration } => {
                validate_input_dimensions(width, height)?;
                let (from_x, from_y) = from.to_pixels(width, height);
                let (to_x, to_y) = to.to_pixels(width, height);
                Ok(PixelInputCommand::Swipe {
                    from_x,
                    from_y,
                    to_x,
                    to_y,
                    duration,
                })
            }
            Self::Key { android_key_code } => Ok(PixelInputCommand::Key { android_key_code }),
        }
    }
}

fn validate_input_dimensions(width: u32, height: u32) -> Result<(), InputError> {
    if width == 0 || height == 0 {
        Err(InputError(
            "input target dimensions must not be zero".to_owned(),
        ))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelInputCommand {
    Tap {
        x: u32,
        y: u32,
    },
    Swipe {
        from_x: u32,
        from_y: u32,
        to_x: u32,
        to_y: u32,
        duration: Duration,
    },
    Key {
        android_key_code: u32,
    },
}

pub trait InputController: Send + Sync {
    fn submit(&self, command: PixelInputCommand) -> Result<(), InputError>;
}

pub trait VideoSource: Send {
    fn start(&mut self) -> Result<(), VideoSourceError>;
    fn try_latest_frame(&mut self) -> Result<Option<Frame>, VideoSourceError>;
    fn stop(&mut self) -> Result<(), VideoSourceError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    pub label: String,
    pub confidence: f32,
    pub center: NormalizedPoint,
    pub bounds: NormalizedRect,
}

pub trait Recognizer: Send + Sync {
    fn recognize(&self, frame: &Frame) -> Result<Vec<Detection>, RecognitionError>;
}

macro_rules! string_error {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name(pub String);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl Error for $name {}
    };
}

string_error!(InputError);
string_error!(VideoSourceError);
string_error!(RecognitionError);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_normalized_input_to_pixels() {
        let point = NormalizedPoint::new(0.5, 1.0).unwrap();
        let command = InputCommand::Tap { point }.to_pixels(100, 50).unwrap();

        assert_eq!(command, PixelInputCommand::Tap { x: 50, y: 49 });
    }

    #[test]
    fn rejects_input_without_target_dimensions() {
        let command = InputCommand::Tap {
            point: NormalizedPoint::new(0.5, 0.5).unwrap(),
        };

        assert!(command.to_pixels(0, 1_080).is_err());
    }
}

use std::{error::Error, fmt, time::Duration};

use crate::{Frame, NormalizedPoint};

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

pub trait InputController: Send + Sync {
    fn submit(&self, command: InputCommand) -> Result<(), InputError>;
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


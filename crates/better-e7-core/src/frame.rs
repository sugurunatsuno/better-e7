use std::{error::Error, fmt, sync::Arc, time::Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb8,
    Rgba8,
}

impl PixelFormat {
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgb8 => 3,
            Self::Rgba8 => 4,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Frame {
    id: u64,
    captured_at: Instant,
    width: u32,
    height: u32,
    format: PixelFormat,
    pixels: Arc<[u8]>,
}

impl Frame {
    pub fn new(
        id: u64,
        captured_at: Instant,
        width: u32,
        height: u32,
        format: PixelFormat,
        pixels: impl Into<Arc<[u8]>>,
    ) -> Result<Self, FrameError> {
        let pixels = pixels.into();
        let expected = width as usize * height as usize * format.bytes_per_pixel();
        if pixels.len() != expected {
            return Err(FrameError::UnexpectedBufferLength {
                expected,
                actual: pixels.len(),
            });
        }

        Ok(Self {
            id,
            captured_at,
            width,
            height,
            format,
            pixels,
        })
    }

    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    #[must_use]
    pub const fn captured_at(&self) -> Instant {
        self.captured_at
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub const fn format(&self) -> PixelFormat {
        self.format
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    UnexpectedBufferLength { expected: usize, actual: usize },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedBufferLength { expected, actual } => {
                write!(f, "expected {expected} bytes, received {actual}")
            }
        }
    }
}

impl Error for FrameError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_an_invalid_buffer_length() {
        let result = Frame::new(1, Instant::now(), 2, 2, PixelFormat::Rgba8, vec![0; 15]);
        assert_eq!(
            result.unwrap_err(),
            FrameError::UnexpectedBufferLength {
                expected: 16,
                actual: 15
            }
        );
    }
}


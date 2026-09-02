use std::{error::Error, fmt};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizedPoint {
    x: f32,
    y: f32,
}

impl NormalizedPoint {
    pub fn new(x: f32, y: f32) -> Result<Self, PointError> {
        if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
            return Err(PointError { x, y });
        }
        Ok(Self { x, y })
    }

    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    #[must_use]
    pub fn to_pixels(self, width: u32, height: u32) -> (u32, u32) {
        let x = (self.x * width.saturating_sub(1) as f32).round() as u32;
        let y = (self.y * height.saturating_sub(1) as f32).round() as u32;
        (x, y)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointError {
    pub x: f32,
    pub y: f32,
}

impl fmt::Display for PointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "normalized point must be within 0.0..=1.0: ({}, {})",
            self.x, self.y
        )
    }
}

impl Error for PointError {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizedRect {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl NormalizedRect {
    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> Result<Self, RectError> {
        let values_are_normalized = [left, top, right, bottom]
            .into_iter()
            .all(|value| (0.0..=1.0).contains(&value));
        if !values_are_normalized || left >= right || top >= bottom {
            return Err(RectError::InvalidBounds {
                left,
                top,
                right,
                bottom,
            });
        }
        Ok(Self {
            left,
            top,
            right,
            bottom,
        })
    }

    #[must_use]
    pub const fn full() -> Self {
        Self {
            left: 0.0,
            top: 0.0,
            right: 1.0,
            bottom: 1.0,
        }
    }

    #[must_use]
    pub const fn left(self) -> f32 {
        self.left
    }

    #[must_use]
    pub const fn top(self) -> f32 {
        self.top
    }

    #[must_use]
    pub const fn right(self) -> f32 {
        self.right
    }

    #[must_use]
    pub const fn bottom(self) -> f32 {
        self.bottom
    }

    #[must_use]
    pub fn center(self) -> NormalizedPoint {
        NormalizedPoint {
            x: (self.left + self.right) * 0.5,
            y: (self.top + self.bottom) * 0.5,
        }
    }

    pub fn to_pixels(self, width: u32, height: u32) -> Result<PixelRect, RectError> {
        if width == 0 || height == 0 {
            return Err(RectError::ZeroTargetDimensions);
        }
        let left = (self.left * width as f32).floor() as u32;
        let top = (self.top * height as f32).floor() as u32;
        let right = (self.right * width as f32).ceil().min(width as f32) as u32;
        let bottom = (self.bottom * height as f32).ceil().min(height as f32) as u32;
        Ok(PixelRect {
            x: left,
            y: top,
            width: right.saturating_sub(left),
            height: bottom.saturating_sub(top),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PixelRect {
    #[must_use]
    pub const fn right(self) -> u32 {
        self.x.saturating_add(self.width)
    }

    #[must_use]
    pub const fn bottom(self) -> u32 {
        self.y.saturating_add(self.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RectError {
    InvalidBounds {
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
    },
    ZeroTargetDimensions,
}

impl fmt::Display for RectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBounds {
                left,
                top,
                right,
                bottom,
            } => write!(
                formatter,
                "normalized rectangle is invalid: {left}, {top}, {right}, {bottom}"
            ),
            Self::ZeroTargetDimensions => {
                formatter.write_str("rectangle target dimensions must not be zero")
            }
        }
    }
}

impl Error for RectError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_edges_to_pixel_coordinates() {
        assert_eq!(
            NormalizedPoint::new(0.0, 0.0).unwrap().to_pixels(100, 50),
            (0, 0)
        );
        assert_eq!(
            NormalizedPoint::new(1.0, 1.0).unwrap().to_pixels(100, 50),
            (99, 49)
        );
    }

    #[test]
    fn rejects_points_outside_the_screen() {
        assert!(NormalizedPoint::new(-0.1, 0.5).is_err());
        assert!(NormalizedPoint::new(0.5, 1.1).is_err());
    }

    #[test]
    fn converts_a_normalized_rectangle_to_pixels() {
        let rect = NormalizedRect::new(0.25, 0.2, 0.75, 0.8).unwrap();

        assert_eq!(
            rect.to_pixels(200, 100).unwrap(),
            PixelRect {
                x: 50,
                y: 20,
                width: 100,
                height: 60,
            }
        );
        assert_eq!(rect.center(), NormalizedPoint::new(0.5, 0.5).unwrap());
    }

    #[test]
    fn rejects_an_inverted_rectangle() {
        assert!(NormalizedRect::new(0.8, 0.1, 0.2, 0.9).is_err());
    }
}

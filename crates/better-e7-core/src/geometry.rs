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
}

use crate::GeometryError;

/// A point in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    pub fn validate(self) -> Result<(), GeometryError> {
        if self.is_finite() {
            Ok(())
        } else {
            Err(GeometryError::NonFinite)
        }
    }

    /// Adds a logical-pixel offset, rejecting non-finite inputs and overflow.
    pub fn checked_offset(self, dx: f32, dy: f32) -> Result<Self, GeometryError> {
        self.validate()?;
        if !dx.is_finite() || !dy.is_finite() {
            return Err(GeometryError::NonFinite);
        }

        let result = Self::new(self.x + dx, self.y + dy);
        if !result.is_finite() {
            return Err(GeometryError::ArithmeticOverflow);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::{GeometryError, Point};

    #[test]
    fn point_accepts_finite_logical_pixels() {
        assert_eq!(Point::new(1.25, -2.5).validate(), Ok(()));
    }

    #[test]
    fn point_rejects_non_finite_values_and_overflow() {
        assert_eq!(
            Point::new(f32::NAN, 0.0).validate(),
            Err(GeometryError::NonFinite)
        );
        assert_eq!(
            Point::new(f32::MAX, 0.0).checked_offset(f32::MAX, 0.0),
            Err(GeometryError::ArithmeticOverflow)
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn point_round_trips_through_serde() {
        let point = Point::new(12.5, -3.0);
        let encoded = serde_json::to_string(&point).unwrap();
        assert_eq!(serde_json::from_str::<Point>(&encoded).unwrap(), point);
    }
}

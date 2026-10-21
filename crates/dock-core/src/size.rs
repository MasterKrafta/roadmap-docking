use crate::GeometryError;

/// A width and height in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const ZERO: Self = Self { width: 0.0, height: 0.0 };

    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    pub fn is_finite(self) -> bool {
        self.width.is_finite() && self.height.is_finite()
    }

    /// Validates a non-negative size. Empty sizes are allowed where meaningful.
    pub fn validate(self) -> Result<(), GeometryError> {
        if !self.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        if self.width < 0.0 || self.height < 0.0 {
            return Err(GeometryError::NegativeDimension);
        }
        Ok(())
    }

    /// Validates a size whose width and height must both be positive.
    pub fn validate_positive(self) -> Result<(), GeometryError> {
        self.validate()?;
        if self.width <= 0.0 || self.height <= 0.0 {
            return Err(GeometryError::NonPositiveDimension);
        }
        Ok(())
    }

    /// Multiplies the dimensions, rejecting non-finite results.
    pub fn checked_area(self) -> Result<f32, GeometryError> {
        self.validate()?;
        let area = self.width * self.height;
        if !area.is_finite() {
            return Err(GeometryError::ArithmeticOverflow);
        }
        Ok(area)
    }
}

#[cfg(test)]
mod tests {
    use super::{GeometryError, Size};

    #[test]
    fn size_can_be_empty_but_never_negative() {
        assert_eq!(Size::ZERO.validate(), Ok(()));
        assert_eq!(Size::new(-1.0, 2.0).validate(), Err(GeometryError::NegativeDimension));
        assert_eq!(Size::new(0.0, 2.0).validate_positive(), Err(GeometryError::NonPositiveDimension));
    }

    #[test]
    fn size_rejects_non_finite_values_and_area_overflow() {
        assert_eq!(Size::new(f32::INFINITY, 1.0).validate(), Err(GeometryError::NonFinite));
        assert_eq!(
            Size::new(f32::MAX, 2.0).checked_area(),
            Err(GeometryError::ArithmeticOverflow)
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn size_round_trips_through_serde() {
        let size = Size::new(720.0, 480.0);
        let encoded = serde_json::to_string(&size).unwrap();
        assert_eq!(serde_json::from_str::<Size>(&encoded).unwrap(), size);
    }
}

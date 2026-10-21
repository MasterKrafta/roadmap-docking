use crate::{GeometryError, Size};

/// Size constraints copied from a panel descriptor when a panel is opened.
///
/// This type contains layout hints only; it does not hold the panel's UI or
/// content state.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PanelConstraints {
    pub min_size: Size,
    pub preferred_size: Size,
    pub max_size: Option<Size>,
    pub singleton: bool,
}

impl PanelConstraints {
    pub const fn new(
        min_size: Size,
        preferred_size: Size,
        max_size: Option<Size>,
        singleton: bool,
    ) -> Self {
        Self { min_size, preferred_size, max_size, singleton }
    }

    /// Checks that all configured dimensions are positive and consistently ordered.
    pub fn validate(self) -> Result<(), GeometryError> {
        self.min_size.validate_positive()?;
        self.preferred_size.validate_positive()?;
        if let Some(max_size) = self.max_size {
            max_size.validate_positive()?;
        }

        if self.min_size.width > self.preferred_size.width
            || self.min_size.height > self.preferred_size.height
        {
            return Err(GeometryError::NonPositiveDimension);
        }

        if let Some(max_size) = self.max_size {
            if self.preferred_size.width > max_size.width
                || self.preferred_size.height > max_size.height
                || self.min_size.width > max_size.width
                || self.min_size.height > max_size.height
            {
                return Err(GeometryError::NonPositiveDimension);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{GeometryError, Size};
    use super::PanelConstraints;

    #[test]
    fn accepts_ordered_positive_size_constraints() {
        let constraints = PanelConstraints::new(
            Size::new(120.0, 80.0),
            Size::new(280.0, 220.0),
            Some(Size::new(640.0, 480.0)),
            false,
        );
        assert_eq!(constraints.validate(), Ok(()));
    }

    #[test]
    fn rejects_inconsistent_or_non_finite_constraints() {
        let constraints = PanelConstraints::new(
            Size::new(300.0, 80.0),
            Size::new(280.0, 220.0),
            None,
            false,
        );
        assert_eq!(constraints.validate(), Err(GeometryError::NonPositiveDimension));

        let constraints = PanelConstraints::new(
            Size::new(1.0, 1.0),
            Size::new(f32::INFINITY, 10.0),
            None,
            true,
        );
        assert_eq!(constraints.validate(), Err(GeometryError::NonFinite));
    }
}

use crate::{GeometryError, Size};

/// Layout-only state remembered for one panel instance.
///
/// This deliberately excludes content state such as scroll position, selection,
/// expanded sections, or any application-specific state. Such state belongs to
/// the panel implementation, not dock-core.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PanelState {
    last_known_size: Option<Size>,
}

impl PanelState {
    pub const fn new(last_known_size: Option<Size>) -> Self {
        Self { last_known_size }
    }

    pub const fn last_known_size(&self) -> Option<Size> {
        self.last_known_size
    }

    /// Validates the remembered size when present.
    ///
    /// A last-known size represents actual logical geometry, so both
    /// dimensions must be finite and strictly positive.
    pub fn validate(self) -> Result<(), GeometryError> {
        if let Some(size) = self.last_known_size {
            size.validate_positive()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::PanelState;
    use crate::{GeometryError, Size};

    #[test]
    fn default_state_has_no_remembered_size() {
        let state = PanelState::default();
        assert_eq!(state.last_known_size(), None);
        assert_eq!(state.validate(), Ok(()));
    }

    #[test]
    fn accepts_a_valid_last_known_size() {
        let state = PanelState::new(Some(Size::new(280.0, 420.0)));
        assert_eq!(state.last_known_size(), Some(Size::new(280.0, 420.0)));
        assert_eq!(state.validate(), Ok(()));
    }

    #[test]
    fn rejects_invalid_last_known_sizes() {
        for size in [
            Size::new(0.0, 10.0),
            Size::new(-1.0, 10.0),
            Size::new(f32::NAN, 10.0),
            Size::new(10.0, f32::INFINITY),
        ] {
            assert!(PanelState::new(Some(size)).validate().is_err(), "size: {size:?}");
        }

        assert_eq!(
            PanelState::new(Some(Size::new(0.0, 10.0))).validate(),
            Err(GeometryError::NonPositiveDimension)
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip_preserves_layout_state() {
        let state = PanelState::new(Some(Size::new(280.0, 420.0)));
        let encoded = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<PanelState>(&encoded).unwrap(), state);
    }
}

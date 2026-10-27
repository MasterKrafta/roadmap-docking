use crate::{AllowedArea, GeometryError, PanelKey, Size};

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
    /// Permitted destination classes for drop-target filtering.
    #[cfg_attr(feature = "serde", serde(default))]
    pub allowed_areas: AllowedArea,
}

impl PanelConstraints {
    pub const fn new(
        min_size: Size,
        preferred_size: Size,
        max_size: Option<Size>,
        singleton: bool,
    ) -> Self {
        Self {
            min_size,
            preferred_size,
            max_size,
            singleton,
            allowed_areas: AllowedArea::ANY,
        }
    }

    /// Returns the same constraints with an explicit area policy.
    pub const fn with_allowed_areas(mut self, allowed_areas: AllowedArea) -> Self {
        self.allowed_areas = allowed_areas;
        self
    }

    pub const fn allows_area(self, area: AllowedArea) -> bool {
        self.allowed_areas.contains(area)
    }

    /// Checks that all configured dimensions and area policy are valid.
    pub fn validate(self) -> Result<(), GeometryError> {
        if !self.allowed_areas.is_valid() {
            return Err(GeometryError::InconsistentConstraints);
        }
        self.min_size.validate_positive()?;
        self.preferred_size.validate_positive()?;
        if let Some(max_size) = self.max_size {
            max_size.validate_positive()?;
        }

        if self.min_size.width > self.preferred_size.width
            || self.min_size.height > self.preferred_size.height
        {
            return Err(GeometryError::InconsistentConstraints);
        }

        if let Some(max_size) = self.max_size
            && (self.preferred_size.width > max_size.width
                || self.preferred_size.height > max_size.height
                || self.min_size.width > max_size.width
                || self.min_size.height > max_size.height)
        {
            return Err(GeometryError::InconsistentConstraints);
        }

        Ok(())
    }
}

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

/// Core-owned metadata for one instance of a panel implementation.
///
/// The stable key identifies the panel kind; constraints are copied from the
/// app-owned descriptor when OpenPanel is applied. The state stores layout-only
/// memory, not internal content state.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PanelInstance {
    pub key: PanelKey,
    pub constraints: PanelConstraints,
    pub state: PanelState,
}

impl PanelInstance {
    /// Creates a panel instance with no remembered layout size.
    pub fn new(key: PanelKey, constraints: PanelConstraints) -> Self {
        Self {
            key,
            constraints,
            state: PanelState::default(),
        }
    }

    /// Creates a panel instance with explicitly restored layout-only state.
    pub const fn with_state(
        key: PanelKey,
        constraints: PanelConstraints,
        state: PanelState,
    ) -> Self {
        Self {
            key,
            constraints,
            state,
        }
    }

    /// Validates metadata copied from the app descriptor and remembered geometry.
    pub fn validate(&self) -> Result<(), crate::GeometryError> {
        self.constraints.validate()?;
        self.state.validate()
    }
}
#[cfg(test)]
mod constraints_tests {
    use super::PanelConstraints;
    use crate::{GeometryError, Size};

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
        let constraints =
            PanelConstraints::new(Size::new(300.0, 80.0), Size::new(280.0, 220.0), None, false);
        assert_eq!(
            constraints.validate(),
            Err(GeometryError::InconsistentConstraints)
        );

        let constraints = PanelConstraints::new(
            Size::new(1.0, 1.0),
            Size::new(f32::INFINITY, 10.0),
            None,
            true,
        );
        assert_eq!(constraints.validate(), Err(GeometryError::NonFinite));
    }
}

#[cfg(test)]
mod state_tests {
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
            assert!(
                PanelState::new(Some(size)).validate().is_err(),
                "size: {size:?}"
            );
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

#[cfg(test)]
mod instance_tests {
    use super::{PanelConstraints, PanelInstance, PanelKey, PanelState};
    use crate::Size;

    fn constraints() -> PanelConstraints {
        PanelConstraints::new(
            Size::new(120.0, 80.0),
            Size::new(280.0, 220.0),
            Some(Size::new(640.0, 480.0)),
            false,
        )
    }

    #[test]
    fn new_instance_starts_without_remembered_size() {
        let instance = PanelInstance::new(PanelKey::new("layers").unwrap(), constraints());
        assert_eq!(instance.state.last_known_size(), None);
        assert_eq!(instance.validate(), Ok(()));
    }

    #[test]
    fn restored_instance_validates_its_constraints_and_state() {
        let instance = PanelInstance::with_state(
            PanelKey::new("layers").unwrap(),
            constraints(),
            PanelState::new(Some(Size::new(280.0, 420.0))),
        );
        assert_eq!(instance.validate(), Ok(()));
    }
}

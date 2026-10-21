use crate::{PanelConstraints, PanelKey, PanelState};

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
    pub const fn new(key: PanelKey, constraints: PanelConstraints) -> Self {
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
mod tests {
    use crate::{PanelConstraints, PanelInstance, PanelKey, PanelState, Size};

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

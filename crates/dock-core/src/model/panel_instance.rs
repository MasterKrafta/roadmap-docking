use crate::ids::PanelKey;

use super::{PanelConstraints, PanelState};

/// One opened instance of a registered panel; identity is internal, while its key is stable.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PanelInstance {
    key: PanelKey,
    constraints: PanelConstraints,
    state: PanelState,
}

impl PanelInstance {
    pub fn new(key: PanelKey, constraints: PanelConstraints) -> Self {
        Self {
            key,
            constraints,
            state: PanelState::default(),
        }
    }

    pub fn key(&self) -> &PanelKey {
        &self.key
    }
    pub const fn constraints(&self) -> &PanelConstraints {
        &self.constraints
    }
    pub const fn state(&self) -> &PanelState {
        &self.state
    }

    pub(crate) fn set_state(&mut self, state: PanelState) {
        self.state = state;
    }
}

#[cfg(test)]
mod tests {
    use crate::{PanelConstraints, PanelInstance, PanelKey, PanelState, Size};

    #[test]
    fn instance_keeps_stable_key_constraints_and_layout_state() {
        let key = PanelKey::new("layers").expect("valid key");
        let constraints = PanelConstraints::new(
            Size::new(160.0, 100.0),
            Size::new(280.0, 300.0),
            Size::new(640.0, 900.0),
            true,
        );
        let instance = PanelInstance::new(key, constraints);
        assert_eq!(instance.key().as_str(), "layers");
        assert_eq!(instance.constraints().min_size(), Size::new(160.0, 100.0));
        assert_eq!(
            instance.constraints().preferred_size(),
            Size::new(280.0, 300.0)
        );
        assert_eq!(instance.constraints().max_size(), Size::new(640.0, 900.0));
        assert!(instance.constraints().is_singleton());
        assert_eq!(instance.state(), &PanelState::default());
        assert_eq!(instance.state().last_known_size(), None);
    }
}

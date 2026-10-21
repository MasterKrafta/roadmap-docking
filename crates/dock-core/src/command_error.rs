use crate::{FloatingGroupId, NodeId, PanelId, Profile, ValidationError};

/// A command rejected before committing changes to the layout.
///
/// This is runtime diagnostic data, not persisted workspace state; therefore it
/// intentionally does not implement Serde.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum CommandError {
    #[error("command references missing node {node:?}")]
    MissingNode { node: NodeId },

    #[error("command references missing panel instance {panel:?}")]
    MissingPanel { panel: PanelId },

    #[error("command references missing floating group {group:?}")]
    MissingFloatingGroup { group: FloatingGroupId },

    #[error("invalid command target: {reason}")]
    InvalidTarget { reason: &'static str },

    #[error("invalid command value for {field}: {reason}")]
    InvalidValue { field: &'static str, reason: &'static str },

    #[error("command is not supported by profile {profile:?}: {reason}")]
    UnsupportedForProfile { profile: Profile, reason: &'static str },

    #[error("command would produce an invalid layout: {errors:?}")]
    InvalidLayout { errors: Vec<ValidationError> },
}

#[cfg(test)]
mod tests {
    use super::CommandError;
    use crate::{NodeId, Profile};

    #[test]
    fn errors_are_typed_and_readable() {
        let error = CommandError::MissingNode { node: NodeId::null() };
        assert!(error.to_string().contains("missing node"));

        let error = CommandError::UnsupportedForProfile {
            profile: Profile::Photoshop,
            reason: "horizontal split inside a group",
        };
        assert!(error.to_string().contains("Photoshop"));
    }
}

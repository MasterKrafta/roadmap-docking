use crate::ids::{FloatingGroupId, NodeId, PanelId, PanelKey};
use crate::model::Profile;

use super::ColumnRef;

/// Errors returned when a docking command cannot be applied.
///
/// Validation diagnostics are stored as strings so this public error type can
/// be serialized with the optional serde feature without coupling it to the
/// internal ValidationError representation.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CommandError {
    #[error("panel instance does not exist: {0:?}")]
    UnknownPanel(PanelId),

    #[error("dock group does not exist: {0:?}")]
    UnknownGroup(NodeId),

    #[error("dock column does not exist: {0:?}")]
    UnknownColumn(ColumnRef),

    #[error("floating group does not exist: {0:?}")]
    UnknownFloatingGroup(FloatingGroupId),

    #[error("tab index {index} is invalid for group {group:?}")]
    InvalidTabIndex { group: NodeId, index: usize },

    #[error("invalid command parameter {parameter}: {reason}")]
    InvalidCommandParameter { parameter: String, reason: String },

    #[error("invalid drop target: {0}")]
    InvalidDropTarget(String),

    #[error("protected Canvas cannot be closed, moved, or placed in a dock")]
    ProtectedCanvas,

    #[error("singleton panel is already open: {0:?}")]
    SingletonPanelAlreadyOpen(PanelKey),

    #[error("layout validation failed: {reasons:?}")]
    InvalidLayout { reasons: Vec<String> },

    #[error("layout version counter overflowed")]
    VersionOverflow,

    #[error("command is not implemented yet: {0}")]
    UnsupportedCommand(String),

    #[error("command {command} is unsupported by profile {profile:?}")]
    UnsupportedForProfile { command: String, profile: Profile },
}

#[cfg(test)]
mod tests {
    use super::CommandError;
    use crate::ids::NodeId;

    #[test]
    fn command_errors_have_diagnostic_display_messages() {
        let error = CommandError::UnknownGroup(NodeId::null());
        assert!(error.to_string().contains("dock group does not exist"));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn command_error_implements_serde() {
        fn assert_serde<T: serde::Serialize + serde::de::DeserializeOwned>() {}
        assert_serde::<CommandError>();
    }
}

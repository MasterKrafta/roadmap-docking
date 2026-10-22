use crate::{CommandValue, DockCommand};

/// Internal outcome of executing a command against a transaction candidate.
///
/// This stays separate from the public CommandOutcome because it records
/// implementation details before normalization, validation, and commit.
#[derive(Clone, Debug)]
pub(crate) struct CommandApplication {
    pub(crate) actual_value: Option<CommandValue>,
    pub(crate) inverse: Option<DockCommand>,
    pub(crate) persistent: bool,
}

impl CommandApplication {
    pub(crate) fn persistent() -> Self {
        Self {
            actual_value: None,
            inverse: None,
            persistent: true,
        }
    }

    pub(crate) fn transient() -> Self {
        Self {
            actual_value: None,
            inverse: None,
            persistent: false,
        }
    }
}

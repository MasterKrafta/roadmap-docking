use crate::ids::NodeId;

use super::DockCommand;

/// Summary of a successfully applied docking command.
///
/// changed_nodes contains node IDs whose rendering may need invalidation,
/// including IDs that were removed by the operation. The inverse slot is
/// intentionally None until undo support is implemented in stage 10.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CommandOutcome {
    pub changed_nodes: Vec<NodeId>,
    pub version: u64,
    pub inverse: Option<DockCommand>,
    /// Actual logical-pixel result for resize commands: applied delta for boundary
    /// drags, or resulting width for ResizeColumn. None for other commands.
    #[cfg_attr(feature = "serde", serde(default))]
    pub actual_value: Option<f32>,
}

impl CommandOutcome {
    pub fn new(changed_nodes: Vec<NodeId>, version: u64) -> Self {
        Self {
            changed_nodes,
            version,
            inverse: None,
            actual_value: None,
        }
    }

    pub fn with_actual_value(mut self, actual_value: Option<f32>) -> Self {
        self.actual_value = actual_value;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::CommandOutcome;

    #[test]
    fn new_outcome_starts_without_an_inverse() {
        let outcome = CommandOutcome::new(Vec::new(), 7);

        assert!(outcome.changed_nodes.is_empty());
        assert_eq!(outcome.version, 7);
        assert!(outcome.inverse.is_none());
        assert!(outcome.actual_value.is_none());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn command_outcome_implements_serde() {
        fn assert_serde<T: serde::Serialize + serde::de::DeserializeOwned>() {}
        assert_serde::<CommandOutcome>();
    }
}

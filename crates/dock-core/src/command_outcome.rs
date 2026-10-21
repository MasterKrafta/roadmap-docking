use crate::{CommandValue, DockCommand, NodeId};

/// Result metadata returned after a command has been applied successfully.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CommandOutcome {
    /// Nodes whose persistent representation changed.
    pub changed_nodes: Vec<NodeId>,
    /// Layout version after the successful command.
    pub new_version: u64,
    /// Effective value when a requested resize was constrained.
    pub actual_value: Option<CommandValue>,
    /// Best-effort inverse command for undo; None when no safe inverse is known.
    pub inverse: Option<DockCommand>,
}

#[cfg(test)]
mod tests {
    use slotmap::Key;
    use super::CommandOutcome;
    use crate::{CommandValue, DockCommand, NodeId};

    #[test]
    fn outcome_can_report_a_clamped_value_and_inverse() {
        let outcome = CommandOutcome {
            changed_nodes: vec![NodeId::null()],
            new_version: 4,
            actual_value: Some(CommandValue::ColumnWidth(240.0)),
            inverse: Some(DockCommand::ResizeColumn { column: NodeId::null(), width: 280.0 }),
        };
        assert_eq!(outcome.new_version, 4);
        assert_eq!(outcome.actual_value, Some(CommandValue::ColumnWidth(240.0)));
        assert!(outcome.inverse.is_some());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip_preserves_outcome_contract() {
        let outcome = CommandOutcome {
            changed_nodes: Vec::new(),
            new_version: 8,
            actual_value: Some(CommandValue::GroupRatios(vec![0.4, 0.6])),
            inverse: Some(DockCommand::CloseFlyout),
        };
        let encoded = serde_json::to_string(&outcome).unwrap();
        assert_eq!(serde_json::from_str::<CommandOutcome>(&encoded).unwrap(), outcome);
    }
}

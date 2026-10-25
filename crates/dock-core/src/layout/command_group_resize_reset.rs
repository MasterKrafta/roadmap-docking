use super::*;
use super::helpers::ratio_issues;

impl Layout {
    pub(super) fn apply_reset_column_ratios_command(
        &mut self,
        column: &NodeId,
        available_height: &f32,
        metrics: &DockMetrics,
    ) -> Result<CommandApplication, CommandError> {
        if !available_height.is_finite() || *available_height <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "available_height",
                reason: "available group height must be finite and positive",
            });
        }
        let current = match self.nodes.get(*column) {
            Some(Node::Column(value)) => value.clone(),
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "ResetColumnRatios target must be a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: *column }),
        };
        // Validate the supplied height against fixed collapsed strips
        // and expanded minimums even when the reset itself becomes a no-op.
        let _current_heights =
            self.group_heights_for_available_height(*column, *available_height, metrics)?;
        if current
            .groups
            .iter()
            .all(|group| self.group_is_collapsed(*group))
        {
            return Ok(CommandApplication::persistent());
        }
        let heights = self.equal_group_heights(*column, *available_height, metrics)?;
        let ratios = self.group_ratios_from_heights(*column, &heights)?;
        if ratios == current.group_ratios {
            return Ok(CommandApplication::persistent());
        }
        if let Some(Node::Column(value)) = self.nodes.get_mut(*column) {
            value.group_ratios = ratios.clone();
        }
        Ok(CommandApplication {
            actual_value: Some(CommandValue::GroupRatios(ratios)),
            inverse: Some(DockCommand::RestoreGroupRatios {
                column: *column,
                ratios: current.group_ratios,
            }),
            persistent: true,
        })
    }

    pub(super) fn apply_restore_group_ratios_command(
        &mut self,
        column: &NodeId,
        ratios: &Vec<f32>,
    ) -> Result<CommandApplication, CommandError> {
        let current = match self.nodes.get(*column) {
            Some(Node::Column(value)) => value.group_ratios.clone(),
            Some(_) => {
                return Err(CommandError::InvalidTarget {
                    reason: "RestoreGroupRatios target must be a Column node",
                });
            }
            None => return Err(CommandError::MissingNode { node: *column }),
        };
        let count = match self.nodes.get(*column) {
            Some(Node::Column(value)) => value.groups.len(),
            _ => 0,
        };
        if ratios.len() != count {
            return Err(CommandError::InvalidValue {
                field: "ratios",
                reason: "ratio count must match group count",
            });
        }
        if let Some(reason) = ratio_issues(ratios, count).first() {
            return Err(CommandError::InvalidValue {
                field: "ratios",
                reason,
            });
        }
        if current == *ratios {
            return Ok(CommandApplication::persistent());
        }
        if let Some(Node::Column(value)) = self.nodes.get_mut(*column) {
            value.group_ratios = ratios.clone();
        }
        Ok(CommandApplication {
            actual_value: Some(CommandValue::GroupRatios(ratios.clone())),
            inverse: Some(DockCommand::RestoreGroupRatios {
                column: *column,
                ratios: current,
            }),
            persistent: true,
        })
    }
}

use super::*;

impl Layout {
    pub(super) fn validate_flyout(&self, errors: &mut Vec<ValidationError>) {
        if let Some(flyout) = self.flyout {
            match self.nodes.get(flyout.column) {
                None => errors.push(ValidationError::InvalidFlyout {
                    column: flyout.column,
                    group: flyout.group,
                    reason: "references a missing column node",
                }),
                Some(Node::Column(column)) => {
                    if !column.mode.is_rail() {
                        errors.push(ValidationError::InvalidFlyout {
                            column: flyout.column,
                            group: flyout.group,
                            reason: "flyout column must use a rail mode",
                        });
                    }
                    if !column
                        .groups
                        .iter()
                        .any(|root| self.is_descendant_or_same(*root, flyout.group))
                    {
                        errors.push(ValidationError::InvalidFlyout {
                            column: flyout.column,
                            group: flyout.group,
                            reason: "group is not a member of the referenced column",
                        });
                    }
                }
                Some(_) => errors.push(ValidationError::InvalidFlyout {
                    column: flyout.column,
                    group: flyout.group,
                    reason: "column reference does not point to a Column node",
                }),
            }

            if !self.nodes.contains_key(flyout.group) {
                errors.push(ValidationError::InvalidFlyout {
                    column: flyout.column,
                    group: flyout.group,
                    reason: "references a missing group node",
                });
            }

            let panel_belongs_to_group = match self.nodes.get(flyout.group) {
                Some(Node::Tabs(tabs)) => tabs.children.iter().any(|child| {
                    matches!(
                        self.nodes.get(*child),
                        Some(Node::Panel(panel)) if panel.panel_id == flyout.panel
                    )
                }),
                _ => false,
            };
            if !panel_belongs_to_group {
                errors.push(ValidationError::InvalidFlyout {
                    column: flyout.column,
                    group: flyout.group,
                    reason: "panel is not a child of the flyout group",
                });
            }
        }
    }
}

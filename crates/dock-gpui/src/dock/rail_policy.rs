//! Width-based transitions between expanded dock columns and icon rails.

use dock_core::{ColumnMode, CommandError, DockCommand, DockMetrics, DockSide, Layout, Node};

/// Hysteresis thresholds prevent a narrow column from oscillating between modes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RailPolicy {
    collapse_width: f32,
    expand_width: f32,
}

impl RailPolicy {
    /// Invalid theme thresholds disable the automatic policy instead of panicking.
    pub(super) fn new(collapse_width: f32, expand_width: f32) -> Option<Self> {
        (collapse_width.is_finite()
            && expand_width.is_finite()
            && collapse_width > 0.0
            && expand_width > collapse_width)
            .then_some(Self {
                collapse_width,
                expand_width,
            })
    }

    /// Computes the visible resize preview from the immutable pointer-down snapshot
    /// and the previous preview's mode state. Keeping the latter preserves hysteresis
    /// while the pointer crosses back and forth inside the threshold gap.
    pub(super) fn preview(
        self,
        baseline: &Layout,
        previous_preview: &Layout,
        resize_command: &DockCommand,
        metrics: &DockMetrics,
    ) -> Result<(Layout, Vec<DockCommand>, dock_core::CommandOutcome), CommandError> {
        let mut candidate = baseline.clone();

        if let DockCommand::ResizeColumn { column, width } = resize_command
            && crate::tree_query::main_column_ids(baseline, DockSide::Left)
                .into_iter()
                .chain(crate::tree_query::main_column_ids(
                    baseline,
                    DockSide::Right,
                ))
                .any(|candidate_column| candidate_column == *column)
            && let (Some(Node::Column(start_column)), Some(Node::Column(previous_column))) =
                (baseline.node(*column), previous_preview.node(*column))
        {
            let desired_rail = if previous_column.mode.is_rail() {
                *width < self.expand_width
            } else {
                *width <= self.collapse_width
            };
            let mut commands = Vec::with_capacity(3);

            // ResizeColumn is mode-sensitive. Restore the expanded mode first so
            // resizing a column that began this gesture as a rail uses the requested
            // width; collapse only after that width has been saved as last_expanded_width.
            if start_column.mode.is_rail() {
                commands.push(DockCommand::SetColumnMode {
                    column: *column,
                    mode: ColumnMode::Expanded,
                });
            }
            commands.push(resize_command.clone());
            if desired_rail {
                commands.push(DockCommand::SetColumnMode {
                    column: *column,
                    mode: ColumnMode::IconsOnly,
                });
            }

            let mut outcome = None;
            for command in &commands {
                outcome = Some(candidate.apply(command.clone(), metrics)?);
            }
            return Ok((
                candidate,
                commands,
                outcome.expect("resize command sequence is always non-empty"),
            ));
        }

        let mut commands = vec![resize_command.clone()];
        let mut outcome = candidate.apply(resize_command.clone(), metrics)?;

        // Only consider columns in the main docks; floating compositions use their own
        // resize semantics and should never turn into icon rails. Main horizontal
        // boundary resizes can also narrow a column through the entry threshold.
        if matches!(resize_command, DockCommand::ResizeColumnBoundary { .. }) {
            for side in [DockSide::Left, DockSide::Right] {
                for column in crate::tree_query::main_column_ids(&candidate, side) {
                    let crossed_threshold = matches!(
                        (baseline.node(column), previous_preview.node(column), candidate.node(column)),
                        (
                            Some(Node::Column(_)),
                            Some(Node::Column(previous)),
                            Some(Node::Column(after))
                        ) if (if previous.mode.is_rail() {
                            after.width < self.expand_width
                        } else {
                            after.width <= self.collapse_width
                        }) && !after.mode.is_rail()
                    );
                    if crossed_threshold {
                        let transition = DockCommand::SetColumnMode {
                            column,
                            mode: ColumnMode::IconsOnly,
                        };
                        outcome = candidate.apply(transition.clone(), metrics)?;
                        commands.push(transition);
                        return Ok((candidate, commands, outcome));
                    }
                }
            }
        }

        Ok((candidate, commands, outcome))
    }
}

#[cfg(test)]
mod tests {
    use super::RailPolicy;
    use dock_core::{ColumnMode, DockCommand, DockSide, Node};
    use ui::Theme;

    #[test]
    fn collapse_and_expand_thresholds_have_a_hysteresis_gap() {
        let policy = RailPolicy::new(220.0, 240.0).unwrap();
        assert!(RailPolicy::new(240.0, 220.0).is_none());
        assert!(RailPolicy::new(f32::NAN, 240.0).is_none());

        let theme = Theme::default();
        let controller = crate::dock::DockController::new_initial(&theme).unwrap();
        let metrics = crate::dock_metrics_from_theme(&theme);
        let column = crate::tree_query::main_column_ids(controller.layout(), DockSide::Right)
            .first()
            .copied()
            .filter(|id| matches!(controller.layout().node(*id), Some(Node::Column(value)) if !value.mode.is_rail() && value.width > 220.0))
            .expect("canvas-facing right column should exceed the collapse threshold");

        let collapsed = DockCommand::ResizeColumn {
            column,
            width: 219.0,
        };
        let (preview, commands, _) = policy
            .preview(
                controller.layout(),
                controller.layout(),
                &collapsed,
                &metrics,
            )
            .unwrap();
        assert_eq!(
            commands,
            vec![
                collapsed,
                DockCommand::SetColumnMode {
                    column,
                    mode: ColumnMode::IconsOnly
                }
            ]
        );
        assert!(
            matches!(preview.node(column), Some(Node::Column(value)) if value.mode == ColumnMode::IconsOnly)
        );
        assert!(
            matches!(controller.layout().node(column), Some(Node::Column(value)) if value.mode == ColumnMode::Expanded)
        );

        let mut rail = controller.layout().clone();
        rail.apply(
            DockCommand::SetColumnMode {
                column,
                mode: ColumnMode::IconsOnly,
            },
            &metrics,
        )
        .unwrap();
        let expand = DockCommand::ResizeColumn {
            column,
            width: 241.0,
        };
        let (preview, commands, _) = policy.preview(&rail, &rail, &expand, &metrics).unwrap();
        assert_eq!(
            commands,
            vec![
                DockCommand::SetColumnMode {
                    column,
                    mode: ColumnMode::Expanded
                },
                expand,
            ]
        );
        assert!(
            matches!(preview.node(column), Some(Node::Column(value)) if value.mode == ColumnMode::Expanded && value.width >= 240.0)
        );

        // Once expanded during a gesture, the column stays expanded until the lower
        // collapse threshold is crossed. The gap between 220 and 240 prevents flicker.
        let intermediate = DockCommand::ResizeColumn {
            column,
            width: 230.0,
        };
        let (preview, commands, _) = policy
            .preview(&rail, &preview, &intermediate, &metrics)
            .unwrap();
        assert_eq!(
            commands,
            vec![
                DockCommand::SetColumnMode {
                    column,
                    mode: ColumnMode::Expanded
                },
                intermediate
            ]
        );
        assert!(
            matches!(preview.node(column), Some(Node::Column(value)) if value.mode == ColumnMode::Expanded && value.width >= 230.0)
        );

        let boundary = DockCommand::ResizeColumnBoundary {
            side: DockSide::Right,
            boundary_index: 0,
            delta: -70.0,
        };
        let (preview, commands, _) = policy
            .preview(
                controller.layout(),
                controller.layout(),
                &boundary,
                &metrics,
            )
            .unwrap();
        assert_eq!(
            commands,
            vec![
                boundary,
                DockCommand::SetColumnMode {
                    column,
                    mode: ColumnMode::IconsOnly,
                },
            ]
        );
        assert!(matches!(
            preview.node(column),
            Some(Node::Column(value)) if value.mode == ColumnMode::IconsOnly
        ));
    }

    #[test]
    fn hysteresis_keeps_the_mode_reached_when_crossing_back_through_gap() {
        let policy = RailPolicy::new(220.0, 240.0).unwrap();
        let theme = Theme::default();
        let controller = crate::dock::DockController::new_initial(&theme).unwrap();
        let baseline = controller.layout();
        let metrics = crate::dock_metrics_from_theme(&theme);
        let column = crate::tree_query::main_column_ids(baseline, DockSide::Right)
            .first()
            .copied()
            .filter(|id| matches!(baseline.node(*id), Some(Node::Column(value)) if !value.mode.is_rail() && value.width > 220.0))
            .expect("canvas-facing right column should exceed the collapse threshold");
        let resize_to = |width| DockCommand::ResizeColumn { column, width };

        // Enter rail below the collapse threshold.
        let (mut preview, _, _) = policy
            .preview(baseline, baseline, &resize_to(219.0), &metrics)
            .unwrap();
        assert!(matches!(
            preview.node(column),
            Some(Node::Column(value)) if value.mode.is_rail()
        ));

        // Moving back into the gap does not expand until the upper threshold is crossed.
        let (next, _, _) = policy
            .preview(baseline, &preview, &resize_to(230.0), &metrics)
            .unwrap();
        preview = next;
        assert!(matches!(
            preview.node(column),
            Some(Node::Column(value)) if value.mode.is_rail()
        ));

        // Crossing the expand threshold switches to expanded mode.
        let (next, _, _) = policy
            .preview(baseline, &preview, &resize_to(241.0), &metrics)
            .unwrap();
        preview = next;
        assert!(matches!(
            preview.node(column),
            Some(Node::Column(value))
                if value.mode == ColumnMode::Expanded && (value.width - 241.0).abs() < 0.01
        ));

        // Returning into the gap keeps expanded mode until the collapse threshold is crossed.
        let (next, _, _) = policy
            .preview(baseline, &preview, &resize_to(230.0), &metrics)
            .unwrap();
        preview = next;
        assert!(matches!(
            preview.node(column),
            Some(Node::Column(value))
                if value.mode == ColumnMode::Expanded && (value.width - 230.0).abs() < 0.01
        ));

        let (preview, _, _) = policy
            .preview(baseline, &preview, &resize_to(220.0), &metrics)
            .unwrap();
        assert!(matches!(
            preview.node(column),
            Some(Node::Column(value)) if value.mode.is_rail()
        ));
    }
}

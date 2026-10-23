use dock_core::{
    Axis, DockCommand, DockMetrics, DockSide, DropTarget, DropZone, Layout, Node, NodeId, PanelKey,
    Profile,
};

use crate::panels::PanelRegistry;

use super::group_ids_for_column;

/// Build the startup docking tree using Core commands and return its stable column IDs.
pub(super) fn create_initial_layout(
    panels: &PanelRegistry,
    dock_metrics: &DockMetrics,
    split_ratio: f32,
) -> Result<(Layout, Vec<NodeId>), String> {
            let layers_key = PanelKey::new("layers").expect("built-in panel key must be valid");
            let color_key = PanelKey::new("color").expect("built-in panel key must be valid");
            let properties_key =
                PanelKey::new("properties").expect("built-in panel key must be valid");

            let constraints_for = |key: &PanelKey, title: &str| {
                panels
                    .constraints_for_key(key)
                    .ok_or_else(|| format!("{title} panel is missing from the panel registry"))
            };
            let layers_constraints = constraints_for(&layers_key, "Layers")?;
            let color_constraints = constraints_for(&color_key, "Color")?;
            let properties_constraints = constraints_for(&properties_key, "Properties")?;

            let mut layout = Layout::new(Profile::Photoshop);

            // Build the original right column and its two tab groups using Core commands.
            layout
                .apply(
                    DockCommand::OpenPanel {
                        key: layers_key.clone(),
                        constraints: layers_constraints,
                        target: None,
                    },
                    dock_metrics,
                )
                .map_err(|error| format!("Could not open the Layers panel: {error}"))?;

            let initial_right_column = layout
                .main_area(DockSide::Right)
                .ok_or_else(|| "Dock Core did not create the initial right dock area".to_owned())?
                .root;
            let initial_top_group = group_ids_for_column(&layout, initial_right_column)
                .first()
                .copied()
                .ok_or_else(|| "Dock Core did not create the upper tab group".to_owned())?;

            layout
                .apply(
                    DockCommand::OpenPanel {
                        key: color_key.clone(),
                        constraints: color_constraints,
                        target: Some(DropTarget::Tabs {
                            group: initial_top_group,
                            index: 1,
                        }),
                    },
                    dock_metrics,
                )
                .map_err(|error| format!("Could not add the Color tab: {error}"))?;

            layout
                .apply(
                    DockCommand::OpenPanel {
                        key: properties_key.clone(),
                        constraints: properties_constraints,
                        target: Some(DropTarget::GroupEdge {
                            group: initial_top_group,
                            zone: DropZone::Bottom,
                        }),
                    },
                    dock_metrics,
                )
                .map_err(|error| format!("Could not create the lower tab group: {error}"))?;

            layout
                .apply(
                    DockCommand::RestoreGroupRatios {
                        column: initial_right_column,
                        ratios: vec![split_ratio, 1.0 - split_ratio],
                    },
                    dock_metrics,
                )
                .map_err(|error| format!("Could not apply the right column split ratio: {error}"))?;

            // Insert a second copy to the left of the existing column. Duplicate panel
            // keys are allowed by the registry constraints; each column gets independent
            // panel instances and independent active-tab state.
            layout
                .apply(
                    DockCommand::OpenPanel {
                        key: layers_key.clone(),
                        constraints: layers_constraints,
                        target: Some(DropTarget::MainColumn {
                            side: DockSide::Right,
                            index: 0,
                        }),
                    },
                    dock_metrics,
                )
                .map_err(|error| format!("Could not create the left dock column: {error}"))?;

            let right_area = layout
                .main_area(DockSide::Right)
                .ok_or_else(|| "Dock Core lost the right dock area".to_owned())?;
            let Node::Split(split) = layout
                .node(right_area.root)
                .ok_or_else(|| "Dock Core did not return the two-column split".to_owned())?
            else {
                return Err("Dock Core did not create a split for the two dock columns".to_owned());
            };
            if split.axis != Axis::X {
                return Err("Dock Core right dock must be a horizontal split of columns".to_owned());
            }
            let column_ids = split.children.clone();
            if column_ids.last() != Some(&initial_right_column) {
                return Err("Dock Core changed the original column's order unexpectedly".to_owned());
            }
            // Initialize each Core group by activating its first actual panel. This
            // does not require a fixed Layers/Color/Properties slot map.
            for column_id in column_ids.iter().copied() {
                let group_ids = match layout.node(column_id) {
                    Some(Node::Column(column)) => column.groups.clone(),
                    _ => Vec::new(),
                };
                for group_id in group_ids {
                    let first_panel = match layout.node(group_id) {
                        Some(Node::Tabs(tabs)) => tabs.children.iter().find_map(|child_id| {
                            match layout.node(*child_id) {
                                Some(Node::Panel(panel)) => Some(panel.panel_id),
                                _ => None,
                            }
                        }),
                        _ => None,
                    };
                    if let Some(panel) = first_panel {
                        layout
                            .apply(
                                DockCommand::ActivateTab {
                                    group: group_id,
                                    panel,
                                },
                                dock_metrics,
                            )
                            .map_err(|error| {
                                format!("Could not activate the first panel in group {group_id:?}: {error}")
                            })?;
                    }
                }
            }

    Ok((layout, column_ids))
}

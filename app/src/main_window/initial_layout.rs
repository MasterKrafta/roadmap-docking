use dock_core::{
    Axis, DockCommand, DockMetrics, DockSide, DropTarget, DropZone, Layout, Node, NodeId, PanelKey,
    Profile,
};

use crate::panels::PanelRegistry;

use super::group_ids_for_column;

/// Initial layout is data, not renderer logic. Add or reorder groups here without
/// changing the MainWindow algorithms. Columns are declared in visual left-to-right order;
/// construction proceeds right-to-left because Core inserts new main columns at index zero.
const INITIAL_COLUMNS_LEFT_TO_RIGHT: &[&[&[&str]]] = &[
    &[&["layers"]],
    &[&["layers", "color"], &["properties"]],
];

/// Build the startup docking tree using Core commands and return its stable column IDs.
pub(super) fn create_initial_layout(
    panels: &PanelRegistry,
    dock_metrics: &DockMetrics,
    split_ratio: f32,
) -> Result<(Layout, Vec<NodeId>), String> {
    let mut layout = Layout::new(Profile::Photoshop);

    for (constructed_column_index, groups) in INITIAL_COLUMNS_LEFT_TO_RIGHT.iter().rev().enumerate() {
        let first_group = groups
            .first()
            .ok_or_else(|| "Initial layout columns must contain at least one tab group".to_owned())?;
        let first_panel_key = first_group
            .first()
            .ok_or_else(|| "Initial layout tab groups must contain at least one panel".to_owned())?;

        let first_column_target = if constructed_column_index == 0 {
            None
        } else {
            Some(DropTarget::MainColumn {
                side: DockSide::Right,
                index: 0,
            })
        };
        open_initial_panel(
            &mut layout,
            panels,
            dock_metrics,
            first_panel_key,
            first_column_target,
        )?;

        let area = layout
            .main_area(DockSide::Right)
            .ok_or_else(|| "Dock Core did not create the initial right dock area".to_owned())?;
        let column_id = match layout.node(area.root) {
            Some(Node::Column(_)) => area.root,
            Some(Node::Split(split)) if split.axis == Axis::X => split
                .children
                .first()
                .copied()
                .ok_or_else(|| "Dock Core created an empty main column split".to_owned())?,
            _ => return Err("Dock Core created an unexpected main dock shape".to_owned()),
        };
        if !matches!(layout.node(column_id), Some(Node::Column(_))) {
            return Err("Dock Core did not create the requested initial column".to_owned());
        }

        let first_group_id = group_ids_for_column(&layout, column_id)
            .first()
            .copied()
            .ok_or_else(|| "Dock Core did not create the first tab group".to_owned())?;
        let mut group_ids = Vec::with_capacity(groups.len());

        for (group_index, panel_keys) in groups.iter().enumerate() {
            let first_panel_key = panel_keys
                .first()
                .ok_or_else(|| "Initial layout tab groups must contain at least one panel".to_owned())?;

            let group_id = if group_index == 0 {
                first_group_id
            } else {
                let previous_group = *group_ids
                    .last()
                    .ok_or_else(|| "No preceding group exists while building initial layout".to_owned())?;
                let groups_before = group_ids_for_column(&layout, column_id);
                open_initial_panel(
                    &mut layout,
                    panels,
                    dock_metrics,
                    first_panel_key,
                    Some(DropTarget::GroupEdge {
                        group: previous_group,
                        zone: DropZone::Bottom,
                    }),
                )?;
                group_ids_for_column(&layout, column_id)
                    .into_iter()
                    .find(|candidate| !groups_before.contains(candidate))
                    .ok_or_else(|| "Dock Core did not create the requested tab group".to_owned())?
            };

            group_ids.push(group_id);
            for panel_key in panel_keys.iter().skip(1) {
                let next_index = match layout.node(group_id) {
                    Some(Node::Tabs(tabs)) => tabs.children.len(),
                    _ => return Err("Dock Core returned a non-Tabs node for an initial group".to_owned()),
                };
                open_initial_panel(
                    &mut layout,
                    panels,
                    dock_metrics,
                    panel_key,
                    Some(DropTarget::Tabs {
                        group: group_id,
                        index: next_index,
                    }),
                )?;
            }
        }

        let group_ratios = initial_group_ratios(groups.len(), split_ratio)?;
        layout
            .apply(
                DockCommand::RestoreGroupRatios {
                    column: column_id,
                    ratios: group_ratios,
                },
                dock_metrics,
            )
            .map_err(|error| format!("Could not apply initial group sizes: {error}"))?;
    }

    // Activate the first actual panel of every group. The same traversal works
    // for any number of columns, groups, and tabs in the initial layout data.
    let area = layout
        .main_area(DockSide::Right)
        .ok_or_else(|| "Dock Core lost the initial main dock area".to_owned())?;
    let column_ids = column_ids_in_core_order(&layout, area.root)?;
    for column_id in column_ids.iter().copied() {
        for group_id in group_ids_for_column(&layout, column_id) {
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

fn open_initial_panel(
    layout: &mut Layout,
    panels: &PanelRegistry,
    dock_metrics: &DockMetrics,
    panel_key: &str,
    target: Option<DropTarget>,
) -> Result<(), String> {
    let key = PanelKey::new(panel_key)
        .map_err(|_| format!("Initial layout contains an invalid panel key: '{panel_key}'"))?;
    let constraints = panels
        .constraints_for_key(&key)
        .ok_or_else(|| format!("Initial layout references unknown panel '{panel_key}'"))?;

    layout
        .apply(
            DockCommand::OpenPanel {
                key,
                constraints,
                target,
            },
            dock_metrics,
        )
        .map_err(|error| format!("Could not add panel '{panel_key}' to the initial layout: {error}"))?;
    Ok(())
}

fn initial_group_ratios(group_count: usize, split_ratio: f32) -> Result<Vec<f32>, String> {
    if group_count == 0 {
        return Err("Initial columns must contain at least one tab group".to_owned());
    }
    if !split_ratio.is_finite() || split_ratio <= 0.0 || split_ratio >= 1.0 {
        return Err("Initial group split ratio must be between zero and one".to_owned());
    }
    if group_count == 1 {
        return Ok(vec![1.0]);
    }

    let remaining_ratio = (1.0 - split_ratio) / (group_count - 1) as f32;
    let mut ratios = vec![remaining_ratio; group_count];
    ratios[0] = split_ratio;
    Ok(ratios)
}

fn column_ids_in_core_order(layout: &Layout, root: NodeId) -> Result<Vec<NodeId>, String> {
    match layout.node(root) {
        Some(Node::Column(_)) => Ok(vec![root]),
        Some(Node::Split(split)) if split.axis == Axis::X => {
            if split
                .children
                .iter()
                .any(|child| !matches!(layout.node(*child), Some(Node::Column(_))))
            {
                return Err("Core main horizontal split must contain only Columns".to_owned());
            }
            Ok(split.children.clone())
        }
        _ => Err("Core main dock root must be a Column or a horizontal split of Columns".to_owned()),
    }
}


#[cfg(test)]
mod tests {
    use super::{
        INITIAL_COLUMNS_LEFT_TO_RIGHT, create_initial_layout, group_ids_for_column,
        initial_group_ratios,
    };
    use crate::panels::PanelRegistry;
    use dock_core::{DockMetrics, Node};

    #[test]
    fn created_layout_matches_the_column_and_group_specification() {
        let metrics = DockMetrics::default();
        let (layout, columns) =
            create_initial_layout(&PanelRegistry::default(), &metrics, 0.6)
                .expect("the built-in panel registry should satisfy the initial layout spec");

        assert_eq!(columns.len(), INITIAL_COLUMNS_LEFT_TO_RIGHT.len());
        assert_eq!(layout.validate(&metrics), Ok(()));

        for (column_id, expected_groups) in
            columns.iter().zip(INITIAL_COLUMNS_LEFT_TO_RIGHT.iter())
        {
            let actual_groups = group_ids_for_column(&layout, *column_id);
            assert_eq!(actual_groups.len(), expected_groups.len());

            for (group_id, expected_panels) in
                actual_groups.iter().zip(expected_groups.iter())
            {
                let Some(Node::Tabs(tabs)) = layout.node(*group_id) else {
                    panic!("every initial group must be represented by a Tabs node");
                };
                assert_eq!(tabs.children.len(), expected_panels.len());

                for (panel_node_id, expected_key) in
                    tabs.children.iter().zip(expected_panels.iter())
                {
                    let Some(Node::Panel(panel)) = layout.node(*panel_node_id) else {
                        panic!("initial Tabs groups must contain Panel leaves");
                    };
                    let instance = layout
                        .panel_instance(panel.panel_id)
                        .expect("every initial Panel node must have an instance");
                    assert_eq!(instance.key.as_str(), *expected_key);
                }
            }
        }
    }

    #[test]
    fn initial_group_ratios_support_any_positive_group_count() {
        assert_eq!(initial_group_ratios(1, 0.6).unwrap(), vec![1.0]);

        for group_count in 2..=8 {
            let ratios = initial_group_ratios(group_count, 0.6).unwrap();
            assert_eq!(ratios.len(), group_count);
            assert!((ratios[0] - 0.6).abs() < 0.000_001);
            assert!(ratios.iter().skip(1).all(|ratio| (*ratio - ratios[1]).abs() < 0.000_001));
            assert!((ratios.iter().sum::<f32>() - 1.0).abs() < 0.000_1);
        }

        assert!(initial_group_ratios(0, 0.6).is_err());
        assert!(initial_group_ratios(2, f32::NAN).is_err());
    }
}

use dock_core::{DockCommand, FloatingGroupId, Layout, Node, NodeId, PanelId};

/// Builds the command used when a tab is selected by a single click.
pub(crate) fn activate_tab(group: NodeId, panel: PanelId) -> DockCommand {
    DockCommand::ActivateTab { group, panel }
}

/// Builds the command used to toggle a group's collapsed state.
pub(crate) fn collapse_group(group: NodeId, collapsed: bool) -> DockCommand {
    DockCommand::CollapseGroup { group, collapsed }
}

/// Builds the command used to close one panel instance.
pub(crate) fn close_panel(panel: PanelId) -> DockCommand {
    DockCommand::ClosePanel { panel }
}

/// Builds the command used to close all panels in a group.
pub(crate) fn close_group(group: NodeId) -> DockCommand {
    DockCommand::CloseGroup { group }
}

/// Builds one close command for every tab group belonging to a floating window.
///
/// The core has no single-command CloseFloating operation, so each returned
/// command is applied through the controller in sequence and gets its own
/// history entry. The groups are collected in tree order so that removing an
/// earlier group can safely normalize the surrounding composition.
pub(crate) fn close_floating(
    layout: &Layout,
    floating: FloatingGroupId,
) -> Option<Vec<DockCommand>> {
    let floating_group = layout.floating_group(floating)?;
    let root = layout.dock_area(floating_group.area)?.root;
    let mut groups = Vec::new();

    if !collect_tab_groups(layout, root, &mut groups) || groups.is_empty() {
        return None;
    }

    Some(groups.into_iter().map(close_group).collect())
}

fn collect_tab_groups(layout: &Layout, node_id: NodeId, groups: &mut Vec<NodeId>) -> bool {
    match layout.node(node_id) {
        Some(Node::Tabs(_)) => {
            groups.push(node_id);
            true
        }
        Some(Node::Split(split)) => split
            .children
            .iter()
            .all(|child| collect_tab_groups(layout, *child, groups)),
        Some(Node::Column(column)) => column
            .groups
            .iter()
            .all(|group| collect_tab_groups(layout, *group, groups)),
        Some(Node::Panel(_)) | None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{activate_tab, close_floating, close_group, close_panel, collapse_group};
    use dock_core::{
        create_initial_layout, DockCommand, DockMetrics, DockSide, FloatingGroupId, Node,
        PanelConstraints, PanelKey, Point, Profile, Rect, Size,
    };
    use slotmap::Key;

    fn descriptors() -> Vec<(PanelKey, PanelConstraints)> {
        let constraints =
            PanelConstraints::new(Size::new(120.0, 80.0), Size::new(280.0, 220.0), None, false);
        ["layers", "color", "properties", "history"]
            .into_iter()
            .map(|key| (PanelKey::new(key).unwrap(), constraints))
            .collect()
    }

    #[test]
    fn activate_tab_builds_the_expected_command() {
        let group = dock_core::NodeId::null();
        let panel = dock_core::PanelId::null();

        assert_eq!(
            activate_tab(group, panel),
            DockCommand::ActivateTab { group, panel }
        );
    }

    #[test]
    fn collapse_group_builds_the_expected_command() {
        let group = dock_core::NodeId::null();

        assert_eq!(
            collapse_group(group, true),
            DockCommand::CollapseGroup {
                group,
                collapsed: true,
            }
        );
    }

    #[test]
    fn close_panel_builds_the_expected_command() {
        let panel = dock_core::PanelId::null();

        assert_eq!(close_panel(panel), DockCommand::ClosePanel { panel });
    }

    #[test]
    fn close_group_builds_the_expected_command() {
        let group = dock_core::NodeId::null();

        assert_eq!(close_group(group), DockCommand::CloseGroup { group });
    }

    #[test]
    fn close_floating_builds_a_close_command_for_each_tab_group_in_tree_order() {
        let metrics = DockMetrics::default();
        let mut layout =
            create_initial_layout(Profile::Photoshop, &descriptors(), metrics).unwrap();
        let root = layout.main_area(DockSide::Right).unwrap().root;
        let groups = match layout.node(root).unwrap() {
            Node::Column(column) => column.groups.clone(),
            other => panic!("expected a column root, got {other:?}"),
        };

        assert!(close_floating(&layout, FloatingGroupId::null()).is_none());

        layout
            .apply(
                DockCommand::Float {
                    node: root,
                    bounds: Rect::new(Point::new(20.0, 30.0), Size::new(640.0, 480.0)),
                },
                &metrics,
            )
            .unwrap();
        let (floating, _) = layout.floating_groups().next().unwrap();

        assert_eq!(
            close_floating(&layout, floating),
            Some(vec![
                DockCommand::CloseGroup { group: groups[0] },
                DockCommand::CloseGroup { group: groups[1] },
            ])
        );
    }
}

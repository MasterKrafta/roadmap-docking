use dock_core::{
    DockCommand, FloatingGroupId, Layout, Node, NodeId, PanelId, PanelKey,
};
use ui::Theme;

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

/// Returns the command that toggles a registered panel kind in the Dockers menu.
///
/// Existing instances are closed by runtime ID. Missing instances are opened
/// with constraints from the registry and no explicit target, allowing Core to
/// restore the last known dock location or choose the profile default.
pub(crate) fn toggle_panel(layout: &Layout, key: &str, theme: &Theme) -> Option<DockCommand> {
    if let Some((panel, _)) = layout
        .panel_instances()
        .find(|(_, instance)| instance.key.as_str() == key)
    {
        return Some(close_panel(panel));
    }

    let definition = crate::panels::PANELS
        .iter()
        .find(|definition| definition.key == key)?;
    let key = PanelKey::new(definition.key).ok()?;
    let constraints = (definition.constraints)(theme);

    Some(DockCommand::OpenPanel {
        key,
        constraints,
        target: None,
    })
}

/// Builds one close command for every tab group belonging to a floating window.
///
/// The core has no single-command CloseFloating operation, so each returned
/// command is applied through the controller in sequence and gets its own
/// history entry. Groups are collected in tree order so surviving IDs can be
/// closed even as each command normalizes the surrounding composition.
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
    use super::{
        activate_tab, close_floating, close_group, close_panel, collapse_group, toggle_panel,
    };
    use crate::dock::DockController;
    use dock_core::{
        DockCommand, DockSide, FloatingGroupId, Layout, Node, NodeId, PanelKey, Point, Rect, Size,
    };
    use slotmap::Key;
    use ui::Theme;

    fn collect_tab_groups(layout: &Layout, node_id: NodeId, groups: &mut Vec<NodeId>) {
        match layout.node(node_id) {
            Some(Node::Tabs(_)) => groups.push(node_id),
            Some(Node::Split(split)) => {
                for child in &split.children {
                    collect_tab_groups(layout, *child, groups);
                }
            }
            Some(Node::Column(column)) => {
                for group in &column.groups {
                    collect_tab_groups(layout, *group, groups);
                }
            }
            Some(Node::Panel(_)) | None => {}
        }
    }

    #[test]
    fn activate_tab_builds_the_expected_command() {
        let group = NodeId::null();
        let panel = dock_core::PanelId::null();

        assert_eq!(
            activate_tab(group, panel),
            DockCommand::ActivateTab { group, panel }
        );
    }

    #[test]
    fn collapse_group_builds_the_expected_command() {
        let group = NodeId::null();

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
        let group = NodeId::null();

        assert_eq!(close_group(group), DockCommand::CloseGroup { group });
    }

    #[test]
    fn toggle_panel_closes_an_existing_instance() {
        let theme = Theme::default();
        let controller = DockController::new_initial(&theme).unwrap();
        let (panel, _) = controller
            .layout()
            .panel_instances()
            .find(|(_, instance)| instance.key.as_str() == "layers")
            .unwrap();

        assert_eq!(
            toggle_panel(controller.layout(), "layers", &theme),
            Some(DockCommand::ClosePanel { panel })
        );
    }

    #[test]
    fn toggle_panel_opens_a_missing_instance_at_the_core_default_target() {
        let theme = Theme::default();
        let mut controller = DockController::new_initial(&theme).unwrap();
        let (panel, _) = controller
            .layout()
            .panel_instances()
            .find(|(_, instance)| instance.key.as_str() == "layers")
            .unwrap();

        controller
            .apply(DockCommand::ClosePanel { panel })
            .unwrap();

        let expected_constraints = (crate::panels::PANELS
            .iter()
            .find(|definition| definition.key == "layers")
            .unwrap()
            .constraints)(&theme);

        assert_eq!(
            toggle_panel(controller.layout(), "layers", &theme),
            Some(DockCommand::OpenPanel {
                key: PanelKey::new("layers").unwrap(),
                constraints: expected_constraints,
                target: None,
            })
        );
    }

    #[test]
    fn toggle_panel_ignores_unknown_registry_keys() {
        let theme = Theme::default();
        let controller = DockController::new_initial(&theme).unwrap();

        assert_eq!(
            toggle_panel(controller.layout(), "missing-panel", &theme),
            None
        );
    }

    #[test]
    fn close_floating_builds_a_close_command_for_each_tab_group_in_tree_order() {
        let theme = Theme::default();
        let mut controller = DockController::new_initial(&theme).unwrap();
        let root = controller.layout().main_area(DockSide::Right).unwrap().root;
        let mut expected_groups = Vec::new();
        collect_tab_groups(controller.layout(), root, &mut expected_groups);

        assert!(close_floating(controller.layout(), FloatingGroupId::null()).is_none());

        controller
            .apply(DockCommand::Float {
                node: root,
                bounds: Rect::new(Point::new(20.0, 30.0), Size::new(640.0, 480.0)),
            })
            .unwrap();
        let (floating, _) = controller.layout().floating_groups().next().unwrap();
        let commands = close_floating(controller.layout(), floating).unwrap();

        assert_eq!(
            commands,
            expected_groups
                .into_iter()
                .map(|group| DockCommand::CloseGroup { group })
                .collect::<Vec<_>>()
        );
    }
}

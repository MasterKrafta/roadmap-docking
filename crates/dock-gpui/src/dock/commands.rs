use dock_core::{
    ColumnMode, DockCommand, FloatingGroupId, Layout, NodeId, PanelId, PanelKey,
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

/// Builds the next command in the column presentation cycle.
pub(crate) fn cycle_column_mode(column: NodeId, current: ColumnMode) -> DockCommand {
    let mode = match current {
        ColumnMode::Expanded => ColumnMode::IconsAndLabels,
        ColumnMode::IconsAndLabels => ColumnMode::IconsOnly,
        ColumnMode::IconsOnly => ColumnMode::Expanded,
    };
    DockCommand::SetColumnMode { column, mode }
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

/// Builds the single atomic command used to close a floating window.
pub(crate) fn close_floating(
    layout: &Layout,
    floating: FloatingGroupId,
) -> Option<DockCommand> {
    let floating_group = layout.floating_group(floating)?;
    layout.dock_area(floating_group.area)?;
    Some(DockCommand::CloseFloatingGroup { group: floating })
}

#[cfg(test)]
mod tests {
    use super::{
        activate_tab, close_floating, close_group, close_panel, collapse_group, cycle_column_mode,
        toggle_panel,
    };
    use crate::dock::DockController;
    use dock_core::{
        ColumnMode, DockCommand, DockSide, FloatingGroupId, Layout, Node, NodeId, PanelKey,
        Point, Rect, Size,
    };
    use slotmap::Key;
    use ui::Theme;


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
    fn cycle_column_mode_uses_the_three_mode_order() {
        let column = NodeId::null();
        for (current, expected) in [
            (ColumnMode::Expanded, ColumnMode::IconsAndLabels),
            (ColumnMode::IconsAndLabels, ColumnMode::IconsOnly),
            (ColumnMode::IconsOnly, ColumnMode::Expanded),
        ] {
            assert_eq!(
                cycle_column_mode(column, current),
                DockCommand::SetColumnMode {
                    column,
                    mode: expected
                }
            );
        }
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

        controller.apply(DockCommand::ClosePanel { panel }).unwrap();

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
    fn close_floating_uses_one_atomic_core_command() {
        let theme = Theme::default();
        let mut controller = DockController::new_initial(&theme).unwrap();
        assert!(close_floating(controller.layout(), FloatingGroupId::null()).is_none());

        let root = controller.layout().main_area(DockSide::Right).unwrap().root;
        let group = match controller.layout().node(root) {
            Some(Node::Column(column)) => column.groups[0],
            _ => panic!("initial Photoshop dock should start with a Column"),
        };
        controller
            .apply(DockCommand::Float {
                node: group,
                bounds: Rect::new(Point::new(20.0, 30.0), Size::new(640.0, 480.0)),
            })
            .unwrap();
        let (floating, _) = controller.layout().floating_groups().next().unwrap();

        assert_eq!(
            close_floating(controller.layout(), floating),
            Some(DockCommand::CloseFloatingGroup { group: floating })
        );
        controller.close_floating(floating).unwrap();
        assert!(controller.layout().floating_group(floating).is_none());
    }
}

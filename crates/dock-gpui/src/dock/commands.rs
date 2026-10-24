use dock_core::{DockCommand, NodeId, PanelId};

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

#[cfg(test)]
mod tests {
    use super::{activate_tab, close_group, close_panel, collapse_group};
    use dock_core::{DockCommand, NodeId, PanelId};
    use slotmap::Key;

    #[test]
    fn activate_tab_builds_the_expected_command() {
        let group = NodeId::null();
        let panel = PanelId::null();

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
        let panel = PanelId::null();

        assert_eq!(close_panel(panel), DockCommand::ClosePanel { panel });
    }

    #[test]
    fn close_group_builds_the_expected_command() {
        let group = NodeId::null();

        assert_eq!(close_group(group), DockCommand::CloseGroup { group });
    }
}

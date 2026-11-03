//! Application-facing GPUI action dispatch helpers.

use dock_core::{DockCommand, DockSide, DropTarget, PanelKey};
use gpui::{App, Entity};

use crate::DockController;

/// Applies a semantic Core command and notifies the owning controller entity.
pub(crate) fn dispatch_command(
    controller: &Entity<DockController>,
    command: DockCommand,
    app: &mut App,
) {
    if let Err(error) = controller.update(app, |controller, controller_cx| {
        controller.dispatch_in(command, controller_cx)
    }) {
        tracing::warn!(%error, "dock command dispatch failed");
    }
}

/// Toggles one registered panel in its assigned Main Dock side.
///
/// This path deliberately bypasses FloatingWindowManager: opening a panel
/// creates a Core Column inside the main window, never a native panel window.
pub(crate) fn toggle_registered_panel(
    controller: &Entity<DockController>,
    key: PanelKey,
    side: DockSide,
    app: &mut App,
) {
    if let Err(error) = controller.update(app, |controller, controller_cx| {
        let existing_panel = controller
            .layout()
            .panel_instances()
            .find_map(|(panel, instance)| (instance.key == key).then_some(panel));

        let command = if let Some(panel) = existing_panel {
            DockCommand::ClosePanel { panel }
        } else {
            let Some(descriptor) = controller.registry().get(&key) else {
                tracing::warn!(panel = %key, "panel menu action references an unknown panel");
                return;
            };

            DockCommand::OpenPanel {
                key: key.clone(),
                constraints: descriptor.constraints(),
                target: Some(DropTarget::MainColumn { side, index: 0 }),
            }
        };

        if let Err(error) = controller.dispatch_in(command, controller_cx) {
            tracing::warn!(%error, panel = %key, "main-dock panel toggle failed");
        }
    }) {
        tracing::warn!(%error, panel = %key, "panel menu action could not reach dock controller");
    }
}

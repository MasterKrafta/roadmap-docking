//! Application-facing GPUI action dispatch helpers.
//!
//! Keyboard action registration and default key bindings are added here when
//! the host exposes the corresponding keyboard interaction contract.

use dock_core::{DockCommand, PanelKey};
use gpui::{App, Entity};

use crate::{DockController, FloatingWindowManager};

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

/// Toggles one registry-backed panel through the manager that owns floating
/// window placement. Panel existence and the resulting checkmark still come
/// from the shared Core Layout.
pub(crate) fn toggle_registered_panel(
    manager: &Entity<FloatingWindowManager>,
    key: PanelKey,
    app: &mut App,
) {
    if let Err(error) = manager.update(app, |manager, manager_cx| {
        manager.toggle_panel_from_menu(&key, manager_cx)
    }) {
        tracing::warn!(%error, panel = %key, "panel menu action failed");
    }
}

//! Application-facing GPUI action dispatch helpers.
//!
//! Keyboard action registration and default key bindings are added here when
//! the host exposes the corresponding keyboard interaction contract.

use dock_core::DockCommand;
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

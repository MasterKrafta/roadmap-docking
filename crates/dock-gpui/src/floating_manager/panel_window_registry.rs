use std::collections::HashMap;

use dock_core::{FloatingGroupId, PanelId, Rect};
use gpui::{WindowHandle, WindowId};
use platform::{NativeWindowId, PhysicalSize};

use crate::{components::FloatingHost, window_chain::WindowGridMember};

use super::{PanelLayoutLocation, PanelWindowEntry, PanelWindowHandle, PanelWindowVisibility};

/// Legacy native-window records keyed by a PanelId host.
///
/// Permanent Floating Header/Group windows are stored separately in `members`;
/// this map remains only for transitional PanelId-based host paths.
#[derive(Default)]
pub struct PanelWindowRegistry {
    /// Transitional PanelId entries are retained only for an active DragHost adoption.
    pub(super) windows: HashMap<PanelId, PanelWindowEntry>,
    /// Permanent Floating Header/Group windows are keyed by their Core-node identity.
    pub(super) members: HashMap<WindowGridMember, PanelWindowEntry>,
    pub(super) panel_locations: HashMap<PanelId, PanelLayoutLocation>,
}

impl PanelWindowRegistry {
    pub fn panel_count(&self) -> usize {
        self.windows.len()
    }

    pub fn visible_count(&self) -> usize {
        self.windows
            .values()
            .chain(self.members.values())
            .filter(|entry| entry.visibility == PanelWindowVisibility::Visible)
            .count()
    }



    pub fn window_for_panel(&self, panel: PanelId) -> Option<WindowHandle<FloatingHost>> {
        match self.windows.get(&panel)?.window {
            PanelWindowHandle::Floating(window) => Some(window),
            PanelWindowHandle::Part(_) | PanelWindowHandle::DragHost(_) => None,
        }
    }

    pub fn window_id_for_panel(&self, panel: PanelId) -> Option<WindowId> {
        self.windows.get(&panel).map(|entry| entry.window_id)
    }

    pub fn bounds_for_panel(&self, panel: PanelId) -> Option<Rect> {
        self.windows.get(&panel).map(|entry| entry.bounds)
    }

    pub fn native_min_size_for_panel(&self, panel: PanelId) -> Option<PhysicalSize> {
        self.windows.get(&panel)?.native_min_size
    }

    pub fn native_scale_factor_for_panel(&self, panel: PanelId) -> Option<f32> {
        self.windows.get(&panel)?.native_scale_factor
    }

    pub fn resize_enabled_for_panel(&self, panel: PanelId) -> Option<bool> {
        self.windows.get(&panel).map(|entry| entry.resize_enabled)
    }

    pub fn native_window_for_panel(&self, panel: PanelId) -> Option<NativeWindowId> {
        self.windows.get(&panel)?.native_window
    }

    pub fn layout_location_for_panel(&self, panel: PanelId) -> Option<PanelLayoutLocation> {
        self.panel_locations.get(&panel).copied()
    }

    pub fn host_group_for_panel(&self, panel: PanelId) -> Option<FloatingGroupId> {
        self.windows.get(&panel)?.host_group
    }

    pub fn visibility_for_panel(&self, panel: PanelId) -> Option<PanelWindowVisibility> {
        self.windows.get(&panel).map(|entry| entry.visibility)
    }
}

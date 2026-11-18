use std::collections::HashMap;

use dock_core::{FloatingGroupId, PanelId};
use gpui::WindowId;

/// Legacy mapping from a Core Floating composition to its PanelId host.
/// Permanent Floating windows no longer use this registry; Main still uses its
/// single shared MainWindow.
#[derive(Default)]
pub struct CompositionHostRegistry {
    pub(super) main_window_id: Option<WindowId>,
    pub(super) floating_groups: HashMap<FloatingGroupId, PanelId>,
}

impl CompositionHostRegistry {
    pub fn main_window_id(&self) -> Option<WindowId> {
        self.main_window_id
    }

    pub fn panel_for_group(&self, group: FloatingGroupId) -> Option<PanelId> {
        self.floating_groups.get(&group).copied()
    }

    pub fn group_for_panel(&self, panel: PanelId) -> Option<FloatingGroupId> {
        self.floating_groups
            .iter()
            .find_map(|(group, host)| (*host == panel).then_some(*group))
    }

    pub fn floating_count(&self) -> usize {
        self.floating_groups.len()
    }
}

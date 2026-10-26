use super::*;

impl Layout {
    pub(super) fn same_state(&self, other: &Self) -> bool {
        self.same_persistent_state(other)
            && self.flyout == other.flyout
            && self.flyout_auto_collapse == other.flyout_auto_collapse
            && self.flyout_widths == other.flyout_widths
            && self.flyout_width_order == other.flyout_width_order
    }

    pub(super) fn same_persistent_state(&self, other: &Self) -> bool {
        self.nodes.iter().eq(other.nodes.iter())
            && self.panel_instances.iter().eq(other.panel_instances.iter())
            && self.dock_areas.iter().eq(other.dock_areas.iter())
            && self.main_areas == other.main_areas
            && self.floating_groups.iter().eq(other.floating_groups.iter())
            && self.last_dock_locations == other.last_dock_locations
            && self.canvas == other.canvas
            && self.profile == other.profile
            && self.version == other.version
            && self.chrome == other.chrome
    }
}

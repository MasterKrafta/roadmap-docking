use super::*;

impl Layout {
    pub(super) fn same_state(&self, other: &Self) -> bool {
        self.same_persistent_state(other)
            && self.flyout == other.flyout
            && self.flyout_auto_collapse == other.flyout_auto_collapse
            && self.flyout_widths == other.flyout_widths
            && self.flyout_width_order == other.flyout_width_order
    }
}

use std::collections::HashMap;

use crate::window_chain::WindowGridMember;

use super::{PanelWindowEntry, PanelWindowVisibility};

/// Registry of native windows that physically represent Floating Columns.
///
/// A promoted DragHost remains in this registry under the same Column identity.
#[derive(Default)]
pub struct PanelWindowRegistry {
    pub(super) members: HashMap<WindowGridMember, PanelWindowEntry>,
}

impl PanelWindowRegistry {
    pub fn column_count(&self) -> usize {
        self.members.len()
    }

    pub fn visible_count(&self) -> usize {
        self.members
            .values()
            .filter(|entry| entry.visibility == PanelWindowVisibility::Visible)
            .count()
    }
}

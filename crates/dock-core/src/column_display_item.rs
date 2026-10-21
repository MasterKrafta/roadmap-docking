use crate::{NodeId, PanelId};

/// One panel entry in the canonical flattened display order of a column.
///
/// Rail adapters enumerate groups top-to-bottom and each group's tabs
/// left-to-right. This is presentation metadata only: it does not rewrite the
/// column tree, tab order, or active-tab selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ColumnDisplayItem {
    /// Tabs group that owns this tab.
    pub group: NodeId,
    /// Panel represented by this entry.
    pub panel: PanelId,
    /// Tab position within its owning group.
    pub tab_index: usize,
    /// Zero-based position in the flattened column order.
    pub display_index: usize,
}

impl ColumnDisplayItem {
    pub const fn new(
        group: NodeId,
        panel: PanelId,
        tab_index: usize,
        display_index: usize,
    ) -> Self {
        Self {
            group,
            panel,
            tab_index,
            display_index,
        }
    }
}

use dock_core::PanelId;

use super::tab_presentation::TabPresentation;

#[derive(Clone, Default)]
pub(crate) struct GroupPresentation {
    /// Effective state after Column overflow compaction.
    pub collapsed: bool,
    /// Persistent user-selected collapsed state in the Tabs model.
    pub model_collapsed: bool,
    /// This Group is temporarily collapsed by its overflowing Column.
    pub auto_collapsed: bool,
    /// This Group is the one temporarily expanded by user choice.
    pub auto_expanded: bool,
    pub active_panel: Option<PanelId>,
    pub tabs: Vec<TabPresentation>,
}

impl GroupPresentation {
    /// Number of tabs currently belonging to this Group.
    pub(crate) fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Whether this Group contains exactly one tab.
    pub(crate) fn is_single_tab(&self) -> bool {
        self.tab_count() == 1
    }
}

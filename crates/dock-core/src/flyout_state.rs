use crate::{NodeId, PanelId};

/// Transient selection shown in a column rail's flyout.
///
/// The selected panel is display state: opening a flyout must not change the
/// owning Tabs node's persistent active_index. group identifies the tab stack
/// that owns the panel and is retained for flyout geometry and validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlyoutState {
    pub column: NodeId,
    pub group: NodeId,
    pub panel: PanelId,
}

impl FlyoutState {
    pub const fn new(column: NodeId, group: NodeId, panel: PanelId) -> Self {
        Self {
            column,
            group,
            panel,
        }
    }
}

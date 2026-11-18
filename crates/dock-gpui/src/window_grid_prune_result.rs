use super::{WindowGridLink, WindowGridMember};

/// Reconciliation details produced when stale WindowGrid members are pruned.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct WindowGridPruneResult {
    pub(crate) removed_members: usize,
    pub(crate) reconnect_candidates: Vec<WindowGridLink>,
    pub(crate) components_to_align: Vec<WindowGridMember>,
}


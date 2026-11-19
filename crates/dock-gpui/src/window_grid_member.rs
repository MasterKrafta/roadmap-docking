use dock_core::{FloatingGroupId, NodeId};

/// Identity used by the logical WindowGrid while native Column HWNDs are materialized.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum WindowGridMember {
    /// Short-lived Core-level endpoint used while a FloatingGroup is waiting for
    /// its physical Column HWND entries or while an external group link is migrated.
    /// This variant never represents a native window.
    FloatingComposition(FloatingGroupId),
    /// The only persistent Floating HWND member: Header + all Groups of one Column.
    Column(NodeId),
}

impl WindowGridMember {
    pub(crate) const fn floating_composition(group: FloatingGroupId) -> Self {
        Self::FloatingComposition(group)
    }

    /// Returns a Core FloatingGroup only for the temporary composition endpoint.
    pub(crate) const fn floating_group(self) -> Option<FloatingGroupId> {
        match self {
            Self::FloatingComposition(group) => Some(group),
            Self::Column(_) => None,
        }
    }
}

impl From<FloatingGroupId> for WindowGridMember {
    fn from(group: FloatingGroupId) -> Self {
        Self::FloatingComposition(group)
    }
}

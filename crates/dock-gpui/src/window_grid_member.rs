use dock_core::{FloatingGroupId, NodeId};

/// Stable identity of one persistent native member in a Floating WindowGrid.
///
/// FloatingComposition is retained only as a compatibility endpoint for external
/// links. Every persistent Floating HWND is represented by one Core Column.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum WindowGridMember {
    FloatingComposition(FloatingGroupId),
    /// Complete native window containing one Core Column's Header and Group stack.
    Column(NodeId),
}

impl WindowGridMember {
    pub(crate) const fn floating_composition(group: FloatingGroupId) -> Self {
        Self::FloatingComposition(group)
    }

    /// Returns the legacy Core FloatingGroup represented by a compatibility endpoint.
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

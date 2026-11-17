use dock_core::{FloatingGroupId, NodeId};

/// Stable identity of one persistent native member in a Floating WindowGrid.
///
/// FloatingComposition is a compatibility bridge for the current one-window-per-
/// composition implementation. New Floating hosts will use Header and Group, so
/// a Core FloatingGroupId is no longer overloaded as the identity of every HWND.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[allow(dead_code)] // Header and Group variants are consumed by the dedicated-window stage.
pub(crate) enum WindowGridMember {
    FloatingComposition(FloatingGroupId),
    Header(NodeId),
    Group(NodeId),
}

impl WindowGridMember {
    pub(crate) const fn floating_composition(group: FloatingGroupId) -> Self {
        Self::FloatingComposition(group)
    }

    /// Returns the legacy Core FloatingGroup represented by this member.
    /// Header and Group members will resolve through the dedicated host registry.
    pub(crate) const fn floating_group(self) -> Option<FloatingGroupId> {
        match self {
            Self::FloatingComposition(group) => Some(group),
            Self::Header(_) | Self::Group(_) => None,
        }
    }
}

impl From<FloatingGroupId> for WindowGridMember {
    fn from(group: FloatingGroupId) -> Self {
        Self::FloatingComposition(group)
    }
}


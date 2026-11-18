use dock_core::{FloatingGroupId, NodeId};

/// Stable identity of one persistent native member in a Floating WindowGrid.
///
/// FloatingComposition remains a compatibility endpoint for older chain links.
/// Current Floating presentation uses one member per Core Column; Header and Group
/// variants are retained only for transitional/legacy host paths.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[allow(dead_code)] // Header and Group variants remain for transitional migration paths.
pub(crate) enum WindowGridMember {
    FloatingComposition(FloatingGroupId),
    /// Complete native window containing one Core Column's Header and Group stack.
    Column(NodeId),
    Header(NodeId),
    Group(NodeId),
}

impl WindowGridMember {
    pub(crate) const fn floating_composition(group: FloatingGroupId) -> Self {
        Self::FloatingComposition(group)
    }

    /// Returns the legacy Core FloatingGroup represented by this member.
    /// Column, Header and Group members resolve through the member-owner registry.
    pub(crate) const fn floating_group(self) -> Option<FloatingGroupId> {
        match self {
            Self::FloatingComposition(group) => Some(group),
            Self::Column(_) | Self::Header(_) | Self::Group(_) => None,
        }
    }
}

impl From<FloatingGroupId> for WindowGridMember {
    fn from(group: FloatingGroupId) -> Self {
        Self::FloatingComposition(group)
    }
}


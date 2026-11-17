use super::{WindowGridDirection, WindowGridMember};

/// One directed description of an undirected physical adjacency.
///
/// The reverse relationship is derived with `WindowGridDirection::opposite`;
/// the registry never stores both directions of the same connection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct WindowGridLink {
    pub(crate) from: WindowGridMember,
    pub(crate) to: WindowGridMember,
    pub(crate) direction: WindowGridDirection,
}

impl WindowGridLink {
    pub(super) fn new(
        from: WindowGridMember,
        to: WindowGridMember,
        direction: WindowGridDirection,
    ) -> Option<Self> {
        (from != to).then_some(Self {
            from,
            to,
            direction,
        })
    }
}

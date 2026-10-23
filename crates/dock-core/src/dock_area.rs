use crate::{DockAreaKind, DockSide, NodeId};

/// A root reference for one main or floating dock area.
///
/// The tree itself is owned by Layout's node arena. DockArea stores only its
/// identity, placement metadata, and root handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DockArea {
    pub kind: DockAreaKind,
    /// Present only for main areas; floating areas have no dock side.
    pub side: Option<DockSide>,
    pub root: NodeId,
}

impl DockArea {
    pub const fn main(side: DockSide, root: NodeId) -> Self {
        Self {
            kind: DockAreaKind::Main,
            side: Some(side),
            root,
        }
    }

    pub const fn floating(root: NodeId) -> Self {
        Self {
            kind: DockAreaKind::Floating,
            side: None,
            root,
        }
    }

    /// Checks that area kind and side metadata are consistent.
    pub const fn validate_metadata(self) -> Result<(), &'static str> {
        match (self.kind, self.side) {
            (DockAreaKind::Main, Some(_)) => Ok(()),
            (DockAreaKind::Floating, None) => Ok(()),
            (DockAreaKind::Main, None) => Err("main dock area must have a side"),
            (DockAreaKind::Floating, Some(_)) => Err("floating dock area must not have a side"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{DockArea, DockAreaKind, DockSide, NodeId};
    use slotmap::SlotMap;

    #[test]
    fn constructors_keep_kind_and_side_consistent() {
        let mut nodes = SlotMap::<NodeId, ()>::with_key();
        let root = nodes.insert(());
        let main = DockArea::main(DockSide::Left, root);
        assert_eq!(main.kind, DockAreaKind::Main);
        assert_eq!(main.side, Some(DockSide::Left));
        assert_eq!(main.validate_metadata(), Ok(()));

        let floating = DockArea::floating(root);
        assert_eq!(floating.kind, DockAreaKind::Floating);
        assert_eq!(floating.side, None);
        assert_eq!(floating.validate_metadata(), Ok(()));
    }
}

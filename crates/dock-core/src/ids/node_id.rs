slotmap::new_key_type! {
    /// Ephemeral internal identity of a node in the layout arena.
    pub struct NodeId;
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use slotmap::SlotMap;

    use crate::ids::{FloatingGroupId, PanelId};

    use super::NodeId;

    #[test]
    fn internal_id_types_are_distinct() {
        assert_ne!(TypeId::of::<NodeId>(), TypeId::of::<PanelId>());
        assert_ne!(TypeId::of::<NodeId>(), TypeId::of::<FloatingGroupId>());
        assert_ne!(TypeId::of::<PanelId>(), TypeId::of::<FloatingGroupId>());
    }

    #[test]
    fn removed_key_does_not_resolve_after_slot_reuse() {
        let mut nodes: SlotMap<NodeId, &'static str> = SlotMap::with_key();

        let removed = nodes.insert("removed");
        assert_eq!(nodes.remove(removed), Some("removed"));
        assert!(nodes.get(removed).is_none());

        let replacement = nodes.insert("replacement");
        assert_ne!(removed, replacement);
        assert!(nodes.get(removed).is_none());
        assert_eq!(nodes.get(replacement), Some(&"replacement"));
    }
}

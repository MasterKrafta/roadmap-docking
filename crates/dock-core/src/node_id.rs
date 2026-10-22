use slotmap::new_key_type;

new_key_type! {
    /// Runtime identifier for a node in a layout tree.
    pub struct NodeId;
}

#[cfg(test)]
mod tests {
    use super::NodeId;
    use slotmap::SlotMap;

    #[test]
    fn removed_node_id_no_longer_resolves() {
        let mut nodes = SlotMap::<NodeId, &'static str>::with_key();
        let id = nodes.insert("panel node");
        assert_eq!(nodes.get(id), Some(&"panel node"));
        assert_eq!(nodes.remove(id), Some("panel node"));
        assert!(nodes.get(id).is_none());
    }
}

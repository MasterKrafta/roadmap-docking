use slotmap::new_key_type;

new_key_type! {
    /// Runtime identifier for a floating group.
    pub struct FloatingGroupId;
}

#[cfg(test)]
mod tests {
    use slotmap::SlotMap;
    use super::FloatingGroupId;

    #[test]
    fn removed_floating_group_id_no_longer_resolves() {
        let mut groups = SlotMap::<FloatingGroupId, &'static str>::with_key();
        let id = groups.insert("floating stack");
        assert_eq!(groups.get(id), Some(&"floating stack"));
        assert_eq!(groups.remove(id), Some("floating stack"));
        assert!(groups.get(id).is_none());
    }
}

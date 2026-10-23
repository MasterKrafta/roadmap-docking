use slotmap::new_key_type;

new_key_type! {
    /// Runtime identifier for a main or floating dock area.
    pub struct DockAreaId;
}

#[cfg(test)]
mod tests {
    use super::DockAreaId;
    use slotmap::SlotMap;

    #[test]
    fn removed_dock_area_id_no_longer_resolves() {
        let mut areas = SlotMap::<DockAreaId, &'static str>::with_key();
        let id = areas.insert("main dock");
        assert_eq!(areas.get(id), Some(&"main dock"));
        assert_eq!(areas.remove(id), Some("main dock"));
        assert!(areas.get(id).is_none());
    }
}

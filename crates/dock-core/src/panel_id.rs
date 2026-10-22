use slotmap::new_key_type;

new_key_type! {
    /// Runtime identifier for one panel instance.
    pub struct PanelId;
}

#[cfg(test)]
mod tests {
    use super::PanelId;
    use slotmap::SlotMap;

    #[test]
    fn removed_panel_id_no_longer_resolves() {
        let mut panels = SlotMap::<PanelId, &'static str>::with_key();
        let id = panels.insert("layers instance");
        assert_eq!(panels.get(id), Some(&"layers instance"));
        assert_eq!(panels.remove(id), Some("layers instance"));
        assert!(panels.get(id).is_none());
    }
}

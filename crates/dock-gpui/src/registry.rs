#[path = "registry/icon_source.rs"]
mod icon_source;
#[path = "registry/panel_descriptor.rs"]
mod panel_descriptor;
#[path = "registry/panel_registry.rs"]
mod panel_registry;
#[path = "registry/panel_registry_error.rs"]
mod panel_registry_error;
#[path = "registry/panel_view_cache.rs"]
mod panel_view_cache;

pub use icon_source::IconSource;
pub use panel_descriptor::PanelDescriptor;
pub use panel_registry::PanelRegistry;
pub use panel_registry_error::PanelRegistryError;
pub(crate) use panel_view_cache::PanelViewCache;

#[cfg(test)]
mod tests {
    use dock_core::{PanelConstraints, PanelId, PanelKey, Size};
    use slotmap::SlotMap;

    use super::{IconSource, PanelDescriptor, PanelRegistry, PanelRegistryError, PanelViewCache};

    fn descriptor(key: &str) -> PanelDescriptor {
        PanelDescriptor::new(
            PanelKey::new(key).unwrap(),
            key,
            IconSource::None,
            PanelConstraints::new(Size::new(120.0, 80.0), Size::new(280.0, 220.0), None, true),
            |_, _| unreachable!("panel factories are lazy"),
        )
    }

    #[test]
    fn panel_view_cache_keeps_instances_when_their_layout_location_changes() {
        let mut ids = SlotMap::<PanelId, ()>::with_key();
        let moved_panel = ids.insert(());
        let closed_panel = ids.insert(());
        let mut views = PanelViewCache::<&'static str>::default();

        views.get_or_insert_with(moved_panel, || "the original entity");
        views.get_or_insert_with(closed_panel, || "another entity");

        // Docking, floating, merging, and tab activation leave the open PanelId
        // set unchanged, so those operations must retain its exact cached view.
        views.retain(|panel| panel == moved_panel);

        assert_eq!(views.len(), 1);
        assert_eq!(views.get(moved_panel), Some(&"the original entity"));
        assert_eq!(views.get(closed_panel), None);
    }

    #[test]
    fn panel_view_cache_creates_a_view_once_and_never_replaces_it() {
        let mut ids = SlotMap::<PanelId, ()>::with_key();
        let panel = ids.insert(());
        let mut views = PanelViewCache::<&'static str>::default();
        let mut factory_calls = 0;

        let first = views.get_or_insert_with(panel, || {
            factory_calls += 1;
            "original entity"
        });
        assert_eq!(*first, "original entity");

        let second = views.get_or_insert_with(panel, || {
            factory_calls += 1;
            "replacement entity"
        });
        assert_eq!(*second, "original entity");
        assert_eq!(factory_calls, 1);
        assert_eq!(views.len(), 1);
    }

    #[test]
    fn panel_view_cache_can_be_cleared_when_the_workspace_is_replaced() {
        let mut ids = SlotMap::<PanelId, ()>::with_key();
        let panel = ids.insert(());
        let mut views = PanelViewCache::<&'static str>::default();
        views.get_or_insert_with(panel, || "entity");

        views.clear();

        assert_eq!(views.len(), 0);
        assert_eq!(views.get(panel), None);
    }

    #[test]
    fn registry_exports_sorted_core_descriptors() {
        let mut registry = PanelRegistry::new();
        registry.register(descriptor("history")).unwrap();
        registry.register(descriptor("color")).unwrap();
        registry.register(descriptor("layers")).unwrap();

        let keys = registry
            .descriptors_for_core()
            .into_iter()
            .map(|(key, _)| key.to_string())
            .collect::<Vec<_>>();

        assert_eq!(
            keys,
            vec![
                "color".to_owned(),
                "history".to_owned(),
                "layers".to_owned()
            ]
        );
    }

    #[test]
    fn registry_exposes_descriptors_in_sorted_key_order_for_menus() {
        let mut registry = PanelRegistry::new();
        registry.register(descriptor("history")).unwrap();
        registry.register(descriptor("color")).unwrap();
        registry.register(descriptor("layers")).unwrap();

        let keys = registry
            .descriptors()
            .into_iter()
            .map(|descriptor| descriptor.key().to_string())
            .collect::<Vec<_>>();

        assert_eq!(
            keys,
            vec![
                "color".to_owned(),
                "history".to_owned(),
                "layers".to_owned()
            ]
        );
    }

    #[test]
    fn registry_rejects_duplicate_keys() {
        let mut registry = PanelRegistry::new();
        registry.register(descriptor("layers")).unwrap();
        assert!(matches!(
            registry.register(descriptor("layers")),
            Err(PanelRegistryError::DuplicateKey { .. })
        ));
    }

    #[test]
    fn registry_rejects_invalid_constraints_without_inserting() {
        let mut registry = PanelRegistry::new();
        let invalid = PanelDescriptor::new(
            PanelKey::new("broken-panel").unwrap(),
            "Broken panel",
            IconSource::None,
            PanelConstraints::new(Size::new(320.0, 80.0), Size::new(280.0, 220.0), None, false),
            |_, _| unreachable!("panel factories are lazy"),
        );

        assert!(matches!(
            registry.register(invalid),
            Err(PanelRegistryError::InvalidConstraints { .. })
        ));
        assert!(registry.is_empty());
    }
}

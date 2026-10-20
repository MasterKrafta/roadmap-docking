use std::collections::HashMap;

use dock_core::{DockCommand, DropTarget, PanelConstraints, PanelKey, Size};

use super::{
    AllowedArea, IconResource, PanelContent, PanelContentFactory, PanelDescriptor, PanelMenuItem,
    PanelMenuSource, PlaceholderPanelContent, RegistryError,
};

/// App-owned index of descriptors. The core model receives only a key and copied
/// PanelConstraints through DockCommand::OpenPanel; factories, icons, and menu sources
/// never cross into dock-core.
#[derive(Default)]
pub struct PanelRegistry {
    descriptors: HashMap<PanelKey, PanelDescriptor>,
}

impl PanelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_builtins() -> Result<Self, RegistryError> {
        let mut registry = Self::new();
        for descriptor in [
            builtin_descriptor(
                "layers",
                "Layers",
                "builtin:layers",
                "Layer stack and visibility controls.",
            ),
            builtin_descriptor(
                "color",
                "Color",
                "builtin:color",
                "Color selection and foreground/background swatches.",
            ),
            builtin_descriptor(
                "properties",
                "Properties",
                "builtin:properties",
                "Properties for the selected document object.",
            ),
            builtin_descriptor(
                "history",
                "History",
                "builtin:history",
                "Recent editing operations.",
            ),
        ] {
            registry.register(descriptor)?;
        }
        Ok(registry)
    }

    pub fn register(&mut self, descriptor: PanelDescriptor) -> Result<(), RegistryError> {
        if let Some(reason) = descriptor.validation_error() {
            return Err(RegistryError::InvalidDescriptor {
                key: descriptor.key().clone(),
                reason,
            });
        }
        if self.descriptors.contains_key(descriptor.key()) {
            return Err(RegistryError::DuplicateKey(descriptor.key().clone()));
        }
        self.descriptors.insert(descriptor.key().clone(), descriptor);
        Ok(())
    }

    pub fn get(&self, key: &PanelKey) -> Option<&PanelDescriptor> {
        self.descriptors.get(key)
    }

    pub fn get_by_str(&self, key: &str) -> Option<&PanelDescriptor> {
        self.descriptors.values().find(|descriptor| descriptor.key().as_str() == key)
    }

    /// Returns descriptors in key order for deterministic menus and tests.
    pub fn descriptors(&self) -> Vec<&PanelDescriptor> {
        let mut descriptors: Vec<_> = self.descriptors.values().collect();
        descriptors.sort_by(|left, right| left.key().cmp(right.key()));
        descriptors
    }

    pub fn len(&self) -> usize {
        self.descriptors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }

    pub fn create_content(&self, key: &PanelKey) -> Result<Box<dyn PanelContent>, RegistryError> {
        self.get(key)
            .map(PanelDescriptor::create_content)
            .ok_or_else(|| RegistryError::UnknownPanel(key.clone()))
    }

    pub fn panel_menu_items(&self, key: &PanelKey) -> Result<Vec<PanelMenuItem>, RegistryError> {
        self.get(key)
            .map(PanelDescriptor::menu_items)
            .ok_or_else(|| RegistryError::UnknownPanel(key.clone()))
    }

    /// Translate a registered descriptor into the core OpenPanel command so callers
    /// cannot accidentally pass size constraints unrelated to the registered panel.
    pub fn open_command(
        &self,
        key: &PanelKey,
        target: Option<DropTarget>,
    ) -> Result<DockCommand, RegistryError> {
        let descriptor = self
            .get(key)
            .ok_or_else(|| RegistryError::UnknownPanel(key.clone()))?;

        let area = match target.as_ref() {
            Some(DropTarget::FloatingGroup { .. } | DropTarget::FreeArea { .. }) => {
                AllowedArea::Floating
            }
            None
            | Some(
                DropTarget::Group { .. }
                | DropTarget::TabInsertion { .. }
                | DropTarget::Column { .. }
                | DropTarget::DockEdge { .. },
            ) => AllowedArea::Dock,
        };
        if !descriptor.allows(area) {
            return Err(RegistryError::AreaNotAllowed {
                key: key.clone(),
                area,
            });
        }

        Ok(DockCommand::OpenPanel {
            key: descriptor.key().clone(),
            constraints: descriptor.constraints(),
            target,
        })
    }
}

fn builtin_descriptor(
    key: &str,
    title: &str,
    icon: &str,
    message: &str,
) -> PanelDescriptor {
    let key = PanelKey::new(key).expect("built-in panel key is valid");
    let content = PlaceholderPanelContent::new(key.clone(), title, message);
    let constraints = PanelConstraints::new(
        Size::new(200.0, 80.0),
        Size::new(280.0, 240.0),
        Size::new(f32::MAX, f32::MAX),
        true,
    );
    let icon = IconResource::new(icon).expect("built-in icon resource id is valid");
    let content_factory = PanelContentFactory::new(move || content.clone());

    PanelDescriptor::new(
        key,
        title,
        icon,
        constraints,
        vec![AllowedArea::Dock, AllowedArea::Floating, AllowedArea::Rail],
        content_factory,
        PanelMenuSource::empty(),
    )
}

#[cfg(test)]
mod tests {
    use super::PanelRegistry;
    use crate::panels::{
        AllowedArea, IconResource, PanelContentFactory, PanelDescriptor, PanelMenuItem,
        PanelMenuSource, PlaceholderPanelContent, RegistryError,
    };
    use dock_core::{
        CanvasState, DockCommand, DockMetrics, DropTarget, Layout, PanelConstraints, PanelKey,
        Point, Profile, Rect, Size,
    };

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct CustomPanelContent {
        heading: String,
    }

    fn custom_descriptor(
        key: &str,
        areas: Vec<AllowedArea>,
    ) -> PanelDescriptor {
        let key = PanelKey::new(key).expect("test key is valid");
        let content_factory =
            PanelContentFactory::new(|| CustomPanelContent { heading: "Custom".to_owned() });
        let menu_source = PanelMenuSource::new(|| {
            vec![
                PanelMenuItem::new("reset", "Reset"),
                PanelMenuItem::new("advanced", "Advanced").with_separator_before(),
            ]
        });
        PanelDescriptor::new(
            key,
            "Custom Panel",
            IconResource::new("test:custom").expect("valid icon key"),
            PanelConstraints::new(
                Size::new(120.0, 80.0),
                Size::new(240.0, 160.0),
                Size::new(640.0, 480.0),
                false,
            ),
            areas,
            content_factory,
            menu_source,
        )
    }

    #[test]
    fn custom_panel_registers_and_creates_content_without_a_core_variant() {
        let mut registry = PanelRegistry::new();
        let key = PanelKey::new("custom.adjustments").unwrap();
        registry
            .register(custom_descriptor(
                key.as_str(),
                vec![AllowedArea::Dock, AllowedArea::Floating],
            ))
            .expect("custom descriptor should register");

        assert_eq!(registry.len(), 1);
        let descriptor = registry.get(&key).expect("custom panel is registered");
        assert_eq!(descriptor.title(), "Custom Panel");
        assert_eq!(descriptor.icon().as_str(), "test:custom");
        assert!(descriptor.allows(AllowedArea::Dock));
        assert!(descriptor.allows(AllowedArea::Floating));
        assert!(!descriptor.allows(AllowedArea::Rail));

        let content = registry.create_content(&key).expect("content factory should run");
        let content = content
            .as_ref()
            .as_any()
            .downcast_ref::<CustomPanelContent>()
            .expect("factory returns the registered panel's content type");
        assert_eq!(content.heading, "Custom");

        let items = registry.panel_menu_items(&key).expect("panel menu source runs");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "reset");
        assert!(items[1].separator_before);
        assert!(
            items.iter().all(|item| item.id != "close" && item.id != "close_tab_group"),
            "shared dock actions are composed by the dock UI"
        );

        let command = registry
            .open_command(&key, None)
            .expect("registered descriptor should produce OpenPanel");
        assert!(matches!(
            command.clone(),
            DockCommand::OpenPanel { key: command_key, constraints, target: None }
                if command_key == key
                    && constraints == PanelConstraints::new(
                        Size::new(120.0, 80.0),
                        Size::new(240.0, 160.0),
                        Size::new(640.0, 480.0),
                        false,
                    )
        ));

        let metrics = DockMetrics {
            column_header_height: 32.0,
            tab_strip_height: 28.0,
            splitter_thickness: 4.0,
            splitter_hit_zone: 8.0,
            edge_drop_zone_width: 24.0,
            drop_edge_zone: 24.0,
            drag_start_threshold: 4.0,
            default_column_width: 280.0,
            min_column_width: 200.0,
            rail_width: 36.0,
            min_group_height: 80.0,
            collapsed_group_height: 28.0,
            min_document_width: 320.0,
            max_columns: 4,
        };
        let mut layout = Layout::new(Profile::Photoshop, CanvasState::default());
        layout
            .apply(command, &metrics)
            .expect("the core should open a registered app panel from the generated command");
        let instance = layout
            .panels()
            .values()
            .find(|panel| panel.key().as_str() == "custom.adjustments")
            .expect("core created the custom panel instance");
        assert_eq!(
            instance.constraints(),
            &PanelConstraints::new(
                Size::new(120.0, 80.0),
                Size::new(240.0, 160.0),
                Size::new(640.0, 480.0),
                false,
            )
        );
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn registry_rejects_duplicate_keys_without_replacing_the_original() {
        let mut registry = PanelRegistry::new();
        let descriptor = custom_descriptor("custom.panel", vec![AllowedArea::Dock]);
        registry.register(descriptor).unwrap();

        assert!(matches!(
            registry.register(custom_descriptor("custom.panel", vec![AllowedArea::Floating])),
            Err(RegistryError::DuplicateKey(key)) if key.as_str() == "custom.panel"
        ));
        assert!(registry.get_by_str("custom.panel").unwrap().allows(AllowedArea::Dock));
        assert!(!registry.get_by_str("custom.panel").unwrap().allows(AllowedArea::Floating));
    }

    #[test]
    fn registry_validates_constraints_and_allowed_areas() {
        let mut registry = PanelRegistry::new();
        let mut invalid = custom_descriptor("custom.invalid", vec![]);
        assert!(matches!(
            registry.register(invalid.clone()),
            Err(RegistryError::InvalidDescriptor {
                key,
                reason: "at least one allowed area must be selected",
            }) if key.as_str() == "custom.invalid"
        ));

        invalid = PanelDescriptor::new(
            PanelKey::new("custom.invalid").unwrap(),
            "Invalid Constraints",
            IconResource::new("test:invalid").expect("valid icon key"),
            PanelConstraints::new(
                Size::new(320.0, 80.0),
                Size::new(240.0, 160.0),
                Size::new(640.0, 480.0),
                false,
            ),
            vec![AllowedArea::Dock],
            PanelContentFactory::new(|| CustomPanelContent { heading: "Invalid".into() }),
            PanelMenuSource::empty(),
        );
        assert!(matches!(
            registry.register(invalid),
            Err(RegistryError::InvalidDescriptor {
                reason: "panel sizes must satisfy minimum <= preferred <= maximum",
                ..
            })
        ));
    }

    #[test]
    fn open_command_respects_allowed_target_area_and_unknown_keys() {
        let mut registry = PanelRegistry::new();
        let key = PanelKey::new("custom.rail-only").unwrap();
        registry
            .register(custom_descriptor("custom.rail-only", vec![AllowedArea::Rail]))
            .unwrap();

        assert_eq!(
            registry.open_command(&key, None),
            Err(RegistryError::AreaNotAllowed {
                key: key.clone(),
                area: AllowedArea::Dock,
            })
        );

        let unknown = PanelKey::new("custom.missing").unwrap();
        assert_eq!(
            registry.open_command(&unknown, None),
            Err(RegistryError::UnknownPanel(unknown))
        );
    }

    #[test]
    fn builtin_descriptors_cover_initial_photoshop_panels() {
        let registry = PanelRegistry::with_builtins().expect("built-ins are valid");
        assert_eq!(registry.len(), 4);
        assert_eq!(
            registry.descriptors().iter().map(|descriptor| descriptor.key().as_str()).collect::<Vec<_>>(),
            vec!["color", "history", "layers", "properties"]
        );

        let descriptor = registry.get_by_str("layers").expect("layers descriptor exists");
        let content = descriptor.create_content();
        let content = content
            .as_ref()
            .as_any()
            .downcast_ref::<PlaceholderPanelContent>()
            .expect("built-in content uses a framework-neutral placeholder model");
        assert_eq!(content.panel_key.as_str(), "layers");
        assert_eq!(content.heading, "Layers");
        assert!(content.message.contains("visibility"));
    }

    #[test]
    fn floating_open_command_requires_floating_permission() {
        let mut registry = PanelRegistry::new();
        let key = PanelKey::new("custom.dock-only").unwrap();
        registry
            .register(custom_descriptor("custom.dock-only", vec![AllowedArea::Dock]))
            .unwrap();

        assert_eq!(
            registry.open_command(
                &key,
                Some(DropTarget::FreeArea {
                    bounds: Rect::new(Point::new(100.0, 100.0), Size::new(240.0, 160.0)),
                }),
            ),
            Err(RegistryError::AreaNotAllowed {
                key,
                area: AllowedArea::Floating,
            })
        );
    }
}

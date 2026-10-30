use std::collections::HashMap;

use dock_core::{GeometryError, PanelConstraints, PanelKey};
use gpui::{AnyView, App, SharedString, Window};

/// An icon reference supplied by the application for a panel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IconSource {
    /// A path resolved by the GPUI asset source.
    Asset(SharedString),
    /// The descriptor intentionally has no icon.
    None,
}

type PanelFactory = dyn Fn(&mut Window, &mut App) -> AnyView + 'static;

/// Application-owned metadata and view factory for one panel kind.
///
/// The descriptor contains no runtime panel ID and no docking state. Constraints
/// are copied into dock-core when an instance is opened; the factory is called
/// lazily by DockController when the panel is first rendered.
pub struct PanelDescriptor {
    key: PanelKey,
    title: SharedString,
    icon: IconSource,
    constraints: PanelConstraints,
    factory: Box<PanelFactory>,
}

impl PanelDescriptor {
    /// Creates a descriptor. PanelRegistry::register validates its constraints
    /// and rejects duplicate stable keys.
    pub fn new(
        key: PanelKey,
        title: impl Into<SharedString>,
        icon: IconSource,
        constraints: PanelConstraints,
        factory: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        Self {
            key,
            title: title.into(),
            icon,
            constraints,
            factory: Box::new(factory),
        }
    }

    pub fn key(&self) -> &PanelKey {
        &self.key
    }

    pub fn title(&self) -> &SharedString {
        &self.title
    }

    pub fn icon(&self) -> &IconSource {
        &self.icon
    }

    pub const fn constraints(&self) -> PanelConstraints {
        self.constraints
    }

    pub(crate) fn create_view(&self, window: &mut Window, app: &mut App) -> AnyView {
        (self.factory)(window, app)
    }
}

/// Errors produced while registering panel descriptors.
#[derive(Debug, thiserror::Error)]
pub enum PanelRegistryError {
    #[error("panel key '{key}' is already registered")]
    DuplicateKey { key: PanelKey },

    #[error("panel '{key}' has invalid constraints: {source}")]
    InvalidConstraints {
        key: PanelKey,
        #[source]
        source: GeometryError,
    },
}

/// Registry of application-provided panel descriptors.
///
/// This is not a second docking model: it contains stable panel kinds and their
/// factories only. Runtime instances, their IDs, and their placement belong to
/// dock_core::Layout.
#[derive(Default)]
pub struct PanelRegistry {
    descriptors: HashMap<PanelKey, PanelDescriptor>,
}

impl PanelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a descriptor after validating its constraints.
    pub fn register(
        &mut self,
        descriptor: PanelDescriptor,
    ) -> Result<(), PanelRegistryError> {
        let key = descriptor.key.clone();
        if self.descriptors.contains_key(&key) {
            return Err(PanelRegistryError::DuplicateKey { key });
        }
        descriptor.constraints.validate().map_err(|source| {
            PanelRegistryError::InvalidConstraints {
                key: key.clone(),
                source,
            }
        })?;
        self.descriptors.insert(key, descriptor);
        Ok(())
    }

    pub fn get(&self, key: &PanelKey) -> Option<&PanelDescriptor> {
        self.descriptors.get(key)
    }

    pub fn contains(&self, key: &PanelKey) -> bool {
        self.descriptors.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.descriptors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }

    /// Produces the deterministic, UI-independent descriptor list required by
    /// create_initial_layout and WorkspaceFile::load_json.
    pub fn descriptors_for_core(&self) -> Vec<(PanelKey, PanelConstraints)> {
        let mut descriptors = self
            .descriptors
            .values()
            .map(|descriptor| (descriptor.key.clone(), descriptor.constraints))
            .collect::<Vec<_>>();
        descriptors.sort_by(|left, right| left.0.cmp(&right.0));
        descriptors
    }
}

#[cfg(test)]
mod tests {
    use dock_core::{PanelConstraints, PanelKey, Size};

    use super::{IconSource, PanelDescriptor, PanelRegistry, PanelRegistryError};

    fn descriptor(key: &str) -> PanelDescriptor {
        PanelDescriptor::new(
            PanelKey::new(key).unwrap(),
            key,
            IconSource::None,
            PanelConstraints::new(
                Size::new(120.0, 80.0),
                Size::new(280.0, 220.0),
                None,
                true,
            ),
            |_, _| unreachable!("panel factories are lazy"),
        )
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
    fn registry_rejects_duplicate_keys() {
        let mut registry = PanelRegistry::new();
        registry.register(descriptor("layers")).unwrap();
        assert!(matches!(
            registry.register(descriptor("layers")),
            Err(PanelRegistryError::DuplicateKey { .. })
        ));
    }
}

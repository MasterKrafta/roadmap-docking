use std::collections::HashMap;

use dock_core::{PanelConstraints, PanelKey};

use super::{PanelDescriptor, PanelRegistryError};

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
    pub fn register(&mut self, descriptor: PanelDescriptor) -> Result<(), PanelRegistryError> {
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

    /// Returns panel descriptors in stable key order for application menus.
    ///
    /// The registry remains the source of panel types; open/checked state must
    /// always be derived from the current Core Layout.
    pub fn descriptors(&self) -> Vec<&PanelDescriptor> {
        let mut descriptors = self.descriptors.values().collect::<Vec<_>>();
        descriptors.sort_by(|left, right| left.key.cmp(&right.key));
        descriptors
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

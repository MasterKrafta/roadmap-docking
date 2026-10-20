use std::{collections::HashSet, fmt};

use dock_core::PanelConstraints;

use super::{
    AllowedArea, IconResource, PanelContent, PanelContentFactory, PanelKey, PanelMenuItem,
    PanelMenuSource,
};

/// Application-owned metadata needed to register, create, and place a panel.
#[derive(Clone)]
pub struct PanelDescriptor {
    key: PanelKey,
    title: String,
    icon: IconResource,
    constraints: PanelConstraints,
    allowed_areas: Vec<AllowedArea>,
    content_factory: PanelContentFactory,
    menu_source: PanelMenuSource,
}

impl PanelDescriptor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        key: PanelKey,
        title: impl Into<String>,
        icon: IconResource,
        constraints: PanelConstraints,
        allowed_areas: Vec<AllowedArea>,
        content_factory: PanelContentFactory,
        menu_source: PanelMenuSource,
    ) -> Self {
        Self {
            key,
            title: title.into(),
            icon,
            constraints,
            allowed_areas,
            content_factory,
            menu_source,
        }
    }

    pub fn key(&self) -> &PanelKey {
        &self.key
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn icon(&self) -> &IconResource {
        &self.icon
    }

    pub const fn constraints(&self) -> PanelConstraints {
        self.constraints
    }

    pub fn allowed_areas(&self) -> &[AllowedArea] {
        &self.allowed_areas
    }

    pub fn allows(&self, area: AllowedArea) -> bool {
        self.allowed_areas.contains(&area)
    }

    pub fn create_content(&self) -> Box<dyn PanelContent> {
        self.content_factory.create()
    }

    pub fn menu_items(&self) -> Vec<PanelMenuItem> {
        self.menu_source.items()
    }

    pub(super) fn validation_error(&self) -> Option<&'static str> {
        if self.title.trim().is_empty() {
            return Some("panel title must not be empty");
        }
        if self.allowed_areas.is_empty() {
            return Some("at least one allowed area must be selected");
        }
        let unique_areas: HashSet<_> = self.allowed_areas.iter().copied().collect();
        if unique_areas.len() != self.allowed_areas.len() {
            return Some("allowed areas must not contain duplicates");
        }

        let min = self.constraints.min_size();
        let preferred = self.constraints.preferred_size();
        let max = self.constraints.max_size();
        let values = [
            min.width,
            min.height,
            preferred.width,
            preferred.height,
            max.width,
            max.height,
        ];
        if values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Some("panel size constraints must be finite and non-negative");
        }
        if min.width > preferred.width
            || min.height > preferred.height
            || preferred.width > max.width
            || preferred.height > max.height
        {
            return Some("panel sizes must satisfy minimum <= preferred <= maximum");
        }

        None
    }
}

impl fmt::Debug for PanelDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PanelDescriptor")
            .field("key", &self.key)
            .field("title", &self.title)
            .field("icon", &self.icon)
            .field("constraints", &self.constraints)
            .field("allowed_areas", &self.allowed_areas)
            .field("content_factory", &self.content_factory)
            .field("menu_source", &self.menu_source)
            .finish()
    }
}

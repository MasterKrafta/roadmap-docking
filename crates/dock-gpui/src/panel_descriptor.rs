use dock_core::{PanelConstraints, PanelKey};
use gpui::{AnyView, App, SharedString, Window};

use super::IconSource;

type PanelFactory = dyn Fn(&mut Window, &mut App) -> AnyView + 'static;

/// Application-owned metadata and view factory for one panel kind.
///
/// The descriptor contains no runtime panel ID and no docking state. Constraints
/// are copied into dock-core when an instance is opened. DockHost calls the
/// factory once per open PanelId when it synchronizes the runtime view cache;
/// this may happen before an inactive tab is actually mounted.
pub struct PanelDescriptor {
    pub(super) key: PanelKey,
    title: SharedString,
    icon: IconSource,
    pub(super) constraints: PanelConstraints,
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

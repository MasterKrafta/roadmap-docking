//! High-level GPUI workspace integration for downstream applications.
//!
//! Applications provide panel definitions (stable ID, title, and content
//! factory). This module owns registry setup, the baseline Core layout,
//! controller/host creation, and the application-facing panel visibility menu.

use std::rc::Rc;

use dock_core::{
    DockMetrics, InitialLayoutError, PanelConstraints, PanelKey, PanelKeyError, Point, Profile,
    Rect, Size,
};
use gpui::{AnyView, App, Context, Entity, IntoElement, Render, SharedString, Window};
use thiserror::Error;

use crate::{
    ContextMenu, ContextMenuCallbacks, ContextMenuItem, DockController,
    DockControllerConfigError, DockHost, DockTheme, IconSource, PanelDescriptor,
    PanelRegistry, PanelRegistryError,
};

type PanelFactory = dyn Fn(&mut Window, &mut App) -> AnyView + 'static;

/// A panel definition supplied by the application.
///
/// The ID and title identify the panel in the docking system and in the
/// workspace visibility menu. The factory creates only the panel's content;
/// placement, visibility, and docking behavior remain owned by the library.
pub struct PanelDefinition {
    key: PanelKey,
    title: SharedString,
    icon: IconSource,
    constraints: PanelConstraints,
    factory: Box<PanelFactory>,
}

impl PanelDefinition {
    /// Creates a panel definition with sensible default size constraints.
    ///
    /// Keys use lowercase ASCII letters/digits and single hyphens, for example
    /// layers, history, or asset-browser.
    pub fn new(
        key: impl Into<String>,
        title: impl Into<SharedString>,
        factory: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> Result<Self, PanelKeyError> {
        Ok(Self {
            key: PanelKey::new(key)?,
            title: title.into(),
            icon: IconSource::None,
            constraints: PanelConstraints::new(
                Size::new(120.0, 80.0),
                Size::new(320.0, 280.0),
                None,
                false,
            ),
            factory: Box::new(factory),
        })
    }

    /// Sets an optional icon shown by dock presentation components.
    pub fn with_icon(mut self, icon: IconSource) -> Self {
        self.icon = icon;
        self
    }

    /// Overrides the default minimum/preferred/maximum panel sizes.
    pub fn with_constraints(mut self, constraints: PanelConstraints) -> Self {
        self.constraints = constraints;
        self
    }

    fn into_descriptor(self) -> PanelDescriptor {
        PanelDescriptor::new(
            self.key,
            self.title,
            self.icon,
            self.constraints,
            self.factory,
        )
    }
}

/// Errors while assembling an application-provided docking workspace.
#[derive(Debug, Error)]
pub enum DockWorkspaceError {
    #[error(transparent)]
    PanelRegistry(#[from] PanelRegistryError),

    #[error(transparent)]
    InitialLayout(#[from] InitialLayoutError),

    #[error(transparent)]
    Controller(#[from] DockControllerConfigError),
}

#[derive(Clone)]
struct PanelMenuDefinition {
    key: PanelKey,
    title: SharedString,
}

/// Ready-to-mount docking workspace for an application.
///
/// DockWorkspace owns the registry, Core-backed controller, and DockHost.
/// The app only supplies panel definitions and chooses where to mount this
/// view in its own window/menu composition.
///
/// The current baseline layout is Photoshop-style and requires the four
/// built-in keys: layers, color, properties, and history. Any additional
/// definitions are automatically registered and appear in the panel
/// visibility menu; they are opened on demand from that menu.
pub struct DockWorkspace {
    controller: Entity<DockController>,
    host: Entity<DockHost>,
    panel_menu: Vec<PanelMenuDefinition>,
}

impl DockWorkspace {
    /// Builds the registry, Photoshop baseline layout, controller, and host.
    ///
    /// The supplied viewport is the initial dock-content area, excluding the
    /// application's own menu bar/titlebar. The host continues to own all dock
    /// rendering and pointer interaction after construction.
    pub fn new(
        panels: impl IntoIterator<Item = PanelDefinition>,
        viewport: Rect,
        cx: &mut Context<Self>,
    ) -> Result<Self, DockWorkspaceError> {
        let metrics = DockMetrics::default();
        let mut registry = PanelRegistry::new();
        let mut panel_menu = Vec::new();

        for panel in panels {
            panel_menu.push(PanelMenuDefinition {
                key: panel.key.clone(),
                title: panel.title.clone(),
            });
            registry.register(panel.into_descriptor())?;
        }

        let layout = crate::dock_core::create_initial_layout(
            Profile::Photoshop,
            &registry.descriptors_for_core(),
            metrics,
        )?;
        let controller = DockController::new(layout, metrics, registry, viewport)?;
        let controller = cx.new(|_| controller);
        let host = cx.new(|_| DockHost::new(controller.clone(), DockTheme::default()));

        Ok(Self {
            controller,
            host,
            panel_menu,
        })
    }

    /// Returns the underlying controller for advanced application integration.
    ///
    /// Most applications do not need this: use panel_menu for visibility
    /// toggles and mount DockWorkspace as a normal GPUI view.
    pub fn controller(&self) -> &Entity<DockController> {
        &self.controller
    }

    /// Renders the library-owned panel visibility popup.
    ///
    /// The application supplies only the popup placement and a dismiss
    /// callback. Each registered definition is reflected automatically, and
    /// selecting an entry opens or closes that panel through DockController.
    pub fn panel_menu(
        &self,
        id: impl Into<SharedString>,
        viewport: Rect,
        anchor: Point,
        on_dismiss: impl Fn(&mut App) + 'static,
        app: &mut App,
    ) -> impl IntoElement {
        let entries = self
            .panel_menu
            .iter()
            .map(|panel| {
                let selected = self.controller.update(app, |controller, _| {
                    controller
                        .layout()
                        .panel_instances()
                        .any(|(_, instance)| instance.key.as_str() == panel.key.as_str())
                });
                let key = panel.key.clone();
                let controller = self.controller.clone();

                ContextMenuItem::new(panel.title.clone(), move |app| {
                    controller.update(app, |controller, controller_cx| {
                        if let Err(error) = controller.toggle_panel(&key) {
                            tracing::warn!(%error, "could not toggle dock panel");
                        }
                        controller.flush_events(controller_cx);
                        controller_cx.notify();
                    });
                })
                .selected(selected)
                .into()
            })
            .collect::<Vec<_>>();

        ContextMenu::render(
            id,
            viewport,
            anchor,
            &entries,
            None,
            ContextMenuCallbacks {
                on_submenu: Rc::new(|_, _| {}),
                on_dismiss: Rc::new(on_dismiss),
            },
            DockTheme::default(),
        )
    }
}

impl Render for DockWorkspace {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.host.clone()
    }
}

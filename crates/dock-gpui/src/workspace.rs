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
use gpui::{
    AnyView, App, AppContext, Context, Entity, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, div,
};
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
    /// Most applications do not need this: create a DockPanelMenu entity
    /// for visibility toggles and mount DockWorkspace as a normal GPUI view.
    pub fn controller(&self) -> &Entity<DockController> {
        &self.controller
    }

    /// Creates a separate library-owned menu view that can be mounted over
    /// the application's menu bar. Returning an Entity avoids returning a
    /// borrowed element out of Entity::update.
    pub fn create_panel_menu(
        &self,
        viewport: Rect,
        anchor: Point,
        cx: &mut Context<Self>,
    ) -> Entity<DockPanelMenu> {
        let controller = self.controller.clone();
        let panels = self.panel_menu.clone();
        cx.new(|_| DockPanelMenu {
            controller,
            panels,
            viewport,
            anchor,
            open: false,
        })
    }
}

impl Render for DockWorkspace {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.host.clone()
    }
}

/// Library-owned overlay for opening and closing registered panels.
///
/// The application mounts this entity above its main content and toggles it
/// from the application's Window menu item. The list and all panel actions
/// are built here from the definitions passed to DockWorkspace.
pub struct DockPanelMenu {
    controller: Entity<DockController>,
    panels: Vec<PanelMenuDefinition>,
    viewport: Rect,
    anchor: Point,
    open: bool,
}

impl DockPanelMenu {
    /// Toggles the visibility of the popup menu.
    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        self.open = !self.open;
        cx.notify();
    }

    /// Explicitly sets the visibility of the popup menu.
    pub fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open != open {
            self.open = open;
            cx.notify();
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }
}

impl Render for DockPanelMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div().absolute().top_0().left_0().size_full();

        if !self.open {
            return root;
        }

        let entries = self
            .panels
            .iter()
            .map(|panel| {
                let selected = self.controller.update(cx, |controller, _| {
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

        let menu_entity = cx.entity().downgrade();
        let on_dismiss: Rc<dyn Fn(&mut App)> = Rc::new(move |app| {
            if let Some(menu) = menu_entity.upgrade() {
                menu.update(app, |menu, menu_cx| {
                    menu.open = false;
                    menu_cx.notify();
                });
            }
        });

        root = root.child(ContextMenu::render(
            "application-window-menu",
            self.viewport,
            self.anchor,
            entries,
            None,
            ContextMenuCallbacks {
                on_submenu: Rc::new(|_, _| {}),
                on_dismiss,
            },
            DockTheme::default(),
        ));

        root
    }
}


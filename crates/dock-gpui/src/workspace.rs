//! High-level GPUI workspace assembly for downstream applications.
//!
//! Applications provide stable panel IDs, titles, and content factories.
//! This module creates the registry, Core layout, controller, DockHost,
//! application Window menu, and the PanelHost-backed presentation pipeline.

use std::rc::Rc;

use dock_core::{
    DockMetrics, InitialLayoutError, PanelConstraints, PanelKey, PanelKeyError, Point, Profile,
    Rect, Size,
};
use gpui::{
    AnyView, App, AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div, px, rgb,
};
use thiserror::Error;

use crate::{
    ContextMenu, ContextMenuCallbacks, ContextMenuItem, DockController,
    DockControllerConfigError, DockHost, DockTheme, IconSource, PanelDescriptor,
    PanelRegistry, PanelRegistryError,
};

type PanelFactory = dyn Fn(&mut Window, &mut App) -> AnyView + 'static;

const MENU_BAR_HEIGHT: f32 = 36.0;
const DEFAULT_VIEWPORT_WIDTH: f32 = 1280.0;
const DEFAULT_VIEWPORT_HEIGHT: f32 = 800.0;

/// Application-provided metadata and content factory for one panel kind.
///
/// DockWorkspace uses the key and title for registration, layout, and the
/// Window menu. The factory is invoked lazily when the active panel is rendered.
pub struct PanelDefinition {
    key: String,
    title: SharedString,
    icon: IconSource,
    constraints: PanelConstraints,
    factory: Box<PanelFactory>,
}

impl PanelDefinition {
    /// Creates a panel definition using default size constraints.
    ///
    /// Keys contain lowercase ASCII letters/digits separated by single
    /// hyphens, for example layers or asset-browser.
    pub fn new(
        key: impl Into<String>,
        title: impl Into<SharedString>,
        factory: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            icon: IconSource::None,
            constraints: PanelConstraints::new(
                Size::new(120.0, 80.0),
                Size::new(320.0, 280.0),
                None,
                false,
            ),
            factory: Box::new(factory),
        }
    }

    /// Sets an optional panel icon.
    pub fn with_icon(mut self, icon: IconSource) -> Self {
        self.icon = icon;
        self
    }

    /// Overrides the default panel size constraints.
    pub fn with_constraints(mut self, constraints: PanelConstraints) -> Self {
        self.constraints = constraints;
        self
    }
}

/// Errors raised while creating the high-level docking workspace.
#[derive(Debug, Error)]
pub enum DockWorkspaceError {
    #[error(transparent)]
    InvalidPanelKey(#[from] PanelKeyError),

    #[error(transparent)]
    PanelRegistry(#[from] PanelRegistryError),

    #[error(transparent)]
    InitialLayout(#[from] InitialLayoutError),

    #[error(transparent)]
    Controller(#[from] DockControllerConfigError),
}

/// One registered panel in the menu, ordered as supplied by the application.
#[derive(Clone)]
struct PanelMenuDefinition {
    key: PanelKey,
    title: SharedString,
}

/// Complete docking workspace used by a downstream GPUI application.
///
/// The application only constructs this value from panel definitions and
/// mounts it as a GPUI view. On first render the workspace creates the
/// controller entity and DockHost itself. DockHost delegates active panel
/// placement to ColumnView and FloatingWindowView, which use PanelHost.
///
/// The initial Photoshop layout requires the four built-in keys:
/// layers, color, properties, and history. Additional definitions are
/// registered automatically and can be opened from the Window menu.
pub struct DockWorkspace {
    pending_controller: Option<DockController>,
    controller: Option<Entity<DockController>>,
    host: Option<Entity<DockHost>>,
    panels: Vec<PanelMenuDefinition>,
    menu_open: bool,
}

impl DockWorkspace {
    /// Creates a complete workspace from the application's panel definitions.
    ///
    /// This constructor does not need a GPUI Context. The controller and host
    /// entities are created lazily on the first render.
    ///
    /// Invalid keys, duplicate IDs, missing baseline panels, or invalid
    /// constraints cause a descriptive panic. Use try_new when the application
    /// wants to handle configuration errors explicitly.
    pub fn new(panels: impl IntoIterator<Item = PanelDefinition>) -> Self {
        Self::try_new(panels).expect("invalid DockWorkspace panel configuration")
    }

    /// Fallible version of new.
    pub fn try_new(
        panels: impl IntoIterator<Item = PanelDefinition>,
    ) -> Result<Self, DockWorkspaceError> {
        let metrics = DockMetrics::default();
        let mut registry = PanelRegistry::new();
        let mut menu_panels = Vec::new();

        for panel in panels {
            let key = PanelKey::new(panel.key)?;
            let title = panel.title.clone();

            registry.register(PanelDescriptor::new(
                key.clone(),
                title.clone(),
                panel.icon,
                panel.constraints,
                panel.factory,
            ))?;

            menu_panels.push(PanelMenuDefinition { key, title });
        }

        let layout = crate::dock_core::create_initial_layout(
            Profile::Photoshop,
            &registry.descriptors_for_core(),
            metrics,
        )?;

        // DockHost measures its actual GPUI bounds on the first frame and
        // immediately replaces this small non-empty bootstrap viewport.
        let initial_viewport = Rect::new(Point::ZERO, Size::new(1.0, 1.0));
        let controller = DockController::new(layout, metrics, registry, initial_viewport)?;

        Ok(Self {
            pending_controller: Some(controller),
            controller: None,
            host: None,
            panels: menu_panels,
            menu_open: false,
        })
    }

    /// Returns the controller after the workspace has been mounted once.
    pub fn controller(&self) -> Option<&Entity<DockController>> {
        self.controller.as_ref()
    }

    fn initialize_entities(&mut self, cx: &mut Context<Self>) {
        if self.host.is_some() {
            return;
        }

        let controller = self
            .pending_controller
            .take()
            .expect("DockWorkspace controller is initialized only once");
        let controller = cx.new(|_| controller);
        let host = cx.new(|_| DockHost::new(controller.clone(), DockTheme::default()));

        self.controller = Some(controller);
        self.host = Some(host);
    }

    fn render_window_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + 'static {
        let mut root = div();

        if let Some(controller) = self.controller.as_ref().cloned() {
            let entries = self
                .panels
                .iter()
                .map(|panel| {
                    let selected = controller.update(cx, |controller, _| {
                        controller
                            .layout()
                            .panel_instances()
                            .any(|(_, instance)| instance.key == panel.key)
                    });
                    let key = panel.key.clone();
                    let controller = controller.clone();

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

            let workspace_entity = cx.entity().downgrade();
            let on_dismiss: Rc<dyn Fn(&mut App)> = Rc::new(move |app| {
                if let Some(workspace) = workspace_entity.upgrade() {
                    workspace.update(app, |workspace, workspace_cx| {
                        workspace.menu_open = false;
                        workspace_cx.notify();
                    });
                }
            });

            root = root.child(ContextMenu::render(
                "dock-workspace-window-menu",
                Rect::new(
                    Point::ZERO,
                    Size::new(DEFAULT_VIEWPORT_WIDTH, DEFAULT_VIEWPORT_HEIGHT),
                ),
                Point::new(92.0, MENU_BAR_HEIGHT),
                entries,
                None,
                ContextMenuCallbacks {
                    on_submenu: Rc::new(|_, _| {}),
                    on_dismiss,
                },
                DockTheme::default(),
            ));
        }

        root
    }
}

impl Render for DockWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.initialize_entities(cx);

        let menu_button = div()
            .id("dock-workspace-window-menu-button")
            .h(px(MENU_BAR_HEIGHT))
            .px(px(12.0))
            .flex()
            .items_center()
            .cursor_pointer()
            .text_color(rgb(0xE8E8E8))
            .hover(|style| style.bg(rgb(0x454545)))
            .child("Окно")
            .on_click(cx.listener(|workspace, _, _, cx| {
                workspace.menu_open = !workspace.menu_open;
                cx.notify();
            }));

        let menu_bar = div()
            .id("dock-workspace-menu-bar")
            .w_full()
            .h(px(MENU_BAR_HEIGHT))
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(6.0))
            .bg(rgb(0x303030))
            .text_color(rgb(0xE8E8E8))
            .child(div().px(px(12.0)).child("Файл"))
            .child(div().px(px(12.0)).child("Правка"))
            .child(menu_button);

        let host = self
            .host
            .as_ref()
            .expect("DockHost was initialized")
            .clone();

        let mut root = div()
            .id("dock-workspace-root")
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(0x252525))
            .child(menu_bar)
            .child(
                div()
                    .id("dock-workspace-content")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(host),
            );

        if self.menu_open {
            root = root.child(self.render_window_menu(cx));
        }

        root
    }
}

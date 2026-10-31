#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::rc::Rc;

use dock_gpui::{
    ContextMenu, ContextMenuCallbacks, ContextMenuEntry, ContextMenuItem, DockController,
    DockHost, DockMetrics, DockTheme, IconSource, PanelConstraints, PanelDescriptor,
    PanelKey, PanelRegistry, Point, Profile, Rect, Size,
};
use gpui::{
    App, AppContext, AnyView, Bounds, Context, Entity, IntoElement, Render, TitlebarOptions,
    Window, WindowBounds, WindowOptions, div, px, rgb, size,
};
use gpui::prelude::*;
use gpui_platform::application;

const APP_WIDTH: f32 = 1280.0;
const APP_HEIGHT: f32 = 800.0;
const MENU_BAR_HEIGHT: f32 = 36.0;
const WORKSPACE_HEIGHT: f32 = APP_HEIGHT - MENU_BAR_HEIGHT;

// These stable keys are the IDs the application gives to the docking library.
// Runtime PanelId values are created by dock-core when a panel instance opens.
const PANELS: [(&str, &str, u32); 4] = [
    ("layers", "Слои", 0x40566F),
    ("color", "Цвет", 0x65506F),
    ("properties", "Свойства", 0x526B5C),
    ("history", "История", 0x725C4B),
];

fn main() {
    application().run(|cx: &mut App| {
        let metrics = DockMetrics::default();
        let registry = create_panel_registry();
        let layout = dock_gpui::dock_core::create_initial_layout(
            Profile::Photoshop,
            &registry.descriptors_for_core(),
            metrics,
        )
        .expect("the app example must provide all four baseline panel descriptors");

        let viewport = Rect::new(
            Point::ZERO,
            Size::new(APP_WIDTH, WORKSPACE_HEIGHT),
        );
        let controller = cx.new(|_| {
            DockController::new(layout, metrics, registry, viewport)
                .expect("the app dock controller must be valid")
        });

        let bounds = Bounds::centered(None, size(px(APP_WIDTH), px(APP_HEIGHT)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Roadmap Docking — app integration".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        cx.open_window(options, move |_window, cx| {
            cx.new(|cx| AppView::new(controller, cx))
        })
        .expect("the app integration window must open");

        cx.activate(true);
    });
}

/// The application owns the descriptors and panel content. The library owns
/// runtime panel instances, layout, hit testing, and docking behavior.
fn create_panel_registry() -> PanelRegistry {
    let mut registry = PanelRegistry::new();
    let constraints = PanelConstraints::new(
        Size::new(120.0, 80.0),
        Size::new(320.0, 280.0),
        None,
        false,
    );

    for (id, title, color) in PANELS {
        let key = PanelKey::new(id).expect("the four app panel IDs are valid");
        let descriptor = PanelDescriptor::new(
            key,
            title,
            IconSource::None,
            constraints,
            move |_window, cx| {
                cx.new(|_| PanelPlaceholder { id, title, color }).into()
            },
        );
        registry
            .register(descriptor)
            .expect("app panel IDs must be unique");
    }

    registry
}

/// A deliberately simple application-provided panel view.
/// DockHost mounts it through the library's PanelHost component.
struct PanelPlaceholder {
    id: &'static str,
    title: &'static str,
    color: u32,
}

impl Render for PanelPlaceholder {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_center()
            .gap(px(8.0))
            .p(px(16.0))
            .bg(rgb(self.color))
            .text_color(rgb(0xFFFFFF))
            .child(self.title)
            .child(format!("Panel ID: {}", self.id))
            .child("Application panel placeholder")
    }
}

struct AppView {
    controller: Entity<DockController>,
    dock_host: Entity<DockHost>,
    menu_open: bool,
    controller_observed: bool,
}

impl AppView {
    fn new(controller: Entity<DockController>, cx: &mut Context<Self>) -> Self {
        let dock_host = cx.new(|_| DockHost::new(controller.clone(), DockTheme::default()));
        Self {
            controller,
            dock_host,
            menu_open: false,
            controller_observed: false,
        }
    }

    fn panel_menu_entries(&self, cx: &mut Context<Self>) -> Vec<ContextMenuEntry> {
        PANELS
            .iter()
            .map(|(id, title, _)| {
                let is_open = self.controller.update(cx, |controller, _| {
                    controller
                        .layout()
                        .panel_instances()
                        .any(|(_, instance)| instance.key.as_str() == *id)
                });
                let panel_key = PanelKey::new(id).expect("the app panel ID is valid");
                let controller = self.controller.clone();

                ContextMenuItem::new(*title, move |app| {
                    controller.update(app, |controller, controller_cx| {
                        if let Err(error) = controller.toggle_panel(&panel_key) {
                            eprintln!("Could not toggle panel: {error}");
                        }
                        controller.flush_events(controller_cx);
                        controller_cx.notify();
                    });
                })
                .selected(is_open)
                .into()
            })
            .collect()
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.controller_observed {
            cx.observe(&self.controller, |_, _, cx| cx.notify()).detach();
            self.controller_observed = true;
        }

        let menu_button = div()
            .id("window-menu-button")
            .h(px(MENU_BAR_HEIGHT))
            .px(px(12.0))
            .flex()
            .items_center()
            .cursor_pointer()
            .text_color(rgb(0xE8E8E8))
            .hover(|style| style.bg(rgb(0x454545)))
            .child("Окно")
            .on_click(cx.listener(|view, _, _, cx| {
                view.menu_open = !view.menu_open;
                cx.notify();
            }));

        let menu_bar = div()
            .id("application-menu-bar")
            .w_full()
            .h(px(MENU_BAR_HEIGHT))
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(6.0))
            .bg(rgb(0x303030))
            .text_color(rgb(0xE8E8E8))
            .child(
                div()
                    .px(px(12.0))
                    .child("Файл"),
            )
            .child(
                div()
                    .px(px(12.0))
                    .child("Правка"),
            )
            .child(menu_button);

        let mut root = div()
            .id("application-root")
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(0x252525))
            .child(menu_bar)
            .child(
                div()
                    .id("dock-workspace")
                    .flex_1()
                    .min_h_0()
                    .child(self.dock_host.clone()),
            );

        if self.menu_open {
            let entries = self.panel_menu_entries(cx);
            let app_view = cx.entity();
            let on_dismiss = Rc::new(move |app: &mut App| {
                app_view.update(app, |view, cx| {
                    view.menu_open = false;
                    cx.notify();
                });
            });

            root = root.child(ContextMenu::render(
                "application-window-menu",
                Rect::new(
                    Point::ZERO,
                    Size::new(APP_WIDTH, APP_HEIGHT),
                ),
                Point::new(92.0, MENU_BAR_HEIGHT),
                &entries,
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

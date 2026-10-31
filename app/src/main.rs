#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use dock_gpui::{DockWorkspace, PanelDefinition, Point, Rect, Size};
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

fn main() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(APP_WIDTH), px(APP_HEIGHT)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Roadmap Docking — app integration".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        cx.open_window(options, |_window, cx| cx.new(|cx| AppView::new(cx)))
            .expect("the app integration window must open");

        cx.activate(true);
    });
}

struct AppView {
    workspace: Entity<DockWorkspace>,
    menu_open: bool,
}

impl AppView {
    fn new(cx: &mut Context<Self>) -> Self {
        // Add another panel by adding one PanelDefinition::new(...) line here.
        // Its entry appears in the library-owned Window menu automatically.
        let panels = vec![
            PanelDefinition::new("layers", "Слои", layers_factory)
                .expect("the layers panel key is valid"),
            PanelDefinition::new("color", "Цвет", color_factory)
                .expect("the color panel key is valid"),
            PanelDefinition::new("properties", "Свойства", properties_factory)
                .expect("the properties panel key is valid"),
            PanelDefinition::new("history", "История", history_factory)
                .expect("the history panel key is valid"),
        ];

        let viewport = Rect::new(
            Point::ZERO,
            Size::new(APP_WIDTH, WORKSPACE_HEIGHT),
        );
        let workspace = cx.new(|cx| {
            DockWorkspace::new(panels, viewport, cx)
                .expect("the app must provide the four baseline panel definitions")
        });

        Self {
            workspace,
            menu_open: false,
        }
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .child(div().px(px(12.0)).child("Файл"))
            .child(div().px(px(12.0)).child("Правка"))
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
                    .child(self.workspace.clone()),
            );

        if self.menu_open {
            let app_view = cx.entity();
            let menu = self.workspace.update(cx, |workspace, workspace_cx| {
                workspace.panel_menu(
                    "application-window-menu",
                    Rect::new(
                        Point::ZERO,
                        Size::new(APP_WIDTH, APP_HEIGHT),
                    ),
                    Point::new(92.0, MENU_BAR_HEIGHT),
                    move |app| {
                        app_view.update(app, |view, cx| {
                            view.menu_open = false;
                            cx.notify();
                        });
                    },
                    workspace_cx,
                )
            });
            root = root.child(menu);
        }

        root
    }
}

// These factories are application-owned. Replace their placeholder views with
// real panel implementations without changing DockWorkspace or dock-gpui.
fn layers_factory(_window: &mut Window, cx: &mut App) -> AnyView {
    panel_placeholder(cx, "layers", "Слои", 0x40566F)
}

fn color_factory(_window: &mut Window, cx: &mut App) -> AnyView {
    panel_placeholder(cx, "color", "Цвет", 0x65506F)
}

fn properties_factory(_window: &mut Window, cx: &mut App) -> AnyView {
    panel_placeholder(cx, "properties", "Свойства", 0x526B5C)
}

fn history_factory(_window: &mut Window, cx: &mut App) -> AnyView {
    panel_placeholder(cx, "history", "История", 0x725C4B)
}

fn panel_placeholder(cx: &mut App, id: &'static str, title: &'static str, color: u32) -> AnyView {
    cx.new(|_| PanelPlaceholder { id, title, color }).into()
}

/// Deliberately simple application-provided panel content.
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

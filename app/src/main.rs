#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use dock_gpui::{DockWorkspace, PanelDefinition};
use gpui::{
    App, AppContext, AnyView, Bounds, Context, Entity, IntoElement, Render, TitlebarOptions,
    Window, WindowBounds, WindowOptions, div, px, size,
};
use gpui::prelude::*;
use gpui_platform::application;

const APP_WIDTH: f32 = 1280.0;
const APP_HEIGHT: f32 = 800.0;

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
    dock: Entity<DockWorkspace>,
}

impl AppView {
    fn new(cx: &mut Context<Self>) -> Self {
        let panels = [
            PanelDefinition::new("layers", "Слои", layers_factory),
            PanelDefinition::new("color", "Цвет", color_factory),
            PanelDefinition::new("properties", "Свойства", properties_factory),
            PanelDefinition::new("history", "История", history_factory),
        ];

        let dock = DockWorkspace::new(panels);
        let dock = cx.new(|_| dock);

        Self { dock }
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.dock.clone())
    }
}

// These application factories intentionally return empty panel content.
// DockHost places the resulting views through the library's PanelHost.
fn layers_factory(_window: &mut Window, cx: &mut App) -> AnyView {
    empty_panel_factory(cx)
}

fn color_factory(_window: &mut Window, cx: &mut App) -> AnyView {
    empty_panel_factory(cx)
}

fn properties_factory(_window: &mut Window, cx: &mut App) -> AnyView {
    empty_panel_factory(cx)
}

fn history_factory(_window: &mut Window, cx: &mut App) -> AnyView {
    empty_panel_factory(cx)
}

fn empty_panel_factory(cx: &mut App) -> AnyView {
    cx.new(|_| EmptyPanel).into()
}

struct EmptyPanel;

impl Render for EmptyPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

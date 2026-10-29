use dock_core::PanelKey;
use gpui::{
    App, Bounds, Context, IntoElement, Render, TitlebarOptions, Window, WindowBounds, WindowKind,
    WindowOptions, div, px, rgb, size,
};
use gpui::prelude::*;

pub(crate) struct MainWindow;

#[derive(Clone, Copy)]
enum PanelKind {
    Layers,
    Properties,
}

impl PanelKind {
    const fn key(self) -> &'static str {
        match self {
            Self::Layers => "layers",
            Self::Properties => "properties",
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::Layers => "Слои",
            Self::Properties => "Свойства",
        }
    }
}

fn menu_button(kind: PanelKind) -> impl IntoElement {
    div()
        .id(kind.key())
        .px_3()
        .py_2()
        .bg(rgb(0x394451))
        .text_color(rgb(0xf0f2f5))
        .rounded_sm()
        .cursor_pointer()
        .child(kind.title())
        .on_click(move |_, _, cx| open_floating_panel(kind, cx))
}

fn open_floating_panel(kind: PanelKind, cx: &mut App) {
    // These keys are static, lowercase identifiers accepted by dock-core.
    let key = match PanelKey::new(kind.key()) {
        Ok(key) => key,
        Err(error) => {
            eprintln!("Invalid built-in panel key: {error}");
            return;
        }
    };

    let bounds = WindowBounds::Windowed(Bounds::centered(
        None,
        size(px(340.0), px(460.0)),
        cx,
    ));
    let options = WindowOptions {
        window_bounds: Some(bounds),
        titlebar: Some(TitlebarOptions {
            title: Some(kind.title().into()),
            ..Default::default()
        }),
        kind: WindowKind::Floating,
        ..Default::default()
    };

    if let Err(error) = cx.open_window(options, move |_window, cx| {
        cx.new(|_| FloatingPanel { key })
    }) {
        eprintln!("Could not open floating panel '{}': {error}", kind.key());
    }
}

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x20252b))
            .text_color(rgb(0xe8ebef))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .h(px(48.0))
                    .px_3()
                    .bg(rgb(0x2b323a))
                    .border_b_1()
                    .border_color(rgb(0x414a55))
                    .child("Панели")
                    .child(menu_button(PanelKind::Layers))
                    .child(menu_button(PanelKind::Properties)),
            )
            .child(div().flex_1().w_full().bg(rgb(0x20252b)))
    }
}

struct FloatingPanel {
    key: PanelKey,
}

impl Render for FloatingPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x292f37))
            .text_color(rgb(0xe8ebef))
            .p_4()
            .child(self.key.as_str().to_owned())
    }
}

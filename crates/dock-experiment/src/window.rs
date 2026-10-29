use dock_core::PanelKey;
use gpui::{
    AppContext, Bounds, Context, IntoElement, Render, TitlebarOptions, WeakEntity, Window,
    WindowBounds, WindowHandle, WindowKind, WindowOptions, div, px, rgb, size,
};
use gpui::prelude::*;

pub(crate) struct MainWindow {
    floating_panels: Vec<WindowHandle<FloatingPanel>>,
}

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

impl MainWindow {
    pub(crate) fn new() -> Self {
        Self {
            floating_panels: Vec::new(),
        }
    }

    fn open_floating_panel(&mut self, kind: PanelKind, cx: &mut Context<Self>) {
        let key = match PanelKey::new(kind.key()) {
            Ok(key) => key,
            Err(error) => {
                eprintln!("Invalid built-in panel key: {error}");
                return;
            }
        };

        let bounds = WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(280.0), px(360.0)),
            cx,
        ));
        let options = WindowOptions {
            window_bounds: Some(bounds),
            titlebar: Some(TitlebarOptions {
                title: Some(kind.title().into()),
                ..Default::default()
            }),
            kind: WindowKind::PopUp,
            ..Default::default()
        };

        match cx.open_window(options, move |_window, cx| {
            cx.new(|_| FloatingPanel { key })
        }) {
            Ok(handle) => self.floating_panels.push(handle),
            Err(error) => {
                eprintln!("Could not open floating panel '{}': {error}", kind.key());
            }
        }
    }

    /// Closes every still-open panel before GPUI closes the main window.
    pub(crate) fn close_floating_panels(&mut self, cx: &mut Context<Self>) {
        for panel in self.floating_panels.drain(..) {
            // A panel may already have been closed directly by the user.
            if let Err(_error) = panel.update(cx, |_, window, _| window.remove_window()) {}
        }
    }
}

fn menu_button(kind: PanelKind, owner: WeakEntity<MainWindow>) -> impl IntoElement {
    div()
        .id(kind.key())
        .px_3()
        .py_2()
        .bg(rgb(0x394451))
        .text_color(rgb(0xf0f2f5))
        .rounded_sm()
        .cursor_pointer()
        .child(kind.title())
        .on_click(move |_, _, cx| {
            if let Err(error) =
                owner.update(cx, |this, cx| this.open_floating_panel(kind, cx))
            {
                eprintln!("Could not access the main window to open a panel: {error:?}");
            }
        })
}

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity().downgrade();

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
                    .child(menu_button(PanelKind::Layers, owner.clone()))
                    .child(menu_button(PanelKind::Properties, owner)),
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

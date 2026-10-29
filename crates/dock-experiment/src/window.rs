use dock_core::PanelKey;
use gpui::{
    App, Bounds, Context, IntoElement, Render, TitlebarOptions, Window, WindowBounds, WindowKind,
    WindowOptions, div, px, rgb, size,
};
use gpui::prelude::*;

#[cfg(windows)]
use platform::windows::{self, NativeWindowHandle};

#[cfg(windows)]
type OwnerWindow = Option<NativeWindowHandle>;

#[cfg(not(windows))]
#[derive(Clone, Copy)]
struct OwnerWindow;

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

#[cfg(windows)]
fn main_window_owner(window: &Window) -> OwnerWindow {
    NativeWindowHandle::from_window(window)
}

#[cfg(not(windows))]
fn main_window_owner(_window: &Window) -> OwnerWindow {
    OwnerWindow
}

fn menu_button(kind: PanelKind, owner: OwnerWindow) -> impl IntoElement {
    div()
        .id(kind.key())
        .px_3()
        .py_2()
        .bg(rgb(0x394451))
        .text_color(rgb(0xf0f2f5))
        .rounded_sm()
        .cursor_pointer()
        .child(kind.title())
        .on_click(move |_, _, cx| open_floating_panel(kind, owner, cx))
}

fn open_floating_panel(kind: PanelKind, _owner: OwnerWindow, cx: &mut App) {
    // Refuse to create an unowned HWND: it would outlive the main window.
    #[cfg(windows)]
    let Some(owner) = _owner else {
        eprintln!("Could not get the main window's native handle; panel was not opened");
        return;
    };

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
        // On Windows, keep it hidden until the native owner relationship is set.
        show: !cfg!(windows),
        ..Default::default()
    };

    let handle = match cx.open_window(options, move |_window, cx| {
        cx.new(|_| FloatingPanel { key })
    }) {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("Could not open floating panel '{}': {error}", kind.key());
            return;
        }
    };

    #[cfg(not(windows))]
    let _ = handle;

    #[cfg(windows)]
    {
        let native_handle = handle.update(cx, |_, floating_window, _| {
            NativeWindowHandle::from_window(floating_window)
        });

        match native_handle {
            Ok(Some(floating)) => {
                windows::set_owner(floating, owner);
                // Showing only after SetWindowLongPtrW ensures the popup never
                // becomes an independent top-level window without an owner.
                windows::show_window(floating);
            }
            Ok(None) => {
                eprintln!("Could not get the floating panel's native handle");
                if let Err(error) = handle.update(cx, |_, window, _| window.remove_window()) {
                    eprintln!("Could not close unowned floating panel: {error:?}");
                }
            }
            Err(error) => {
                eprintln!("Could not access floating panel window: {error:?}");
            }
        }
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let owner = main_window_owner(window);

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
                    .child(menu_button(PanelKind::Layers, owner))
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

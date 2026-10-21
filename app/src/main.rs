mod panels;

#[cfg(windows)]
fn main() {
    use gpui::{
        App, AppContext, Bounds, Context, IntoElement, Render, Window, WindowBounds, WindowOptions,
        div, px, rgb, size,
    };
    use gpui_platform::application;
    use ui::Theme;

    struct StartupWindow {
        // Сохраняем тему для дальнейшего построения интерфейса.
        _theme: Theme,
    }

    impl Render for StartupWindow {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            use gpui::ParentElement;
            use gpui::Styled;

            div()
                .max_size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(0x252526))
                .text_color(rgb(0xe0e0e0))
                .child("Docking — Core")
        }
    }

    application().run(|cx: &mut App| {
        let theme = Theme::default();

        let bounds = Bounds::centered(None, size(px(1000.0), px(700.0)), cx);

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };

        if let Err(error) = cx.open_window(window_options, move |_window, cx| {
            cx.new(|_| StartupWindow { _theme: theme })
        }) {
            eprintln!("Could not open the application window: {error}");
            return;
        }

        cx.activate(true);
    });
}

#[cfg(not(windows))]
fn main() {
    eprintln!("This application currently targets Windows 10 x64 (MSVC).");
}

mod panels;

#[cfg(windows)]
mod startup_window;

#[cfg(windows)]
fn main() {
    use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};
    use gpui_platform::application;
    use startup_window::StartupWindow;

    application().run(|cx: &mut App| {
        let theme = ui::Theme::default();

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

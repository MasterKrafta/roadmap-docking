#[cfg(windows)]
mod main_window;

#[cfg(windows)]
fn main() {
    use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};
    use gpui_platform::application;
    use main_window::MainWindow;

    application().run(|cx: &mut App| {
        let theme = ui::Theme::default();
        let main_window = match MainWindow::new(theme) {
            Ok(window) => window,
            Err(error) => {
                eprintln!("Could not initialize the docking layout: {error}");
                return;
            }
        };

        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };

        if let Err(error) = cx.open_window(window_options, move |_window, cx| {
            cx.new(|_| main_window)
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

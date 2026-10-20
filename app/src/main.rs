mod dock_metrics;
mod panels;

#[cfg(windows)]
fn main() {
    let _panel_registry = panels::PanelRegistry::with_builtins()
        .expect("built-in panel descriptors must be valid");

    use dock_gpui::PrototypeWindow;
    use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};
    use gpui_platform::application;
    use ui::Theme;

    application().run(|cx: &mut App| {
        let theme = Theme::default();
        let _metrics = dock_metrics::build(&theme);

        // Создаем границы окна: по центру экрана с размером 800x600
        let bounds = Bounds::centered(None, size(px(1000.), px(700.)), cx);
        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };

        let result = cx.open_window(window_options, move |_window, cx| {
            cx.new(|_| PrototypeWindow::new(theme))
        });
        if let Err(error) = result {
            eprintln!("Could not open the GPUI prototype window: {error}");
            return;
        }
        cx.activate(true);
    });
}

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "This workspace's first runnable window targets Windows 10 x64 (MSVC). Use a Windows toolchain to run `cargo run -p app`."
    );
}

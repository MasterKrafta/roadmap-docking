mod dock_metrics;
mod main_window;
mod panels;

#[cfg(windows)]
fn main() {
    use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};
    use gpui_platform::application;
    use ui::Theme;

    application().run(|cx: &mut App| {
        // One app-wide Theme feeds metrics, geometry and every visible component.
        let theme = Theme::default();
        let metrics = dock_metrics::build(&theme);
        let registry = match panels::PanelRegistry::with_builtins() {
            Ok(registry) => registry,
            Err(error) => {
                eprintln!("Could not initialize panel registry: {error}");
                return;
            }
        };
        let layout = match registry.build_initial_photoshop_layout(&metrics) {
            Ok(layout) => layout,
            Err(error) => {
                eprintln!("Could not build initial dock layout: {error}");
                return;
            }
        };

        // This opens exactly one OS window. Floating groups remain an in-window concept.
        let bounds = Bounds::centered(None, size(px(1000.), px(700.)), cx);
        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };

        let result = cx.open_window(window_options, move |_window, cx| {
            cx.new(move |_| main_window::MainWindow::new(theme, metrics, registry, layout))
        });
        if let Err(error) = result {
            eprintln!("Could not open the main application window: {error}");
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

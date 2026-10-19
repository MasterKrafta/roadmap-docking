mod dock_metrics;

#[cfg(windows)]
fn main() {
    use dock_gpui::PrototypeWindow;
    use gpui::{App, AppContext, WindowOptions};
    use gpui_platform::application;
    use ui::Theme;

    application().run(|cx: &mut App| {
        let theme = Theme::default();
        let _metrics = dock_metrics::build(&theme);
        let result = cx.open_window(WindowOptions::default(), move |_window, cx| {
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

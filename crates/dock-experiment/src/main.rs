#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod window;

#[cfg(windows)]
fn main() {
    use gpui::{
        App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
    };
    use gpui_platform::application;

    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Docking experiment".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        if let Err(error) = cx.open_window(options, |_window, cx| {
            cx.new(|_| window::MainWindow)
        }) {
            eprintln!("Could not open the experiment window: {error}");
            return;
        }

        cx.activate(true);
    });
}

#[cfg(not(windows))]
fn main() {
    eprintln!("The docking experiment currently targets Windows.");
}

#[derive(Clone, Debug)]
pub struct RunOptions {
    /// Native window title.
    pub title: String,
    /// Initial client-area width in logical pixels.
    pub width: f32,
    /// Initial client-area height in logical pixels.
    pub height: f32,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            title: "Dock".to_owned(),
            width: 1200.0,
            height: 800.0,
        }
    }
}

/// Starts the platform window and renders the temporary no-op dock host.
pub fn run(options: RunOptions) {
    #[cfg(windows)]
    run_windows(options);

    #[cfg(not(windows))]
    {
        let _ = options;
        eprintln!("The dock application currently targets Windows 10 x64 (MSVC).");
    }
}

#[cfg(windows)]
fn run_windows(options: RunOptions) {
    use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
    use gpui_platform::application;

    application().run(|cx: &mut App| {
        let theme = ui::Theme::default();
        let main_window = match crate::main_window::MainWindow::new(theme) {
            Ok(window) => window,
            Err(error) => {
                eprintln!("Could not initialize the dock window: {error}");
                return;
            }
        };

        let bounds = Bounds::centered(
            None,
            size(px(options.width.max(1.0)), px(options.height.max(1.0))),
            cx,
        );
        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(options.title.into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        if let Err(error) = cx.open_window(window_options, move |_window, cx| {
            cx.new(|_| main_window)
        }) {
            eprintln!("Could not open the dock window: {error}");
            return;
        }

        cx.activate(true);
    });
}

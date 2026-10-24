use std::path::PathBuf;

use gpui::AssetSource;

/// Options for starting the dock application.
pub struct RunOptions {
    /// Native window title.
    pub title: String,
    /// Initial client-area width in logical pixels.
    pub width: f32,
    /// Initial client-area height in logical pixels.
    pub height: f32,
    /// Optional source for application assets (icons, fonts and other packaged resources).
    pub assets: Option<Box<dyn AssetSource>>,
    /// Reserved for workspace persistence, wired in Stage 15.
    pub workspace_path: Option<PathBuf>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            title: "Dock".to_owned(),
            width: 1200.0,
            height: 800.0,
            assets: None,
            workspace_path: None,
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
struct BoxedAssetSource(Box<dyn AssetSource>);

#[cfg(windows)]
impl AssetSource for BoxedAssetSource {
    fn load(
        &self,
        path: &str,
    ) -> gpui::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        self.0.load(path)
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<gpui::SharedString>> {
        self.0.list(path)
    }
}

#[cfg(windows)]
fn run_windows(options: RunOptions) {
    use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
    use gpui_platform::application;

    let RunOptions {
        title,
        width,
        height,
        assets,
        workspace_path: _,
    } = options;

    let run_application = move |cx: &mut App| {
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
            size(px(width.max(1.0)), px(height.max(1.0))),
            cx,
        );
        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(title.into()),
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
    };

    let application = application();
    match assets {
        Some(assets) => application.with_assets(BoxedAssetSource(assets)).run(run_application),
        None => application.run(run_application),
    }
}

//! Independent prototype of the next docking model and GPUI view.
//!
//! This module deliberately does not use `dock-core`. It is developed alongside
//! the existing implementation so its model and behavior can be evaluated separately.

mod model;
mod view;
mod window;

#[cfg(windows)]
use gpui::AssetSource;

use crate::RunOptions;

/// Starts the independent docking prototype.
pub fn run(options: RunOptions) {
    #[cfg(windows)]
    run_windows(options);

    #[cfg(not(windows))]
    {
        let _ = options;
        tracing::error!("The dock application currently targets Windows 10 x64 (MSVC).");
    }
}

#[cfg(windows)]
struct BoxedAssetSource(Box<dyn AssetSource>);

#[cfg(windows)]
impl AssetSource for BoxedAssetSource {
    fn load(&self, path: &str) -> gpui::Result<Option<std::borrow::Cow<'static, [u8]>>> {
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
        let bounds = Bounds::centered(None, size(px(width.max(1.0)), px(height.max(1.0))), cx);
        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(title.into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        if let Err(error) = cx.open_window(window_options, move |_window, cx| {
            cx.new(|_| window::NextDockWindow::new(theme))
        }) {
            tracing::error!(error = %error, "could not open the next dock window");
            return;
        }

        cx.activate(true);
    };

    let application = application();
    match assets {
        Some(assets) => application
            .with_assets(BoxedAssetSource(assets))
            .run(run_application),
        None => application.run(run_application),
    }
}

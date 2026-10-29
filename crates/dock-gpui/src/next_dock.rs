//! Independent prototype of the next docking model and GPUI view.
//!
//! This module deliberately does not use `dock-core`. It is developed alongside
//! the existing implementation so its model and behavior can be evaluated separately.

mod components;
mod geometry;
mod model;
mod view;
mod window;

#[cfg(windows)]
mod floating_window;
#[cfg(windows)]
mod native_floating;

#[cfg(windows)]
use gpui::AssetSource;
#[cfg(windows)]
use std::time::Instant;

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
        let startup_started = Instant::now();
        eprintln!("[timing] старт next_dock: {:?}", startup_started.elapsed());
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

        let main_startup_started = startup_started;
        let main_view = cx.new(|_| window::NextDockWindow::new(theme, main_startup_started));

        // Подготавливаем скрытое floating-окно до создания системного главного окна.
        native_floating::prepare_hidden_floating_window(main_view.downgrade(), cx);

        eprintln!("[timing] вызов open_window главного: {:?}", startup_started.elapsed());
        let open_window_result = cx.open_window(window_options, move |_window, _cx| main_view);
        eprintln!("[timing] главное окно создано: {:?}", startup_started.elapsed());

        if let Err(error) = open_window_result {
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

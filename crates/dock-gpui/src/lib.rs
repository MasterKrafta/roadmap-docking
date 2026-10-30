//! GPUI docking implementation.
//!
//! The crate exposes one implementation: the workspace and its native floating
//! compositions are managed by the `next_dock` module tree.

pub mod components;
mod next_dock;

use gpui::AssetSource;

/// Options used to start the dock application.
pub struct RunOptions {
    /// Native window title.
    pub title: String,
    /// Initial client-area width in logical pixels.
    pub width: f32,
    /// Initial client-area height in logical pixels.
    pub height: f32,
    /// Optional source for application assets (icons, fonts and other packaged resources).
    pub assets: Option<Box<dyn AssetSource>>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            title: "Dock".to_owned(),
            width: 1200.0,
            height: 800.0,
            assets: None,
        }
    }
}

pub use next_dock::run;

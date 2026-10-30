//! Clean starting point for the GPUI docking adapter.
//!
//! The previous dock-gpui implementation was removed to rebuild the adapter
//! around dock-core without keeping parallel state or rendering paths.

use std::path::PathBuf;

/// Options for starting the dock application.
#[derive(Default)]
pub struct RunOptions {
    /// Optional workspace file to restore at startup.
    pub workspace_path: Option<PathBuf>,
}

/// Temporary entry point retained so the application crate keeps compiling.
///
/// The actual GPUI window and docking host will be implemented in the new adapter.
pub fn run(_options: RunOptions) {
    tracing::warn!("dock-gpui is an empty scaffold; the application window is not implemented yet");
}

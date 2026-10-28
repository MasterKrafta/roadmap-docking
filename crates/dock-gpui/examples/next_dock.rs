//! Launches the independent dock prototype used by the application.
//!
//! Run on Windows with: `cargo run -p dock-gpui --example next_dock`

fn main() {
    dock_gpui::run_next_dock(dock_gpui::RunOptions::default());
}

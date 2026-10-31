#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    dock_gpui::run(dock_gpui::RunOptions::default());
}

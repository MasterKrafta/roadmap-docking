#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    let options = dock_gpui::RunOptions {
        // Start with a clean canvas instead of restoring the previous demo panels.
        // Workspace persistence can be re-enabled once the real panel setup is ready.
        workspace_path: None,
        ..Default::default()
    };
    dock_gpui::run(options);
}

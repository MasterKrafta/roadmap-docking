use dock_core::{Profile, WorkspaceFile, WorkspaceLoadOptions, create_initial_layout};
use dock_gpui::{dock_metrics_from_theme, panels};
use ui::Theme;

#[test]
fn registered_panel_layout_round_trips_through_workspace_json() {
    let theme = Theme::default();
    let descriptors = panels::descriptors(&theme);
    let metrics = dock_metrics_from_theme(&theme);
    let original_layout = create_initial_layout(Profile::Photoshop, &descriptors, metrics)
        .expect("registered panels should produce a valid baseline layout");

    let original_workspace = WorkspaceFile::from_layout(&original_layout, &metrics)
        .expect("the baseline layout should export to a workspace");
    let json = original_workspace
        .to_json_pretty()
        .expect("the workspace should serialize as JSON");

    let loaded = WorkspaceFile::load_json(
        &json,
        &descriptors,
        &metrics,
        WorkspaceLoadOptions::default(),
    )
    .expect("the serialized workspace should load");

    assert!(loaded.diagnostics.is_empty());
    assert_eq!(loaded.layout.validate(&metrics), Ok(()));
    assert_eq!(loaded.layout.profile(), original_layout.profile());
    assert_eq!(loaded.layout.chrome(), original_layout.chrome());
    assert_eq!(
        loaded.layout.panel_instances().count(),
        original_layout.panel_instances().count()
    );

    let restored_workspace = WorkspaceFile::from_layout(&loaded.layout, &metrics)
        .expect("the restored layout should still export to a workspace");
    assert_eq!(restored_workspace, original_workspace);
}

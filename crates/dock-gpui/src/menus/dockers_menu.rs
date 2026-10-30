use dock_core::Layout;
use ui::Theme;

use crate::{dock::toggle_panel, panels::PANELS};

use super::MenuItemModel;

pub(super) fn items(layout: &Layout, theme: &Theme) -> Vec<MenuItemModel> {
    PANELS
        .iter()
        .map(|definition| {
            let checked = layout
                .panel_instances()
                .any(|(_, instance)| instance.key.as_str() == definition.key);
            let command = toggle_panel(layout, definition.key, theme);
            MenuItemModel {
                id: format!("dockers-{}", definition.key),
                label: definition.title.to_string(),
                checked,
                enabled: command.is_some(),
                command,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::items;
    use crate::dock::DockController;
    use ui::Theme;

    #[test]
    fn dockers_menu_follows_registry_order_and_marks_open_panels() {
        let theme = Theme::default();
        let controller = DockController::new_initial(&theme).unwrap();
        let items = items(controller.layout(), &theme);
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "dockers-layers",
                "dockers-color",
                "dockers-properties",
                "dockers-history",
            ]
        );
        assert!(items.iter().all(|item| item.checked && item.enabled));
    }
}

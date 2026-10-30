use dock_core::ColumnMode;

/// Current reference-window UI language is Russian; technical diagnostics may remain English.
pub(crate) const PANEL_MENU_TRIGGER_LABEL: &str = "Панели ▾";
pub(crate) const UNKNOWN_PANEL_TITLE: &str = "Неизвестная панель";
pub(crate) const FLOATING_GROUP_TITLE: &str = "Плавающая группа";
pub(crate) const DOCK_GEOMETRY_ERROR_PREFIX: &str =
    "Не удалось рассчитать геометрию рабочей области";

/// Produces the visible label for a panel in a rail column.
pub(crate) fn rail_entry_label(mode: ColumnMode, icon: &str, title: &str) -> String {
    match mode {
        ColumnMode::IconsOnly => icon.to_owned(),
        ColumnMode::IconsAndLabels => format!("{icon}  {title}"),
        ColumnMode::Expanded => title.to_owned(),
    }
}

/// Adds the collapsed-stack marker to the first tab label only.
pub(crate) fn tab_header_label(title: &str, collapsed: bool, tab_index: usize) -> String {
    if collapsed && tab_index == 0 {
        format!("▸ {title}")
    } else {
        title.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DOCK_GEOMETRY_ERROR_PREFIX, FLOATING_GROUP_TITLE, PANEL_MENU_TRIGGER_LABEL,
        UNKNOWN_PANEL_TITLE, rail_entry_label, tab_header_label,
    };
    use dock_core::ColumnMode;

    #[test]
    fn built_in_reference_window_labels_are_russian() {
        assert_eq!(PANEL_MENU_TRIGGER_LABEL, "Панели ▾");
        assert_eq!(UNKNOWN_PANEL_TITLE, "Неизвестная панель");
        assert_eq!(FLOATING_GROUP_TITLE, "Плавающая группа");
        assert_eq!(
            DOCK_GEOMETRY_ERROR_PREFIX,
            "Не удалось рассчитать геометрию рабочей области"
        );
    }

    #[test]
    fn rail_modes_have_distinct_icon_and_label_presentations() {
        assert_eq!(rail_entry_label(ColumnMode::IconsOnly, "▱", "Layers"), "▱");
        assert_eq!(
            rail_entry_label(ColumnMode::IconsAndLabels, "▱", "Layers"),
            "▱  Layers"
        );
        assert_eq!(
            rail_entry_label(ColumnMode::Expanded, "▱", "Layers"),
            "Layers"
        );
    }

    #[test]
    fn collapsed_stack_marker_appears_only_on_the_first_tab() {
        assert_eq!(tab_header_label("Layers", true, 0), "▸ Layers");
        assert_eq!(tab_header_label("Color", true, 1), "Color");
        assert_eq!(tab_header_label("Layers", false, 0), "Layers");
    }
}

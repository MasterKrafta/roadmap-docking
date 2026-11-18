use gpui::SharedString;

use super::context_menu_item::ContextMenuItem;

/// One row, separator, or first-level submenu in a context menu.
#[derive(Clone)]
pub enum ContextMenuEntry {
    Item(ContextMenuItem),
    Separator,
    Submenu {
        label: SharedString,
        hotkey: Option<SharedString>,
        disabled: bool,
        entries: Vec<ContextMenuEntry>,
    },
}

impl From<ContextMenuItem> for ContextMenuEntry {
    fn from(item: ContextMenuItem) -> Self {
        Self::Item(item)
    }
}

impl ContextMenuEntry {
    /// Adds a horizontal separator.
    pub fn separator() -> Self {
        Self::Separator
    }

    /// Adds a submenu. Submenus are intentionally limited to one level.
    pub fn submenu(
        label: impl Into<SharedString>,
        hotkey: Option<SharedString>,
        entries: Vec<ContextMenuEntry>,
    ) -> Self {
        Self::Submenu {
            label: label.into(),
            hotkey,
            disabled: false,
            entries,
        }
    }

    /// Adds a visible but unavailable submenu.
    pub fn disabled_submenu(label: impl Into<SharedString>) -> Self {
        Self::Submenu {
            label: label.into(),
            hotkey: None,
            disabled: true,
            entries: Vec::new(),
        }
    }
}

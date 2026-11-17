use std::rc::Rc;

use gpui::{App, SharedString};

type ContextMenuAction = Rc<dyn Fn(&mut App)>;

/// A reusable action row in a context menu.
#[derive(Clone)]
pub struct ContextMenuItem {
    label: SharedString,
    hotkey: Option<SharedString>,
    icon: Option<SharedString>,
    disabled: bool,
    selected: bool,
    action: Option<ContextMenuAction>,
}

impl ContextMenuItem {
    /// Creates an enabled action item.
    pub fn new(label: impl Into<SharedString>, action: impl Fn(&mut App) + 'static) -> Self {
        Self {
            label: label.into(),
            hotkey: None,
            icon: None,
            disabled: false,
            selected: false,
            action: Some(Rc::new(action)),
        }
    }

    /// Creates an item that is visible but cannot be activated.
    pub fn disabled(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            hotkey: None,
            icon: None,
            disabled: true,
            selected: false,
            action: None,
        }
    }

    /// Adds a right-aligned keyboard shortcut caption.
    pub fn with_hotkey(mut self, hotkey: impl Into<SharedString>) -> Self {
        self.hotkey = Some(hotkey.into());
        self
    }

    /// Adds a small image icon before the label.
    pub fn with_icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Marks the row as the currently selected option.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
}

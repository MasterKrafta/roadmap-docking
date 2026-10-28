use dock_core::{Axis, ResizeEdge};
use gpui::{Keystroke, StatefulInteractiveElement, Styled, rgb};
use ui::Theme;

/// Direction requested when moving a focused group within its owning column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GroupMoveDirection {
    Previous,
    Next,
}

/// Recognizes the platform command modifier + Shift + Up/Down group-move shortcut.
pub(crate) fn keyboard_group_move_direction(
    keystroke: &Keystroke,
) -> Option<GroupMoveDirection> {
    let modifiers = keystroke.modifiers;
    if !(modifiers.control || modifiers.platform)
        || !modifiers.shift
        || modifiers.alt
        || modifiers.function
    {
        return None;
    }

    match keystroke.key.to_ascii_lowercase().as_str() {
        "up" | "arrowup" => Some(GroupMoveDirection::Previous),
        "down" | "arrowdown" => Some(GroupMoveDirection::Next),
        _ => None,
    }
}

/// Direction requested when moving a focused panel into an adjacent group.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PanelMoveDirection {
    Previous,
    Next,
}

/// Recognizes command/control + Alt + Shift + Up/Down for moving a panel between groups.
pub(crate) fn keyboard_panel_move_direction(
    keystroke: &Keystroke,
) -> Option<PanelMoveDirection> {
    let modifiers = keystroke.modifiers;
    if !(modifiers.control || modifiers.platform)
        || !modifiers.alt
        || !modifiers.shift
        || modifiers.function
    {
        return None;
    }

    match keystroke.key.to_ascii_lowercase().as_str() {
        "up" | "arrowup" => Some(PanelMoveDirection::Previous),
        "down" | "arrowdown" => Some(PanelMoveDirection::Next),
        _ => None,
    }
}

/// Direction requested when moving a focused panel to an adjacent main-dock column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PanelColumnMoveDirection {
    Previous,
    Next,
}

/// Recognizes command/control + Alt + Shift + Left/Right for moving a panel between columns.
pub(crate) fn keyboard_panel_column_move_direction(
    keystroke: &Keystroke,
) -> Option<PanelColumnMoveDirection> {
    let modifiers = keystroke.modifiers;
    if !(modifiers.control || modifiers.platform)
        || !modifiers.alt
        || !modifiers.shift
        || modifiers.function
    {
        return None;
    }

    match keystroke.key.to_ascii_lowercase().as_str() {
        "left" | "arrowleft" => Some(PanelColumnMoveDirection::Previous),
        "right" | "arrowright" => Some(PanelColumnMoveDirection::Next),
        _ => None,
    }
}

/// Direction requested when reordering a focused tab with the keyboard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TabReorderDirection {
    Previous,
    Next,
}

/// Recognizes the platform command modifier + Shift + Left/Right tab-reorder shortcut.
pub(crate) fn keyboard_tab_reorder_direction(
    keystroke: &Keystroke,
) -> Option<TabReorderDirection> {
    let modifiers = keystroke.modifiers;
    if !(modifiers.control || modifiers.platform)
        || !modifiers.shift
        || modifiers.alt
        || modifiers.function
    {
        return None;
    }

    match keystroke.key.to_ascii_lowercase().as_str() {
        "left" | "arrowleft" => Some(TabReorderDirection::Previous),
        "right" | "arrowright" => Some(TabReorderDirection::Next),
        _ => None,
    }
}

/// Selects the resize axis for a floating edge from the focused arrow key.
pub(crate) fn keyboard_resize_axis_for_floating_edge(
    keystroke: &Keystroke,
    edge: ResizeEdge,
) -> Option<Axis> {
    match edge {
        ResizeEdge::Left | ResizeEdge::Right => {
            keyboard_resize_delta(keystroke, Axis::X).map(|_| Axis::X)
        }
        ResizeEdge::Top | ResizeEdge::Bottom => {
            keyboard_resize_delta(keystroke, Axis::Y).map(|_| Axis::Y)
        }
        ResizeEdge::TopLeft
        | ResizeEdge::TopRight
        | ResizeEdge::BottomLeft
        | ResizeEdge::BottomRight => {
            if keyboard_resize_delta(keystroke, Axis::X).is_some() {
                Some(Axis::X)
            } else if keyboard_resize_delta(keystroke, Axis::Y).is_some() {
                Some(Axis::Y)
            } else {
                None
            }
        }
    }
}

/// Returns a signed resize delta for arrow-key input on a focused splitter.
///
/// Arrow keys follow the splitter axis. Shift selects a larger, 50px increment;
/// unmodified arrows move the boundary by 10px. Command/control shortcuts are
/// left to the focused element's normal shortcut handling.
pub(crate) fn keyboard_resize_delta(keystroke: &Keystroke, axis: Axis) -> Option<f32> {
    let modifiers = keystroke.modifiers;
    if modifiers.control || modifiers.platform || modifiers.alt || modifiers.function {
        return None;
    }

    let step = if modifiers.shift { 50.0 } else { 10.0 };
    match (axis, keystroke.key.to_ascii_lowercase().as_str()) {
        (Axis::X, "left" | "arrowleft") => Some(-step),
        (Axis::X, "right" | "arrowright") => Some(step),
        (Axis::Y, "up" | "arrowup") => Some(-step),
        (Axis::Y, "down" | "arrowdown") => Some(step),
        _ => None,
    }
}

/// Returns whether a keystroke should activate a focused dock control.
pub(crate) fn is_keyboard_activation_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("enter")
        || key.eq_ignore_ascii_case("return")
        || key.eq_ignore_ascii_case("space")
        || key == " "
}

/// Makes a stateful GPUI element keyboard-focusable and gives it the standard
/// visible focus outline used across the dock chrome.
pub fn with_focus_ring<E>(element: E, theme: &Theme) -> E
where
    E: StatefulInteractiveElement,
{
    let focus_color = theme.colors.focus;
    element
        .focusable()
        .focus_visible(move |style| style.border_2().border_color(rgb(focus_color)))
}

#[cfg(test)]
mod tests {
    use super::{
        GroupMoveDirection, PanelColumnMoveDirection, PanelMoveDirection, TabReorderDirection,
        is_keyboard_activation_key, keyboard_group_move_direction, keyboard_panel_column_move_direction,
        keyboard_panel_move_direction, keyboard_resize_axis_for_floating_edge,
        keyboard_resize_delta, keyboard_tab_reorder_direction,
    };
    use dock_core::{Axis, ResizeEdge};
    use gpui::{Keystroke, Modifiers};

    fn key(key: &str, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            key: key.to_owned(),
            modifiers,
            key_char: None,
        }
    }

    #[test]
    fn keyboard_group_move_uses_command_shift_up_and_down() {
        let control_shift = Modifiers {
            control: true,
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(
            keyboard_group_move_direction(&key("up", control_shift)),
            Some(GroupMoveDirection::Previous)
        );
        assert_eq!(
            keyboard_group_move_direction(&key("ArrowDown", control_shift)),
            Some(GroupMoveDirection::Next)
        );

        let platform_shift = Modifiers {
            platform: true,
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(
            keyboard_group_move_direction(&key("arrowup", platform_shift)),
            Some(GroupMoveDirection::Previous)
        );
    }

    #[test]
    fn keyboard_group_move_requires_modifiers_and_ignores_other_keys() {
        assert_eq!(
            keyboard_group_move_direction(&key("up", Modifiers::default())),
            None
        );
        assert_eq!(
            keyboard_group_move_direction(&key(
                "down",
                Modifiers {
                    shift: true,
                    ..Modifiers::default()
                },
            )),
            None
        );
        for modifiers in [
            Modifiers {
                control: true,
                shift: true,
                alt: true,
                ..Modifiers::default()
            },
            Modifiers {
                control: true,
                shift: true,
                function: true,
                ..Modifiers::default()
            },
        ] {
            assert_eq!(keyboard_group_move_direction(&key("down", modifiers)), None);
        }
        assert_eq!(
            keyboard_group_move_direction(&key("escape", Modifiers::default())),
            None
        );
    }

    #[test]
    fn keyboard_panel_move_uses_command_alt_shift_up_and_down() {
        let control_alt_shift = Modifiers {
            control: true,
            alt: true,
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(
            keyboard_panel_move_direction(&key("up", control_alt_shift)),
            Some(PanelMoveDirection::Previous)
        );
        assert_eq!(
            keyboard_panel_move_direction(&key("ArrowDown", control_alt_shift)),
            Some(PanelMoveDirection::Next)
        );

        let platform_alt_shift = Modifiers {
            platform: true,
            alt: true,
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(
            keyboard_panel_move_direction(&key("arrowup", platform_alt_shift)),
            Some(PanelMoveDirection::Previous)
        );
    }

    #[test]
    fn keyboard_panel_move_requires_all_modifiers_and_ignores_other_keys() {
        for modifiers in [
            Modifiers::default(),
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
            Modifiers {
                control: true,
                shift: true,
                ..Modifiers::default()
            },
            Modifiers {
                control: true,
                alt: true,
                ..Modifiers::default()
            },
            Modifiers {
                control: true,
                alt: true,
                shift: true,
                function: true,
                ..Modifiers::default()
            },
        ] {
            assert_eq!(keyboard_panel_move_direction(&key("up", modifiers)), None);
        }
        assert_eq!(
            keyboard_panel_move_direction(&key(
                "down",
                Modifiers {
                    control: true,
                    alt: true,
                    shift: true,
                    ..Modifiers::default()
                },
            )),
            Some(PanelMoveDirection::Next)
        );
        assert_eq!(
            keyboard_panel_move_direction(&key(
                "escape",
                Modifiers {
                    control: true,
                    alt: true,
                    shift: true,
                    ..Modifiers::default()
                },
            )),
            None
        );
    }

    #[test]
    fn keyboard_panel_column_move_uses_command_alt_shift_left_and_right() {
        let control_alt_shift = Modifiers {
            control: true,
            alt: true,
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(
            keyboard_panel_column_move_direction(&key("left", control_alt_shift)),
            Some(PanelColumnMoveDirection::Previous)
        );
        assert_eq!(
            keyboard_panel_column_move_direction(&key("ArrowRight", control_alt_shift)),
            Some(PanelColumnMoveDirection::Next)
        );

        let platform_alt_shift = Modifiers {
            platform: true,
            alt: true,
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(
            keyboard_panel_column_move_direction(&key("arrowleft", platform_alt_shift)),
            Some(PanelColumnMoveDirection::Previous)
        );
    }

    #[test]
    fn keyboard_panel_column_move_rejects_other_keys_and_missing_modifiers() {
        assert_eq!(
            keyboard_panel_column_move_direction(&key(
                "up",
                Modifiers {
                    control: true,
                    alt: true,
                    shift: true,
                    ..Modifiers::default()
                },
            )),
            None
        );
        for modifiers in [
            Modifiers::default(),
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
            Modifiers {
                control: true,
                shift: true,
                alt: true,
                ..Modifiers::default()
            },
            Modifiers {
                control: true,
                alt: true,
                shift: true,
                function: true,
                ..Modifiers::default()
            },
        ] {
            assert_eq!(
                keyboard_panel_column_move_direction(&key("right", modifiers)),
                None
            );
        }
    }

    #[test]
    fn keyboard_reorder_accepts_command_shift_arrows() {
        let control_shift = Modifiers {
            control: true,
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(
            keyboard_tab_reorder_direction(&key("left", control_shift)),
            Some(TabReorderDirection::Previous)
        );
        assert_eq!(
            keyboard_tab_reorder_direction(&key("ArrowRight", control_shift)),
            Some(TabReorderDirection::Next)
        );

        let platform_shift = Modifiers {
            platform: true,
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(
            keyboard_tab_reorder_direction(&key("arrowleft", platform_shift)),
            Some(TabReorderDirection::Previous)
        );
    }

    #[test]
    fn keyboard_reorder_requires_modifiers_and_ignores_other_keys() {
        let no_modifiers = Modifiers::default();
        assert_eq!(keyboard_tab_reorder_direction(&key("left", no_modifiers)), None);

        let shift_only = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(keyboard_tab_reorder_direction(&key("right", shift_only)), None);

        let alt_modified = Modifiers {
            control: true,
            shift: true,
            alt: true,
            ..Modifiers::default()
        };
        assert_eq!(keyboard_tab_reorder_direction(&key("left", alt_modified)), None);

        let function_modified = Modifiers {
            control: true,
            shift: true,
            function: true,
            ..Modifiers::default()
        };
        assert_eq!(
            keyboard_tab_reorder_direction(&key("right", function_modified)),
            None
        );
        assert_eq!(
            keyboard_tab_reorder_direction(&key("escape", Modifiers::default())),
            None
        );
    }

    #[test]
    fn floating_edge_resize_selects_only_supported_arrow_axis() {
        assert_eq!(
            keyboard_resize_axis_for_floating_edge(
                &key("left", Modifiers::default()),
                ResizeEdge::Left,
            ),
            Some(Axis::X)
        );
        assert_eq!(
            keyboard_resize_axis_for_floating_edge(
                &key("up", Modifiers::default()),
                ResizeEdge::Left,
            ),
            None
        );
        assert_eq!(
            keyboard_resize_axis_for_floating_edge(
                &key("down", Modifiers::default()),
                ResizeEdge::Bottom,
            ),
            Some(Axis::Y)
        );
        assert_eq!(
            keyboard_resize_axis_for_floating_edge(
                &key("right", Modifiers::default()),
                ResizeEdge::TopLeft,
            ),
            Some(Axis::X)
        );
        assert_eq!(
            keyboard_resize_axis_for_floating_edge(
                &key("up", Modifiers { shift: true, ..Modifiers::default() }),
                ResizeEdge::BottomRight,
            ),
            Some(Axis::Y)
        );
        assert_eq!(
            keyboard_resize_axis_for_floating_edge(
                &key("escape", Modifiers::default()),
                ResizeEdge::BottomRight,
            ),
            None
        );
    }

    #[test]
    fn keyboard_resize_uses_axis_direction_and_shift_step() {
        assert_eq!(
            keyboard_resize_delta(
                &key("left", Modifiers::default()),
                Axis::X
            ),
            Some(-10.0)
        );
        assert_eq!(
            keyboard_resize_delta(
                &key("ArrowRight", Modifiers::default()),
                Axis::X
            ),
            Some(10.0)
        );
        assert_eq!(
            keyboard_resize_delta(
                &key("up", Modifiers::default()),
                Axis::Y
            ),
            Some(-10.0)
        );
        assert_eq!(
            keyboard_resize_delta(
                &key("down", Modifiers { shift: true, ..Modifiers::default() }),
                Axis::Y
            ),
            Some(50.0)
        );
    }

    #[test]
    fn keyboard_resize_ignores_wrong_axis_and_modified_shortcuts() {
        assert_eq!(
            keyboard_resize_delta(&key("up", Modifiers::default()), Axis::X),
            None
        );
        assert_eq!(
            keyboard_resize_delta(&key("right", Modifiers::default()), Axis::Y),
            None
        );
        for modifiers in [
            Modifiers { control: true, ..Modifiers::default() },
            Modifiers { platform: true, ..Modifiers::default() },
            Modifiers { alt: true, ..Modifiers::default() },
            Modifiers { function: true, ..Modifiers::default() },
        ] {
            assert_eq!(keyboard_resize_delta(&key("right", modifiers), Axis::X), None);
        }
    }

    #[test]
    fn keyboard_activation_accepts_enter_return_and_space() {
        assert!(is_keyboard_activation_key("enter"));
        assert!(is_keyboard_activation_key("Enter"));
        assert!(is_keyboard_activation_key("return"));
        assert!(is_keyboard_activation_key("space"));
        assert!(is_keyboard_activation_key(" "));
    }

    #[test]
    fn keyboard_activation_ignores_navigation_and_escape_keys() {
        assert!(!is_keyboard_activation_key("tab"));
        assert!(!is_keyboard_activation_key("left"));
        assert!(!is_keyboard_activation_key("escape"));
    }
}

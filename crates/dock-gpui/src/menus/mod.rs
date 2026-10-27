//! App-owned popup menu state and pure menu models.

mod dockers_menu;
mod group_menu;
mod overflow_menu;

use dock_core::{DockCommand, DockMetrics, Layout, Point, Rect, Size};
use ui::Theme;

use crate::{frame::FrameGeometry, requests::DockMenuRequest};

/// A menu item before it is adapted to GPUI handlers.
#[derive(Clone, Debug)]
pub(crate) struct MenuItemModel {
    pub id: String,
    pub label: String,
    pub checked: bool,
    pub enabled: bool,
    pub command: Option<DockCommand>,
}

/// Transient popup state. Menu state never enters the dock layout or Undo history.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MenuState {
    Closed,
    Dockers {
        anchor: Point,
        selected_index: usize,
    },
    Overflow {
        group: dock_core::NodeId,
        anchor: Point,
        selected_index: usize,
    },
    PanelGroup {
        group: dock_core::NodeId,
        panel: Option<dock_core::PanelId>,
        anchor: Point,
        selected_index: usize,
    },
}

impl MenuState {
    pub(crate) fn open(request: DockMenuRequest, anchor: Point) -> Self {
        match request {
            DockMenuRequest::Dockers => Self::Dockers {
                anchor,
                selected_index: 0,
            },
            DockMenuRequest::TabOverflow { group } => Self::Overflow {
                group,
                anchor,
                selected_index: 0,
            },
            DockMenuRequest::PanelGroup { group, panel } => Self::PanelGroup {
                group,
                panel,
                anchor,
                selected_index: 0,
            },
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        !matches!(self, Self::Closed)
    }

    pub(crate) fn anchor(&self) -> Option<Point> {
        match self {
            Self::Closed => None,
            Self::Dockers { anchor, .. }
            | Self::Overflow { anchor, .. }
            | Self::PanelGroup { anchor, .. } => Some(*anchor),
        }
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        match self {
            Self::Closed => None,
            Self::Dockers { selected_index, .. }
            | Self::Overflow { selected_index, .. }
            | Self::PanelGroup { selected_index, .. } => Some(*selected_index),
        }
    }

    pub(crate) fn move_selection(&mut self, delta: isize, item_count: usize) {
        if item_count == 0 {
            return;
        }
        let Some(current) = self.selected_index() else {
            return;
        };
        let next = (current as isize + delta).rem_euclid(item_count as isize) as usize;
        match self {
            Self::Closed => {}
            Self::Dockers { selected_index, .. }
            | Self::Overflow { selected_index, .. }
            | Self::PanelGroup { selected_index, .. } => *selected_index = next,
        }
    }
}

pub(crate) fn items(
    layout: &Layout,
    theme: &Theme,
    metrics: &DockMetrics,
    frame: &FrameGeometry,
    state: &MenuState,
) -> Vec<MenuItemModel> {
    match state {
        MenuState::Closed => Vec::new(),
        MenuState::Dockers { .. } => dockers_menu::items(layout, theme),
        MenuState::Overflow { group, .. } => {
            overflow_menu::items(layout, frame, *group)
        }
        MenuState::PanelGroup { group, panel, .. } => {
            group_menu::items(layout, metrics, frame, *group, *panel)
        }
    }
}

/// Computes the popup's screen-space bounds, clamped to the window viewport.
pub(crate) fn popup_bounds(anchor: Point, viewport: Rect, item_count: usize) -> Rect {
    const MENU_WIDTH: f32 = 232.0;
    const ROW_HEIGHT: f32 = 30.0;

    let width = MENU_WIDTH.min(viewport.size.width.max(1.0));
    let height = ((item_count.max(1) as f32) * ROW_HEIGHT)
        .min(viewport.size.height.max(1.0));
    let max_x = (viewport.origin.x + viewport.size.width - width).max(viewport.origin.x);
    let max_y = (viewport.origin.y + viewport.size.height - height).max(viewport.origin.y);
    Rect::new(
        Point::new(
            anchor.x.clamp(viewport.origin.x, max_x),
            anchor.y.clamp(viewport.origin.y, max_y),
        ),
        Size::new(width, height),
    )
}

/// Uses half-open hit testing so the right and bottom edges are not part of the popup.
pub(crate) fn contains(bounds: Rect, point: Point) -> bool {
    let right = bounds.origin.x + bounds.size.width;
    let bottom = bounds.origin.y + bounds.size.height;
    point.is_finite()
        && bounds.origin.is_finite()
        && bounds.size.is_finite()
        && bounds.size.width > 0.0
        && bounds.size.height > 0.0
        && right.is_finite()
        && bottom.is_finite()
        && point.x >= bounds.origin.x
        && point.x < right
        && point.y >= bounds.origin.y
        && point.y < bottom
}

#[cfg(test)]
mod tests {
    use super::{MenuState, contains, popup_bounds};
    use crate::requests::DockMenuRequest;
    use dock_core::{NodeId, Point, Rect, Size};
    use slotmap::Key;

    #[test]
    fn menu_selection_wraps_and_closed_state_stays_closed() {
        let mut state = MenuState::open(
            DockMenuRequest::PanelGroup {
                group: NodeId::null(),
                panel: None,
            },
            Point::ZERO,
        );
        state.move_selection(-1, 4);
        assert_eq!(state.selected_index(), Some(3));
        state.move_selection(1, 4);
        assert_eq!(state.selected_index(), Some(0));
        state.move_selection(1, 0);
        assert_eq!(state.selected_index(), Some(0));

        let mut closed = MenuState::Closed;
        closed.move_selection(1, 4);
        assert_eq!(closed, MenuState::Closed);
    }

    #[test]
    fn popup_bounds_clamp_near_every_viewport_edge() {
        let viewport = Rect::new(Point::ZERO, Size::new(320.0, 180.0));
        let bounds = popup_bounds(Point::new(310.0, 170.0), viewport, 5);
        assert_eq!(bounds.origin, Point::new(88.0, 30.0));
        assert_eq!(bounds.size, Size::new(232.0, 150.0));
        assert!(contains(bounds, Point::new(100.0, 40.0)));
        assert!(!contains(bounds, Point::new(320.0, 40.0)));
        assert!(!contains(bounds, Point::new(100.0, 180.0)));
    }
}

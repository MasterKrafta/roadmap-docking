use std::collections::{HashMap, HashSet};

use dock_core::{
    Axis, DockCommand, DockSide, DragHandle, DragPayload, Layout, LayoutRects, Node, NodeId,
    PanelId, Point, Rect, Size,
};
use gpui::{AnyElement, IntoElement};

use crate::chrome_geometry::{GROUP_DRAG_GRIP_HEIGHT, GROUP_MENU_BUTTON_WIDTH};
use crate::components::{
    GroupFrame, GroupGrip, GroupMenuButton, PanelSurface, PanelTab, RailEntry, RectBox,
    SplitterHandle, TabOverflowButton, TabStrip,
};
use crate::geometry::translate_rect_to_local;
use crate::host::DockHost;
use crate::labels::tab_header_label;
use crate::requests::{DockMenuRequest, ResizeHandle};
use crate::surface::RenderData;
use crate::tree_query::{active_panel, main_column_ids, subtree_nodes};

/// Geometry and node subset for one portion of a tree being painted.
///

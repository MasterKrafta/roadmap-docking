//! Runtime owner of native windows for Floating Columns.
//!
//! Main-dock panels do not allocate native surfaces. Each Floating Column owns
//! one HWND containing its Header and complete vertical Group stack. The shared
//! DockController remains the sole owner of Layout and panel views.

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
    time::Duration,
};

use dock_core::{
    DockCommand, DockSide, DragPayload, DragState, DropSource, DropTarget, FloatingGroupId,
    Node, NodeId, PanelId, Point, Rect, Size, SplitterRect,
};
use gpui::{
    AppContext, Bounds, Context, Entity, Subscription, WeakEntity, Window, WindowBounds,
    WindowHandle, WindowId, WindowKind, WindowOptions, point, px, size,
};
use platform::{
    hide_from_taskbar, NativeHorizontalResizeEdge, NativeKey, NativeWindowAtPoint,
    NativeWindowId,
    NativeWindowOps, PhysicalPoint, SystemNativeWindowOps,
};

use crate::{
    DockController, DockEvent, DockTheme, DragHostManager, DragSourceHost,
    components::floating::{floating_column_host_rect, floating_column_local_rect, FloatingColumnHost},
    geometry::DockGeometry,
    window_chain::{WindowGridDirection, WindowGridLink, WindowGridMember, WindowGridRegistry},
    window_chain_geometry::{
        NativeFrameGeometry,
        arrange_window_grid_bounds_with_preserved_offsets as arrange_grid_geometry_bounds,
        match_moving_window_width_to_target,
        window_grid_members_in_native_z_order as grid_window_members_in_native_z_order,
        native_resize_right_edge_delta, native_resize_bottom_edge_delta,
        shift_window_grid_after_resize, floating_column_outer_bounds,
    },
};

#[path = "panel_window_visibility.rs"]
mod panel_window_visibility;
#[path = "panel_window_handle.rs"]
mod panel_window_handle;
#[path = "panel_window_entry.rs"]
mod panel_window_entry;
#[path = "panel_window_registry.rs"]
mod panel_window_registry;
#[path = "docking_window_at_point.rs"]
mod docking_window_at_point;
#[path = "native_move_target.rs"]
mod native_move_target;
#[path = "native_move_window.rs"]
mod native_move_window;
#[path = "native_move_session.rs"]
mod native_move_session;
#[path = "floating_group_snapshot.rs"]
mod floating_group_snapshot;
#[path = "floating_column_window_snapshot.rs"]
mod floating_column_window_snapshot;
#[path = "native_resize_session.rs"]
mod native_resize_session;

pub use panel_window_registry::PanelWindowRegistry;
pub use panel_window_visibility::PanelWindowVisibility;
pub(crate) use docking_window_at_point::DockingWindowAtPoint;
use floating_group_snapshot::FloatingGroupSnapshot;
use floating_column_window_snapshot::FloatingColumnWindowSnapshot;
use native_move_session::NativeMoveSession;
use native_move_target::NativeMoveTarget;
use native_move_window::NativeMoveWindow;
use native_resize_session::NativeResizeSession;
use panel_window_entry::PanelWindowEntry;
use panel_window_handle::PanelWindowHandle;

fn floating_member_drag_point(
    screen: PhysicalPoint,
    client_origin: PhysicalPoint,
    core_bounds: Rect,
    scale: f32,
) -> Option<Point> {
    // Source drags must extrapolate beyond the client rectangle: the cursor
    // routinely leaves its original HWND before the drag is released.
    let local = DockGeometry::host_local_point_from_screen(
        screen,
        client_origin,
        Point::ZERO,
        scale,
    )?;
    Some(Point::new(
        core_bounds.origin.x + local.x,
        core_bounds.origin.y + local.y,
    ))
}

fn floating_column_screen_bounds(
    floating_bounds: Rect,
    local_bounds: Rect,
    scale_factor: f32,
) -> Rect {
    Rect::new(
        Point::new(
            floating_bounds.origin.x + local_bounds.origin.x * scale_factor,
            floating_bounds.origin.y + local_bounds.origin.y * scale_factor,
        ),
        local_bounds.size,
    )
}

fn floating_root_columns(
    layout: &dock_core::Layout,
    group: FloatingGroupId,
) -> Option<Vec<NodeId>> {
    let floating = layout.floating_group(group)?;
    let root = layout.dock_area(floating.area)?.root;
    match layout.node(root) {
        Some(Node::Column(_)) => Some(vec![root]),
        Some(Node::Split(split))
            if split.axis == dock_core::Axis::X
                && !split.children.is_empty()
                && split
                    .children
                    .iter()
                    .all(|child| matches!(layout.node(*child), Some(Node::Column(_)))) =>
        {
            Some(split.children.clone())
        }
        _ => None,
    }
}

fn floating_column_for_group(
    layout: &dock_core::Layout,
    floating_group: FloatingGroupId,
    group: NodeId,
) -> Option<NodeId> {
    floating_root_columns(layout, floating_group)?
        .into_iter()
        .find(|column_id| {
            matches!(
                layout.node(*column_id),
                Some(Node::Column(column)) if column.groups.contains(&group)
            )
        })
}

fn floating_column_core_resize_edge(
    native_edge: Option<NativeHorizontalResizeEdge>,
    is_first_column: bool,
    column_count: usize,
    width_changed: bool,
    left_edge_moved: bool,
) -> Option<dock_core::FloatingResizeEdge> {
    match native_edge {
        Some(NativeHorizontalResizeEdge::Left) => Some(dock_core::FloatingResizeEdge::Left),
        Some(NativeHorizontalResizeEdge::Right)
            if is_first_column && column_count > 1 =>
        {
            Some(dock_core::FloatingResizeEdge::Left)
        }
        Some(NativeHorizontalResizeEdge::Right) => Some(dock_core::FloatingResizeEdge::Right),
        None if width_changed && left_edge_moved => {
            Some(dock_core::FloatingResizeEdge::Left)
        }
        None if width_changed && is_first_column && column_count > 1 => {
            Some(dock_core::FloatingResizeEdge::Left)
        }
        None if width_changed => Some(dock_core::FloatingResizeEdge::Right),
        None => None,
    }
}

fn floating_column_right_resize_enabled(
    is_first_column: bool,
    is_last_column: bool,
    has_multiple_columns: bool,
    has_right_neighbor: bool,
) -> bool {
    (is_first_column && has_multiple_columns)
        || (is_last_column && !has_right_neighbor)
}

fn floating_column_resize_target_bounds(
    current_group_bounds: Rect,
    native_column_bounds: Rect,
    edge: Option<dock_core::FloatingResizeEdge>,
    width_delta: f32,
) -> Rect {
    let origin_x = match edge {
        Some(dock_core::FloatingResizeEdge::Left) => native_column_bounds.origin.x,
        Some(dock_core::FloatingResizeEdge::Right) | None => current_group_bounds.origin.x,
    };
    let horizontal_delta = if edge.is_some() { width_delta } else { 0.0 };

    // Horizontal edges update the shared Core composition width. Vertical
    // size and origin belong to the individual Column HWND, not FloatingGroup.
    Rect::new(
        Point::new(origin_x, current_group_bounds.origin.y),
        Size::new(
            current_group_bounds.size.width + horizontal_delta,
            current_group_bounds.size.height,
        ),
    )
}

fn floating_root_x_splitters(
    layout: &dock_core::Layout,
    group: FloatingGroupId,
    splitters: &[SplitterRect],
) -> Option<Vec<SplitterRect>> {
    let floating = layout.floating_group(group)?;
    let root = layout.dock_area(floating.area)?.root;
    Some(
        splitters
            .iter()
            .filter(|splitter| splitter.owner == root && splitter.axis == dock_core::Axis::X)
            .copied()
            .collect(),
    )
}

fn floating_column_host_bounds(
    layout: &dock_core::Layout,
    metrics: &dock_core::DockMetrics,
    group: FloatingGroupId,
    column: NodeId,
    scale_factor: f32,
) -> Option<(Rect, Rect)> {
    let floating = layout.floating_group(group)?;
    let rects = layout.compute_floating_rects(group, metrics).ok()?;
    let column_rect = rects
        .columns
        .iter()
        .find(|candidate| candidate.column == column)?;
    let root_splitters = floating_root_x_splitters(layout, group, &rects.splitters)?;
    let host_rect = floating_column_host_rect(
        column_rect.rect,
        &root_splitters,
        metrics.splitter_thickness,
    );
    Some((
        floating.bounds,
        floating_column_screen_bounds(floating.bounds, host_rect, scale_factor),
    ))
}

fn collect_floating_column_windows(
    layout: &dock_core::Layout,
    metrics: &dock_core::DockMetrics,
    scale_factor: f32,
    transferred_panels: &HashSet<PanelId>,
) -> Vec<FloatingColumnWindowSnapshot> {
    let mut result = Vec::new();

    for (floating_group, floating) in layout.floating_groups() {
        let Ok(rects) = layout.compute_floating_rects(floating_group, metrics) else {
            continue;
        };
        let Some(root_splitters) =
            floating_root_x_splitters(layout, floating_group, &rects.splitters)
        else {
            continue;
        };

        for column_rect in &rects.columns {
            let Some(Node::Column(column)) = layout.node(column_rect.column) else {
                continue;
            };

            let mut panels = Vec::new();
            let mut seen = HashSet::new();
            for group_id in &column.groups {
                collect_panel_order(layout, *group_id, &mut panels, &mut seen);
            }
            let contains_transferred_panels = panels
                .iter()
                .any(|panel| transferred_panels.contains(panel));
            let host_rect = floating_column_host_rect(
                column_rect.rect,
                &root_splitters,
                metrics.splitter_thickness,
            );
            let bounds = floating_column_screen_bounds(
                floating.bounds,
                host_rect,
                scale_factor,
            );
            if bounds.validate_non_empty().is_err() {
                continue;
            }

            result.push(FloatingColumnWindowSnapshot {
                member: WindowGridMember::Column(column_rect.column),
                floating_group: Some(floating_group),
                main_drag_side: None,
                bounds,
                floating_origin: floating.bounds.origin,
                local_origin: host_rect.origin,
                contains_transferred_panels,
            });
        }
    }

    result
}

fn floating_column_internal_links(
    specs: &[FloatingColumnWindowSnapshot],
) -> Vec<WindowGridLink> {
    let mut floating_groups = Vec::new();
    for spec in specs {
        if !floating_groups.contains(&spec.floating_group()) {
            floating_groups.push(spec.floating_group());
        }
    }

    let mut links = Vec::new();
    for floating_group in floating_groups {
        let mut columns = specs
            .iter()
            .filter(|spec| spec.floating_group() == floating_group)
            .collect::<Vec<_>>();
        columns.sort_by(|left, right| left.local_origin.x.total_cmp(&right.local_origin.x));

        for pair in columns.windows(2) {
            links.push(WindowGridLink {
                from: pair[0].member,
                to: pair[1].member,
                direction: WindowGridDirection::Right,
            });
        }
    }

    links
}

fn floating_column_boundary_member_from_rects(
    candidates: impl IntoIterator<Item = (WindowGridMember, Rect)>,
    side: WindowGridDirection,
) -> Option<WindowGridMember> {
    let candidates = candidates.into_iter().collect::<Vec<_>>();
    let selected = match side {
        WindowGridDirection::Left => candidates.iter().min_by(|(_, left), (_, right)| {
            left.origin
                .x
                .total_cmp(&right.origin.x)
                .then_with(|| left.origin.y.total_cmp(&right.origin.y))
        }),
        WindowGridDirection::Right => candidates.iter().max_by(|(_, left), (_, right)| {
            (left.origin.x + left.size.width)
                .total_cmp(&(right.origin.x + right.size.width))
                .then_with(|| left.origin.y.total_cmp(&right.origin.y))
        }),
        WindowGridDirection::Above => candidates.iter().min_by(|(_, left), (_, right)| {
            left.origin
                .y
                .total_cmp(&right.origin.y)
                .then_with(|| left.origin.x.total_cmp(&right.origin.x))
        }),
        WindowGridDirection::Below => candidates.iter().max_by(|(_, left), (_, right)| {
            (left.origin.y + left.size.height)
                .total_cmp(&(right.origin.y + right.size.height))
                .then_with(|| left.origin.x.total_cmp(&right.origin.x))
        }),
    }?;

    Some(selected.0)
}

fn floating_column_boundary_member(
    specs: &[FloatingColumnWindowSnapshot],
    floating_group: FloatingGroupId,
    side: WindowGridDirection,
) -> Option<WindowGridMember> {
    let candidates = specs
        .iter()
        .filter(|spec| spec.floating_group() == floating_group)
        .map(|spec| {
            (
                spec.member,
                Rect::new(spec.local_origin, spec.bounds.size),
            )
        });
    floating_column_boundary_member_from_rects(candidates, side)
}

fn floating_column_boundary_member_from_members(
    members: &[WindowGridMember],
    side: WindowGridDirection,
) -> Option<WindowGridMember> {
    let columns = members
        .iter()
        .filter(|member| matches!(member, WindowGridMember::Column(_)))
        .copied()
        .collect::<Vec<_>>();

    match side {
        WindowGridDirection::Left | WindowGridDirection::Above => columns.first().copied(),
        WindowGridDirection::Right | WindowGridDirection::Below => columns.last().copied(),
    }
}

fn previous_window_grid_link_is_external(
    link: WindowGridLink,
    previous_owners: &HashMap<WindowGridMember, FloatingGroupId>,
    current_owners: &HashMap<WindowGridMember, FloatingGroupId>,
) -> bool {
    let previous_from = previous_member_floating_group(link.from, previous_owners);
    let previous_to = previous_member_floating_group(link.to, previous_owners);
    let current_from = current_member_floating_group(link.from, current_owners);
    let current_to = current_member_floating_group(link.to, current_owners);
    matches!(
        (previous_from, previous_to, current_from, current_to),
        (Some(previous_from), Some(previous_to), Some(current_from), Some(current_to))
            if previous_from != previous_to && current_from != current_to
    )
}

fn current_member_floating_group(
    member: WindowGridMember,
    current_owners: &HashMap<WindowGridMember, FloatingGroupId>,
) -> Option<FloatingGroupId> {
    match member {
        WindowGridMember::FloatingComposition(group) => current_owners
            .values()
            .any(|owner| *owner == group)
            .then_some(group),
        WindowGridMember::Column(_) => current_owners.get(&member).copied(),
    }
}

fn previous_member_floating_group(
    member: WindowGridMember,
    previous_owners: &HashMap<WindowGridMember, FloatingGroupId>,
) -> Option<FloatingGroupId> {
    match member {
        WindowGridMember::FloatingComposition(group) => Some(group),
        WindowGridMember::Column(_) => previous_owners.get(&member).copied(),
    }
}

fn window_grid_member(group: FloatingGroupId) -> WindowGridMember {
    WindowGridMember::floating_composition(group)
}


fn floating_chain_drop_target_is_supported(
    target: DropTarget,
    target_is_main: bool,
) -> bool {
    match (target_is_main, target) {
        (true, DropTarget::MainColumn { .. }) => true,
        (
            false,
            DropTarget::GroupEdge {
                zone: dock_core::DropZone::Left
                    | dock_core::DropZone::Right
                    | dock_core::DropZone::Top
                    | dock_core::DropZone::Bottom
                    | dock_core::DropZone::FloatingTop
                    | dock_core::DropZone::FloatingBottom,
                ..
            },
        ) => true,
        _ => false,
    }
}

pub(crate) fn floating_window_chain_direction(
    target: DropTarget,
) -> Option<WindowGridDirection> {
    match target {
        DropTarget::GroupEdge {
            zone: dock_core::DropZone::Left,
            ..
        } => Some(WindowGridDirection::Left),
        DropTarget::GroupEdge {
            zone: dock_core::DropZone::Right,
            ..
        } => Some(WindowGridDirection::Right),
        DropTarget::GroupEdge {
            zone: dock_core::DropZone::Top,
            ..
        } => Some(WindowGridDirection::Above),
        DropTarget::GroupEdge {
            zone: dock_core::DropZone::Bottom | dock_core::DropZone::FloatingBottom,
            ..
        } => Some(WindowGridDirection::Below),
        DropTarget::GroupEdge {
            zone: dock_core::DropZone::FloatingTop,
            ..
        } => Some(WindowGridDirection::Above),
        _ => None,
    }
}

fn floating_window_drop_destination(
    anchor: FloatingGroupId,
    target: DropTarget,
    point: Point,
    moving_chain: bool,
    target_column: Option<NodeId>,
) -> Option<NativeMoveTarget> {
    match (moving_chain, floating_window_chain_direction(target), target) {
        (true, Some(direction), DropTarget::GroupEdge { .. }) => {
            Some(NativeMoveTarget::FloatingChain {
                anchor,
                anchor_member: WindowGridMember::Column(target_column?),
                direction,
            })
        }
        _ => Some(NativeMoveTarget::FloatingDock { target, point }),
    }
}

fn floating_groups_in_move_order(
    moving_windows: &[NativeMoveWindow],
) -> Vec<FloatingGroupId> {
    let mut groups = Vec::new();
    for moving in moving_windows {
        if !groups.contains(&moving.group) {
            groups.push(moving.group);
        }
    }
    groups
}

/// Reconciles the native-window registry with the current Core Layout.
pub struct FloatingWindowManager {
    controller: Entity<DockController>,
    theme: DockTheme,
    main_native_window: Option<NativeWindowId>,
    main_host_origin: Point,
    native_move: Option<NativeMoveSession>,
    native_move_windows: Vec<NativeMoveWindow>,
    native_resize: Option<NativeResizeSession>,
    drag_host_manager: Option<WeakEntity<DragHostManager>>,
    reconciling: bool,
    reconcile_scheduled: bool,
    registry: PanelWindowRegistry,
    main_window_id: Option<WindowId>,
    window_chains: WindowGridRegistry,
    /// Ordered permanent Column HWNDs belonging to each Core FloatingGroup.
    column_members_by_group: HashMap<FloatingGroupId, Vec<WindowGridMember>>,
    /// Temporary Column HWNDs opened from Main during a DockArea drag.
    pending_main_drag_columns: HashMap<WindowGridMember, PanelWindowEntry>,
    /// Original Main DockArea screen origin used to move every temporary part together.
    pending_main_drag_origin: Option<PhysicalPoint>,
    pending_window_chain_alignment: HashSet<WindowGridMember>,
    pending_window_grid_reconnections: Vec<WindowGridLink>,
    window_chain_alignment_scheduled: bool,
    native_ops: SystemNativeWindowOps,
    controller_subscription: Option<Subscription>,
}

impl FloatingWindowManager {
    pub fn new(controller: Entity<DockController>, theme: DockTheme) -> Self {
        Self {
            controller,
            theme,
            main_native_window: None,
            main_host_origin: Point::ZERO,
            native_move: None,
            native_move_windows: Vec::new(),
            native_resize: None,
            drag_host_manager: None,
            reconciling: false,
            reconcile_scheduled: false,
            registry: PanelWindowRegistry::default(),
            main_window_id: None,
            window_chains: WindowGridRegistry::default(),
            column_members_by_group: HashMap::new(),
            pending_main_drag_columns: HashMap::new(),
            pending_main_drag_origin: None,
            pending_window_chain_alignment: HashSet::new(),
            pending_window_grid_reconnections: Vec::new(),
            window_chain_alignment_scheduled: false,
            native_ops: SystemNativeWindowOps,
            controller_subscription: None,
        }
    }

    pub fn registry(&self) -> &PanelWindowRegistry {
        &self.registry
    }


    pub(crate) fn floating_group_for_member(&self, member: WindowGridMember) -> Option<FloatingGroupId> {
        match member {
            WindowGridMember::FloatingComposition(group) => Some(group),
            WindowGridMember::Column(_) => self
                .column_members_by_group
                .iter()
                .find_map(|(group, members)| members.contains(&member).then_some(*group)),
        }
    }

    fn floating_column_boundary_member_for_group(
        &self,
        group: FloatingGroupId,
        side: WindowGridDirection,
    ) -> Option<WindowGridMember> {
        let members = self.column_members_by_group.get(&group)?;
        let candidates = members
            .iter()
            .filter(|member| matches!(member, WindowGridMember::Column(_)))
            .filter_map(|member| {
                let entry = self.registry.members.get(member)?;
                let scale = entry
                    .native_scale_factor
                    .filter(|scale| scale.is_finite() && *scale > 0.0)
                    .unwrap_or(1.0);
                Some((
                    *member,
                    Rect::new(
                        entry.bounds.origin,
                        Size::new(
                            entry.bounds.size.width * scale,
                            entry.bounds.size.height * scale,
                        ),
                    ),
                ))
            })
            .collect::<Vec<_>>();
        floating_column_boundary_member_from_rects(candidates, side).or_else(|| {
            floating_column_boundary_member_from_members(members, side)
        })
    }

    fn migrate_window_grid_link(
        &self,
        link: WindowGridLink,
    ) -> Option<WindowGridLink> {
        let from = match link.from {
            WindowGridMember::FloatingComposition(group) => {
                self.floating_column_boundary_member_for_group(group, link.direction)?
            }
            member => self.floating_group_for_member(member).map(|_| member)?,
        };
        let to = match link.to {
            WindowGridMember::FloatingComposition(group) => self
                .floating_column_boundary_member_for_group(group, link.direction.opposite())?,
            member => self.floating_group_for_member(member).map(|_| member)?,
        };
        (from != to).then_some(WindowGridLink {
            from,
            to,
            direction: link.direction,
        })
    }

    fn migrate_pending_window_grid_state(&mut self) {
        let pending_links = std::mem::take(&mut self.pending_window_grid_reconnections);
        for link in pending_links {
            if let Some(link) = self.migrate_window_grid_link(link)
                && !self.pending_window_grid_reconnections.contains(&link)
            {
                self.pending_window_grid_reconnections.push(link);
            }
        }

        let pending_anchors =
            std::mem::take(&mut self.pending_window_chain_alignment);
        for anchor in pending_anchors {
            let migrated = match anchor {
                WindowGridMember::FloatingComposition(group) => {
                    self.column_members_by_group
                        .contains_key(&group)
                        .then(|| self.grid_anchor_member_for_group(group))
                }
                member => self.floating_group_for_member(member).map(|_| member),
            };
            if let Some(anchor) = migrated
                && self.window_chains.is_chained(anchor)
            {
                self.pending_window_chain_alignment.insert(anchor);
            }
        }
    }

    fn grid_anchor_member_for_group(&self, group: FloatingGroupId) -> WindowGridMember {
        self.column_members_by_group
            .get(&group)
            .and_then(|members| members.first().copied())
            .unwrap_or_else(|| window_grid_member(group))
    }

    fn native_bounds_for_grid_members(
        &self,
        members: &[WindowGridMember],
    ) -> Option<Vec<(WindowGridMember, Rect)>> {
        members
            .iter()
            .map(|member| {
                let entry = self.panel_window_for_grid_member(*member)?;
                let native_window = entry.native_window?;
                self.core_bounds_from_native_window(native_window)
                    .map(|bounds| (*member, bounds))
            })
            .collect()
    }

    /// Inserts the moving Floating Column's complete WindowGrid immediately before the anchor.
    #[allow(dead_code)] // Consumed by the whole-window docking stage.
    pub(crate) fn link_window_chain_before(
        &mut self,
        anchor: FloatingGroupId,
        moving: FloatingGroupId,
    ) -> bool {
        self.window_chains.link_before(window_grid_member(anchor), window_grid_member(moving))
    }

    /// Inserts the moving window's complete chain immediately after the anchor.
    #[allow(dead_code)] // Consumed by the whole-window docking stage.
    pub(crate) fn link_window_chain_after(
        &mut self,
        anchor: FloatingGroupId,
        moving: FloatingGroupId,
    ) -> bool {
        self.window_chains.link_after(window_grid_member(anchor), window_grid_member(moving))
    }

    /// Removes a Floating window from its chain without disturbing other members.
    #[allow(dead_code)] // Consumed by the content-extraction lifecycle stage.
    pub(crate) fn remove_window_from_chain(&mut self, group: FloatingGroupId) -> bool {
        let key = window_grid_member(group);
        let previous_members = self.window_chains.members_for(key);
        if !self.window_chains.remove_member(key) { return false; }
        for member in previous_members {
            if member == key || !self.window_chains.is_chained(member) { continue; }
            let component = self.window_chains.members_for(member);
            if component.len() > 1 && component.first() == Some(&member) {
                self.pending_window_chain_alignment.insert(member);
            }
        }
        true
    }

    /// Registers the ordered Core result of one Main DockArea tear-off.
    ///
    /// This method deliberately does not reconcile: DragHost registers the
    /// source-ordered Core result, then the next reconcile migrates those
    /// temporary composition links to the permanent Column members.
    pub(crate) fn register_window_chain(&mut self, groups: &[FloatingGroupId]) -> bool {
        if groups.is_empty() { return false; }
        if groups.len() == 1 { return true; }
        let mut seen = HashSet::new();
        if !groups.iter().all(|group| seen.insert(*group)) { return false; }
        let members = groups.iter().copied().map(window_grid_member).collect::<Vec<_>>();
        let mut next = self.window_chains.clone();
        for pair in members.windows(2) {
            if !next.link_after(pair[0], pair[1]) { return false; }
        }
        self.window_chains = next;
        self.pending_window_chain_alignment.insert(members[0]);
        true
    }

    /// Joins newly-created Floating windows to an existing WindowGrid.
    ///
    /// The source groups remain separate Core FloatingGroups and separate HWNDs.
    /// Horizontal joins keep the established visible-frame alignment. Vertical
    /// joins first normalize the attached window to the target's width; exact
    /// vertical placement is handled by the following geometry stage.
    pub(crate) fn join_window_chain_at(
        &mut self,
        anchor: FloatingGroupId,
        groups: &[FloatingGroupId],
        direction: WindowGridDirection,
        cx: &mut Context<Self>,
    ) -> Result<(), dock_core::CommandError> {
        let anchor_member = window_grid_member(anchor);
        let moving_members = groups.iter().copied().map(window_grid_member).collect::<Vec<_>>();
        let mut next_chains = self.window_chains.clone();
        if !next_chains.link_sequence_to_side(anchor_member, &moving_members, direction) {
            return Err(dock_core::CommandError::InvalidTarget { reason: "could not insert the new Floating windows into the target WindowGrid" });
        }
        let members = next_chains.members_for(anchor_member);
        let Some(member_bounds) = self.controller.update(cx, |controller, _| {
            members.iter().map(|member| {
                let group = member.floating_group()?;
                controller.layout().floating_group(group).map(|floating| (*member, floating.bounds))
            }).collect::<Option<Vec<_>>>()
        }) else {
            return Err(dock_core::CommandError::InvalidTarget { reason: "WindowGrid contains a member without a current FloatingGroup host" });
        };
        let member_bounds = match direction {
            WindowGridDirection::Above | WindowGridDirection::Below => {
                let minimum_width = self.floating_minimum_width(groups[0], cx).ok_or(
                    dock_core::CommandError::InvalidTarget { reason: "could not read the minimum width of the moving FloatingGroup" },
                )?;
                match_moving_window_width_to_target(member_bounds, anchor_member, moving_members[0], minimum_width)
            }
            WindowGridDirection::Left | WindowGridDirection::Right => Some(member_bounds),
        }.ok_or(dock_core::CommandError::InvalidTarget { reason: "could not normalize the new Floating window bounds" })?;
        let links = next_chains.links_for(anchor_member).ok_or(
            dock_core::CommandError::InvalidTarget { reason: "new WindowGrid has no adjacency links" },
        )?.to_vec();
        let bounds = self.arrange_window_grid_bounds(member_bounds, &links, anchor_member, cx)
            .ok_or(dock_core::CommandError::InvalidTarget { reason: "could not calculate native bounds for the new WindowGrid connection" })?;
        let current_bounds = self.controller.update(cx, |controller, _| {
            bounds.iter().map(|(member, _)| {
                let group = member.floating_group()?;
                controller.layout().floating_group(group).map(|floating| (*member, floating.bounds))
            }).collect::<Option<Vec<_>>>()
        }).ok_or(dock_core::CommandError::InvalidTarget { reason: "WindowGrid changed while preparing its native bounds" })?;
        let commands = bounds.iter().filter_map(|(member, bounds)| {
            let group = member.floating_group()?;
            current_bounds.iter().find(|(current_member, _)| current_member == member)
                .filter(|(_, current)| current != bounds)
                .map(|_| DockCommand::ResizeFloatingBounds { group, bounds: *bounds })
        }).collect::<Vec<_>>();
        self.window_chains = next_chains;
        self.pending_window_chain_alignment.insert(anchor_member);
        if commands.is_empty() { self.reconcile(cx); }
        else { self.schedule_window_chain_bounds_update(commands, cx); }
        cx.notify();
        Ok(())
    }

    /// Connects the temporary transport manager without creating an ownership cycle.
    pub(crate) fn attach_drag_host_manager(
        &mut self,
        manager: WeakEntity<DragHostManager>,
    ) {
        self.drag_host_manager = Some(manager);
    }

    pub(crate) fn drag_host_manager(&self) -> Option<Entity<DragHostManager>> {
        self.drag_host_manager.as_ref()?.upgrade()
    }

    /// Resolves each typed WindowGrid identity to its matching permanent native host.
    ///
    /// Column members resolve directly from the Core-node registry. FloatingComposition
    /// remains a compatibility endpoint and resolves to the composition's first Column.
    fn panel_window_for_grid_member(&self, member: WindowGridMember) -> Option<&PanelWindowEntry> {
        match member {
            WindowGridMember::Column(_) => self.registry.members.get(&member),
            WindowGridMember::FloatingComposition(group) => self.column_window_for_group(group),
        }
    }

    fn column_window_for_group(&self, group: FloatingGroupId) -> Option<&PanelWindowEntry> {
        self.column_members_by_group
            .get(&group)?
            .first()
            .and_then(|member| self.registry.members.get(member))
    }

    fn all_native_window_entries(&self) -> impl Iterator<Item = &PanelWindowEntry> {
        self.registry.members.values()
    }

    /// Returns the current DPI scale for a managed GPUI window.
    ///
    /// DragHost uses the source HWND's scale when converting the original
    /// logical grab offset into physical screen coordinates.
    pub(crate) fn native_scale_factor_for_window_id(&self, window_id: WindowId) -> Option<f32> {
        let native_window = if self.main_window_id == Some(window_id) {
            self.main_native_window
        } else {
            self.all_native_window_entries()
                .find(|entry| entry.window_id == window_id)
                .and_then(|entry| entry.native_window)
        }?;
        self.native_ops
            .scale_factor(native_window)
            .ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0)
    }

    /// Reads one HWND's actual outer frame, DWM-visible bounds, client insets
    /// and monitor scale. The fallback is used only before native measurements exist.
    fn native_frame_geometry(
        &self,
        native_window: NativeWindowId,
        fallback_scale: Option<f32>,
    ) -> Option<NativeFrameGeometry> {
        let outer = self.native_ops.window_rect(native_window).ok()?;
        let visible = self.native_ops.visible_frame_rect(native_window).unwrap_or(outer);
        let client_origin = self.native_ops.client_origin(native_window).ok()?;
        let client_size = self.native_ops.client_size(native_window).ok()?;
        let scale = self
            .native_ops
            .scale_factor(native_window)
            .ok()
            .or(fallback_scale)
            .filter(|scale| scale.is_finite() && *scale > 0.0)?;
        NativeFrameGeometry::from_native_rects(
            outer,
            visible,
            client_origin,
            client_size,
            scale,
        )
    }

    /// Computes native outer bounds from the desired Core client rectangle.
    /// The physical origin uses this HWND's own monitor scale and measured insets.
    fn column_outer_bounds_for_target(&self, spec: FloatingColumnWindowSnapshot, native_window: NativeWindowId, fallback_scale: Option<f32>) -> Option<Rect> {
        let frame = self.native_frame_geometry(native_window, fallback_scale)?;
        floating_column_outer_bounds(spec.floating_origin, spec.local_origin, spec.bounds.size, frame)
    }

    /// Returns each member's measured native frame using its own HWND and DPI.
    fn frame_geometries_for_bounds(
        &self,
        bounds: &[(WindowGridMember, Rect)],
    ) -> Option<Vec<NativeFrameGeometry>> {
        bounds.iter().map(|(member, bounds)| {
            let Some(entry) = self.panel_window_for_grid_member(*member) else {
                return NativeFrameGeometry::without_frame_size(bounds.size, 1.0);
            };
            let Some(native_window) = entry.native_window else {
                let scale = entry.native_scale_factor
                    .filter(|scale| scale.is_finite() && *scale > 0.0)
                    .unwrap_or(1.0);
                return NativeFrameGeometry::without_frame_size(bounds.size, scale);
            };
            self.native_frame_geometry(native_window, entry.native_scale_factor)?
                .with_logical_client_size(bounds.size)
        }).collect()
    }

    fn floating_minimum_width(
        &self,
        group: FloatingGroupId,
        cx: &mut Context<Self>,
    ) -> Option<f32> {
        self.controller.update(cx, |controller, _| {
            controller
                .layout()
                .floating_minimum_size(group, &controller.metrics())
                .ok()
                .map(|minimum| minimum.width)
        })
    }

    /// Computes a complete Column WindowGrid from measured native frame and client
    /// geometry. The target anchor never moves during an alignment operation.
    fn arrange_window_grid_bounds(
        &self,
        bounds: Vec<(WindowGridMember, Rect)>,
        links: &[WindowGridLink],
        anchor: WindowGridMember,
        _cx: &mut Context<Self>,
    ) -> Option<Vec<(WindowGridMember, Rect)>> {
        let frames = self.frame_geometries_for_bounds(&bounds)?;
        let owners = bounds
            .iter()
            .map(|(member, _)| Some((*member, self.floating_group_for_member(*member)?)))
            .collect::<Option<HashMap<_, _>>>()?;
        let internal_links = links
            .iter()
            .copied()
            .filter(|link| owners.get(&link.from) == owners.get(&link.to))
            .collect::<HashSet<_>>();
        arrange_grid_geometry_bounds(
            bounds,
            &frames,
            links,
            anchor,
            &internal_links,
        )
    }

    /// Connects the manager to the main GPUI window and the shared controller.
    ///
    /// The main HWND is retained for native drag routing. Reconciliation
    /// creates exactly one persistent host for every Floating Core Column.
    pub(crate) fn attach_main_window_for_drag_routing(&mut self, window: &mut Window) {
        self.main_window_id = Some(window.window_handle().window_id());

        if self.main_native_window.is_none() {
            match self.native_ops.hwnd_of(window) {
                Ok(native_window) => self.main_native_window = Some(native_window),
                Err(error) => {
                    tracing::debug!(
                        %error,
                        "main native handle is unavailable; cross-window drag routing is deferred"
                    );
                }
            }
        }
    }

    pub(crate) fn attach_main_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.attach_main_window_for_drag_routing(window);

        if self.controller_subscription.is_none() {
            let controller = self.controller.clone();
            self.controller_subscription = Some(cx.subscribe(
                &controller,
                |manager, _, event: &DockEvent, manager_cx| {
                    match event {
                        DockEvent::LayoutChanged { .. } => {
                            // LayoutChanged can be emitted from inside a Floating Column host
                            // update. Reconcile updates native hosts, so always defer
                            // it until the active GPUI host update unwinds.
                            manager.schedule_reconcile(manager_cx);
                        }
                        DockEvent::MetricsChanged { .. } => {
                            // Updated metrics change every Column client rectangle.
                            // Queue one real member per connected WindowGrid.
                            let mut anchors = HashSet::new();
                            for group in manager.column_members_by_group.keys().copied() {
                                let anchor = manager.grid_anchor_member_for_group(group);
                                if manager.window_chains.members_for(anchor).len() > 1 {
                                    anchors.insert(anchor);
                                }
                            }
                            manager.pending_window_chain_alignment.extend(anchors);
                            manager.schedule_reconcile(manager_cx);
                        }
                        _ => {}
                    }
                },
            ));
        }

        self.reconcile(cx);
    }

    /// Records the main DockHost origin inside the main window's client area.
    pub(crate) fn set_main_host_origin(&mut self, origin: Point) {
        if origin.validate().is_err() || self.main_host_origin == origin {
            return;
        }
        // FloatingGroup bounds already store virtual-desktop physical pixels.
        // This origin is used only to translate main-host input coordinates.
        self.main_host_origin = origin;
    }

    /// Converts a proposed main-host-local logical rectangle to the Core
    /// floating-bounds contract: physical virtual-desktop origin + logical size.
    pub(crate) fn screen_bounds_from_main_local(&self, bounds: Rect) -> Option<Rect> {
        bounds.validate_non_empty().ok()?;
        let main = self.main_native_window?;
        let client = self.native_ops.client_origin(main).ok()?;
        let scale = self.native_ops.scale_factor(main).ok()?;
        let origin = DockGeometry::screen_position_from_host_local(
            bounds.origin,
            self.main_host_origin,
            client,
            scale,
        )?;
        Some(Rect::new(Point::new(origin.x as f32, origin.y as f32), bounds.size))
    }

    /// Converts a rectangle in one floating host's local logical coordinates
    /// to the Core contract: physical virtual-desktop origin + logical size.
    pub(crate) fn screen_bounds_from_floating_column(
        &self, source_group: FloatingGroupId, column: NodeId, bounds: Rect, cx: &mut Context<Self>,
    ) -> Option<Rect> {
        bounds.validate_non_empty().ok()?;
        let entry = self.registry.members.get(&WindowGridMember::Column(column))?;
        if entry.host_group != Some(source_group) { return None; }
        let window_id = entry.window_id;
        let native_window = entry.native_window?;
        let column_origin = self.controller.update(cx, |controller, _| {
            let rects = controller.floating_geometry(window_id, source_group).ok()?;
            rects.columns.iter().find(|rect| rect.column == column).map(|rect| rect.rect.origin)
        })?;
        self.screen_bounds_from_floating_native_window(native_window, floating_column_local_rect(bounds, column_origin))
    }

    fn screen_bounds_from_floating_native_window(
        &self,
        native_window: NativeWindowId,
        bounds: Rect,
    ) -> Option<Rect> {
        let client = self.native_ops.client_origin(native_window).ok()?;
        let scale = self.native_ops.scale_factor(native_window).ok()?;
        let origin = DockGeometry::screen_position_from_host_local(
            bounds.origin,
            Point::ZERO,
            client,
            scale,
        )?;
        Some(Rect::new(Point::new(origin.x as f32, origin.y as f32), bounds.size))
    }

    /// Checks the cursor against the exact Floating Column HWND that initiated the drag.
    pub(crate) fn cursor_over_other_docking_window_from_source(
        &self,
        source_group: FloatingGroupId,
        source_window_id: WindowId,
        source_column: NodeId,
    ) -> bool {
        let Some(source_window) = self
            .registry
            .members
            .get(&WindowGridMember::Column(source_column))
            .filter(|entry| {
                entry.host_group == Some(source_group) && entry.window_id == source_window_id
            })
            .and_then(|entry| entry.native_window)
        else {
            return false;
        };
        self.cursor_over_other_docking_window_from_native(source_window)
    }

    fn cursor_over_other_docking_window_from_native(
        &self,
        source_window: NativeWindowId,
    ) -> bool {
        let Ok(cursor) = self.native_ops.cursor_position() else {
            return false;
        };

        let mut known = Vec::new();
        if let Some(main) = self.main_native_window {
            known.push(main);
        }
        known.extend(
            self.all_native_window_entries()
                .filter_map(|entry| entry.native_window),
        );

        matches!(
            self.native_ops.window_from_point(cursor, &known, None),
            Ok(NativeWindowAtPoint::Known(target_window)) if target_window != source_window
        )
    }

    pub(crate) fn native_move_active(&self) -> bool { self.native_move.is_some() }

    /// Only one Column in a connected WindowGrid owns the close control.
    ///
    /// Select by actual native right edge so a multi-row WindowGrid still receives
    /// exactly one X. If several Columns share that edge, prefer the uppermost one.
    pub(crate) fn is_rightmost_column(&self, column: NodeId) -> bool {
        let current = WindowGridMember::Column(column);
        let members = self.window_chains.members_for(current);
        let mut rightmost: Option<(WindowGridMember, i32, i32)> = None;

        for member in members {
            if !matches!(member, WindowGridMember::Column(_)) {
                continue;
            }
            let Some(native_window) = self
                .registry
                .members
                .get(&member)
                .and_then(|entry| entry.native_window)
            else {
                continue;
            };
            let Ok(bounds) = self.native_ops.window_rect(native_window) else {
                continue;
            };

            let candidate = (member, bounds.right, bounds.top);
            let should_replace = rightmost.is_none_or(|current| {
                candidate.1 > current.1
                    || (candidate.1 == current.1 && candidate.2 < current.2)
            });
            if should_replace {
                rightmost = Some(candidate);
            }
        }

        rightmost
            .map(|(member, _, _)| member == current)
            .unwrap_or(true)
    }

    /// Closes every Core FloatingGroup represented by the connected Column WindowGrid.
    ///
    /// WindowGrid identifies physical Column HWNDs; the existing ownership
    /// map resolves those members back to their Core FloatingGroup identities.
    /// Commands are filtered against the live Layout so a repeated click is harmless.
    pub(crate) fn close_window_grid(
        &mut self,
        group: FloatingGroupId,
        cx: &mut Context<Self>,
    ) -> Result<(), dock_core::CommandError> {
        let anchor = self.grid_anchor_member_for_group(group);
        let mut groups = Vec::new();
        for member in self.window_chains.members_for(anchor) {
            if let Some(member_group) = self.floating_group_for_member(member)
                && !groups.contains(&member_group)
            {
                groups.push(member_group);
            }
        }
        if !groups.contains(&group) {
            groups.push(group);
        }

        self.controller.update(cx, |controller, controller_cx| {
            let commands = groups
                .into_iter()
                .filter(|group| controller.layout().floating_group(*group).is_some())
                .map(|group| DockCommand::CloseFloatingGroup { group })
                .collect();
            controller
                .dispatch_sequence_in(commands, controller_cx)
                .map(|_| ())
        })
    }

    /// Returns whether an HWND belongs to the persistent Main/Floating hosts.
    ///
    /// DragHost is checked separately because it is a temporary window owned by
    /// DragHostManager rather than part of the persistent window registry.
    pub(crate) fn is_managed_native_window(&self, window: NativeWindowId) -> bool {
        is_managed_foreground_window(
            window,
            self.main_native_window,
            self.all_native_window_entries().map(|entry| entry.native_window),
            None,
        )
    }

    /// A GPUI host losing focus is not a cancellation when another managed
    /// window of this application received the OS foreground activation.
    ///
    /// If the platform cannot query the foreground window, preserve the
    /// existing native-move behavior as a conservative fallback.
    pub(crate) fn should_preserve_pointer_interaction_on_focus_out(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        // Native Floating-window moves and OS-owned resizes are tracked
        // independently of GPUI focus. Both must survive transient activation
        // changes while Windows owns the pointer interaction.
        if self.native_move_active() || self.native_resize.is_some() {
            return true;
        }

        // DockHost can lose focus while the pointer crosses between managed
        // native windows. During a content drag, do not cancel synchronously:
        // DragHost's polling loop debounces transient empty/foreign foreground
        // readings and cancels only after focus stays outside managed hosts.
        let drag_host_active = self.drag_host_manager().is_some_and(|manager| {
            manager.update(cx, |manager, _| manager.active_drag_host_id().is_some())
        });
        if drag_host_active {
            return true;
        }

        let interaction_active = self.controller.update(cx, |controller, _| {
            controller.has_active_pointer_interaction()
        });
        if !interaction_active {
            return false;
        }

        let foreground = match self.native_ops.foreground_window() {
            Ok(Some(window)) => window,
            Ok(None) => return false,
            Err(error) => {
                tracing::debug!(
                    %error,
                    "could not determine the foreground window while handling focus loss"
                );
                return self.native_move_active();
            }
        };

        let drag_host_is_foreground = self.drag_host_manager().is_some_and(|manager| {
            manager.update(cx, |manager, _| {
                manager.active_native_window_id() == Some(foreground)
            })
        });

        self.is_managed_native_window(foreground) || drag_host_is_foreground
    }

    /// Ensures the OS sizing loop is tracked after the first GPUI redraw.
    /// Adopts the already-visible Group DragHost HWND as a complete Floating Column.
    /// The same native surface now owns the Column Header and its Group stack.
    pub(crate) fn adopt_drag_host_column_window(
        &mut self,
        column: NodeId,
        floating_group: FloatingGroupId,
        window: WindowHandle<crate::drag_host_manager::DragHostSurface>,
        window_id: WindowId,
        native_window: NativeWindowId,
        cx: &mut Context<Self>,
    ) -> bool {
        let member = WindowGridMember::Column(column);
        if self.registry.members.contains_key(&member)
            || window.update(cx, |_, _, _| ()).is_err()
        {
            return false;
        }

        let resize_enabled = match self.native_ops.enable_resize(native_window) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(
                    ?member,
                    %error,
                    "could not enable native resize before promoting DragHost to Column"
                );
                false
            }
        };
        let scale = self
            .native_ops
            .scale_factor(native_window)
            .ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        let spec = self.controller.update(cx, |controller, _| {
            let transferred = HashSet::new();
            collect_floating_column_windows(
                controller.layout(),
                &controller.metrics(),
                scale,
                &transferred,
            )
            .into_iter()
            .find(|spec| {
                spec.member == member && spec.floating_group() == floating_group
            })
        });
        let bounds = spec
            .and_then(|spec| {
                self.column_outer_bounds_for_target(spec, native_window, Some(scale))
                    .or(Some(spec.bounds))
            })
            .or_else(|| {
                self.native_ops.window_rect(native_window).ok().and_then(|outer| {
                    let client = self.native_ops.client_size(native_window).ok()?;
                    DockGeometry::core_bounds_from_native(outer, client, scale)
                })
            });
        let Some(bounds) = bounds else {
            return false;
        };
        self.sync_column_window_bounds(native_window, bounds);
        if let Err(error) = window.update(cx, |_, window, app| window.bounds_changed(app)) {
            tracing::warn!(
                ?member,
                %error,
                "could not refresh the promoted DragHost Column bounds"
            );
            return false;
        }

        self.registry.members.insert(
            member,
            PanelWindowEntry {
                window: PanelWindowHandle::DragHost(window),
                window_id,
                native_window: Some(native_window),
                bounds,
                native_min_size: None,
                native_horizontal_resize_min_widths: None,
                native_left_resize_enabled: None,
            native_bottom_resize_enabled: None,
                native_right_resize_enabled: None,
                native_top_resize_enabled: None,
                native_scale_factor: Some(scale),
                native_bounds_dirty: false,
                native_vertical_origin_offset: 0.0,
                native_vertical_height: None,
                core_vertical_height: bounds.size.height,
                resize_enabled,
                host_group: Some(floating_group),
                visibility: PanelWindowVisibility::Visible,
                close_button_visible: None,
            },
        );
        true
    }

    /// Starts native resize tracking for a permanent Floating Column HWND.
    pub(crate) fn observe_native_column_resize(
        &mut self,
        column: NodeId,
        group: FloatingGroupId,
        window_id: WindowId,
        cx: &mut Context<Self>,
    ) {
        self.ensure_native_column_resize_session(column, group, window_id, cx);
    }

    /// Samples the native client bounds of one Floating Column after GPUI reports
    /// a window-bounds change. The manager translates that rectangle to the
    /// containing Floating composition before dispatching a Core resize command.
    /// Samples current native bounds after the GPUI window-bounds observer fires.
    /// All geometry components are read from the HWND at deferred execution time.
    fn ensure_native_column_resize_session(
        &mut self,
        column: NodeId,
        group: FloatingGroupId,
        window_id: WindowId,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.native_move.is_some() {
            return false;
        }
        let member = WindowGridMember::Column(column);
        if let Some(state) = self.native_resize {
            return state.group == group && state.member == member && state.window_id == window_id;
        }
        let Some(entry) = self.registry.members.get(&member) else {
            return false;
        };
        if entry.window_id != window_id || entry.host_group != Some(group) {
            return false;
        }
        let Some(native_window) = entry.native_window else {
            return false;
        };
        if !self.native_ops.is_resizing(native_window).unwrap_or(false) {
            return false;
        }
        // Protect metric updates from the moment Windows enters its modal sizing
        // loop, even before the first size delta opens the Core history transaction.
        let session_started = self.controller.update(cx, |controller, _| {
            controller.begin_native_resize_session()
        });
        if !session_started {
            return false;
        }
        self.native_resize = Some(NativeResizeSession {
            group,
            member,
            window_id,
            transaction_started: false,
        });
        if let Some(entry) = self.registry.members.get_mut(&member) {
            // Keep Core authoritative for every sibling Column, but do not let
            // reconciliation overwrite the HWND currently inside WM_SIZING.
            entry.native_bounds_dirty = true;
        }
        self.sync_native_column_resize_constraints(column, group, window_id, cx);
        self.schedule_native_resize(cx, group, window_id);
        true
    }
    fn schedule_native_resize(
        &self,
        cx: &mut Context<Self>,
        group: FloatingGroupId,
        window_id: WindowId,
    ) {
        let manager = cx.entity().downgrade();
        cx.spawn(async move |_, app| {
            loop {
                app.background_executor().timer(Duration::from_millis(8)).await;
                let Ok(keep_polling) = manager.update(app, |manager, manager_cx| {
                    manager.tick_native_resize(group, window_id, manager_cx)
                }) else {
                    break;
                };
                if !keep_polling {
                    break;
                }
            }
        })
        .detach();
    }

    fn tick_native_resize(
        &mut self,
        group: FloatingGroupId,
        window_id: WindowId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(state) = self.native_resize.filter(|state| {
            state.group == group && state.window_id == window_id
        }) else {
            return false;
        };
        let WindowGridMember::Column(column) = state.member else {
            self.finish_native_resize_transaction(state, cx);
            return false;
        };
        self.tick_native_column_resize(column, group, window_id, state, cx)
    }

    fn tick_native_column_resize(
        &mut self,
        column: NodeId,
        group: FloatingGroupId,
        window_id: WindowId,
        mut state: NativeResizeSession,
        cx: &mut Context<Self>,
    ) -> bool {
        let member = WindowGridMember::Column(column);
        let native_window = self.registry.members.get(&member).and_then(|entry| {
            (entry.window_id == window_id && entry.host_group == Some(group))
                .then_some(entry.native_window)
                .flatten()
        });
        let Some(native_window) = native_window else {
            self.finish_native_resize_transaction(state, cx);
            return false;
        };

        // Poll the native HWND directly because Windows can coalesce several
        // WM_SIZE notifications inside its modal resize loop.
        if self.native_ops.is_resizing(native_window).unwrap_or(false) {
            if let Some(bounds) = self.core_bounds_from_native_window(native_window) {
                self.apply_native_column_resize_bounds(column, group, bounds, &mut state, cx);
            }
            self.sync_native_column_resize_constraints(column, group, window_id, cx);
            self.native_resize = Some(state);
            return true;
        }

        if let Some(bounds) = self.core_bounds_from_native_window(native_window) {
            self.apply_native_column_resize_bounds(column, group, bounds, &mut state, cx);
        }
        self.finish_native_resize_transaction(state, cx);
        false
    }
    fn apply_native_column_resize_bounds(
        &mut self,
        column: NodeId,
        group: FloatingGroupId,
        native_column_bounds: Rect,
        state: &mut NativeResizeSession,
        cx: &mut Context<Self>,
    ) {
        let member = WindowGridMember::Column(column);
        let native_window = self
            .registry
            .members
            .get(&member)
            .filter(|entry| entry.window_id == state.window_id)
            .and_then(|entry| entry.native_window);
        let scale_factor = native_window
            .and_then(|window| self.native_ops.scale_factor(window).ok())
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        let Some((current_group_bounds, base_column_bounds)) = self.controller.update(cx, |controller, _| {
            floating_column_host_bounds(
                controller.layout(),
                &controller.metrics(),
                group,
                column,
                scale_factor,
            )
        }) else {
            return;
        };
        let mut current_column_bounds = base_column_bounds;
        if let Some(entry) = self.registry.members.get(&member) {
            current_column_bounds.origin.y += entry.native_vertical_origin_offset;
            if let Some(height) = entry.native_vertical_height {
                current_column_bounds.size.height = height;
            }
        }

        let native_edge = native_window
            .and_then(|window| self.native_ops.horizontal_resize_edge(window).ok())
            .flatten();
        let width_delta =
            native_column_bounds.size.width - current_column_bounds.size.width;
        let width_changed = width_delta.abs() > 0.5;
        let left_edge_moved =
            (native_column_bounds.origin.x - current_column_bounds.origin.x).abs() > 0.5;
        let (is_first_column, column_count) = self.controller.update(cx, |controller, _| {
            let columns = floating_root_columns(controller.layout(), group)?;
            Some((
                columns.first().is_some_and(|first| *first == column),
                columns.len(),
            ))
        }).unwrap_or((false, 0));
        let edge = floating_column_core_resize_edge(
            native_edge,
            is_first_column,
            column_count,
            width_changed,
            left_edge_moved,
        );

        // Column HWND bounds use physical desktop origins and logical client sizes.
        // Keep the untouched horizontal edge anchored, while also applying the
        // native Y/height so diagonal corner resizing updates both axes.
        let vertical_bounds_changed =
            (native_column_bounds.origin.y - current_column_bounds.origin.y).abs() > 0.5
                || (native_column_bounds.size.height - current_column_bounds.size.height).abs()
                    > 0.5;
        if vertical_bounds_changed {
            let origin_y_offset =
                native_column_bounds.origin.y - base_column_bounds.origin.y;
            if let Some(entry) = self.registry.members.get_mut(&member) {
                entry.native_vertical_origin_offset = origin_y_offset;
                entry.native_vertical_height = Some(native_column_bounds.size.height);
                entry.native_bounds_dirty = true;
            }
            self.controller.update(cx, |controller, controller_cx| {
                controller.set_floating_column_viewport(
                    state.window_id,
                    group,
                    column,
                    native_column_bounds.size.height,
                    origin_y_offset / scale_factor,
                );
                controller_cx.notify();
            });
        }

        // A vertical-only drag is local to this Column HWND. Its layout is
        // recomputed from the native viewport; it must not resize sibling Columns.
        if edge.is_none() {
            return;
        }

        let target_group_bounds = floating_column_resize_target_bounds(
            current_group_bounds,
            native_column_bounds,
            edge,
            width_delta,
        );
        if target_group_bounds == current_group_bounds {
            return;
        }

        if width_changed {
            eprintln!(
                "[FLOATING_RESIZE_DIAG] event=native_sample group={:?} column={:?} hwnd_client_width={:.1} core_column_width={:.1} core_composition_width={:.1} requested_composition_width={:.1} width_delta={:.1} edge={:?}",
                group,
                column,
                native_column_bounds.size.width,
                current_column_bounds.size.width,
                current_group_bounds.size.width,
                target_group_bounds.size.width,
                width_delta,
                edge,
            );
        }

        if !state.transaction_started {
            state.transaction_started = self.controller.update(cx, |controller, _| {
                controller.begin_native_resize_transaction()
            });
        }
        self.native_resize = Some(*state);
        if !state.transaction_started {
            return;
        }

        // Capture the current Column HWND grid before Core changes any Floating
        // bounds. WindowGrid contains Column identities, including internal
        // Axis::X neighbours and external Floating compositions.
        let grid_links = self
            .window_chains
            .links_for(member)
            .unwrap_or_default()
            .to_vec();
        let grid_members = self.window_chains.members_for(member);
        let grid_has_only_columns = grid_members
            .iter()
            .all(|grid_member| matches!(grid_member, WindowGridMember::Column(_)))
            && grid_links.iter().all(|link| {
                matches!(link.from, WindowGridMember::Column(_))
                    && matches!(link.to, WindowGridMember::Column(_))
            });
        let grid_bounds = if grid_links.is_empty() || !grid_has_only_columns {
            None
        } else {
            self.native_bounds_for_grid_members(&grid_members)
        };
        let grid_frames = grid_bounds
            .as_ref()
            .and_then(|bounds| self.frame_geometries_for_bounds(bounds));
        let grid_owner_groups = grid_members
            .iter()
            .filter_map(|grid_member| {
                self.panel_window_for_grid_member(*grid_member)
                    .and_then(|entry| entry.host_group)
                    .map(|owner| (*grid_member, owner))
            })
            .collect::<HashMap<_, _>>();
        let grid_geometry = match (grid_bounds, grid_frames) {
            (Some(bounds), Some(frames)) => Some((bounds, frames, grid_owner_groups)),
            _ => None,
        };

        let command = DockCommand::ResizeFloatingBoundsFromEdge {
            group,
            bounds: target_group_bounds,
            edge: edge.expect("horizontal Column resize must resolve a Core edge"),
            scale_factor,
        };

        self.controller.update(cx, |controller, controller_cx| {
            let primary_result = controller.dispatch(command);
            match primary_result {
                Ok(_) => {
                    // Use Core's accepted Column rectangle, not the raw native
                    // proposal: a Core minimum may clamp a WM_SIZING sample.
                    let accepted_column_bounds = floating_column_host_bounds(
                        controller.layout(),
                        &controller.metrics(),
                        group,
                        column,
                        scale_factor,
                    )
                    .map(|(_, bounds)| {
                        Rect::new(
                            Point::new(bounds.origin.x, native_column_bounds.origin.y),
                            Size::new(bounds.size.width, native_column_bounds.size.height),
                        )
                    });

                    if width_changed {
                        let accepted_core_column_width =
                            accepted_column_bounds.map(|bounds| bounds.size.width);
                        let accepted_core_composition_width = controller
                            .layout()
                            .floating_group(group)
                            .map(|floating| floating.bounds.size.width);
                        eprintln!(
                            "[FLOATING_RESIZE_DIAG] event=core_after_resize group={:?} column={:?} hwnd_client_width={:.1} accepted_core_column_width={:?} accepted_core_composition_width={:?}",
                            group,
                            column,
                            native_column_bounds.size.width,
                            accepted_core_column_width,
                            accepted_core_composition_width,
                        );
                    }

                    if let (Some(accepted_column_bounds), Some((grid_bounds, grid_frames, owners))) =
                        (accepted_column_bounds, grid_geometry.as_ref())
                    {
                        let right_edge_delta = native_resize_right_edge_delta(
                            current_column_bounds,
                            accepted_column_bounds,
                            scale_factor,
                        )
                        .unwrap_or(0.0);
                        let bottom_edge_delta = native_resize_bottom_edge_delta(
                            current_column_bounds,
                            accepted_column_bounds,
                            scale_factor,
                        )
                        .unwrap_or(0.0);

                        if right_edge_delta.abs() > 0.01 || bottom_edge_delta.abs() > 0.01 {
                            let accepted_grid_bounds = grid_bounds
                                .iter()
                                .map(|(grid_member, bounds)| {
                                    if *grid_member == member {
                                        (member, accepted_column_bounds)
                                    } else {
                                        (*grid_member, *bounds)
                                    }
                                })
                                .collect::<Vec<_>>();
                            let accepted_grid_frames = grid_bounds
                                .iter()
                                .zip(grid_frames.iter())
                                .map(|((grid_member, _), frame)| {
                                    if *grid_member == member {
                                        frame.with_logical_client_size(
                                            accepted_column_bounds.size,
                                        )
                                    } else {
                                        Some(*frame)
                                    }
                                })
                                .collect::<Option<Vec<_>>>();

                            if let Some(accepted_grid_frames) = accepted_grid_frames {
                                if let Some(shifted_bounds) = shift_window_grid_after_resize(
                                    accepted_grid_bounds,
                                    &accepted_grid_frames,
                                    &grid_links,
                                    member,
                                    right_edge_delta,
                                    bottom_edge_delta,
                                ) {
                                    // One FloatingGroup can own several Column HWNDs.
                                    // Convert all shifted member rectangles back to one
                                    // composition-origin move and reject inconsistent
                                    // per-Column translations instead of moving it twice.
                                    let mut group_deltas =
                                        HashMap::<FloatingGroupId, (f32, f32)>::new();
                                    let mut consistent = true;

                                    for (shifted_member, shifted) in shifted_bounds {
                                        if shifted_member == member {
                                            continue;
                                        }
                                        let Some(owner) = owners.get(&shifted_member).copied()
                                        else {
                                            continue;
                                        };
                                        if owner == group {
                                            continue;
                                        }
                                        let Some((_, previous)) = grid_bounds
                                            .iter()
                                            .find(|(candidate, _)| *candidate == shifted_member)
                                        else {
                                            continue;
                                        };
                                        let delta = (
                                            shifted.origin.x - previous.origin.x,
                                            shifted.origin.y - previous.origin.y,
                                        );
                                        if delta.0.abs() <= 0.01 && delta.1.abs() <= 0.01 {
                                            continue;
                                        }

                                        if let Some(existing) = group_deltas.get(&owner) {
                                            if (existing.0 - delta.0).abs() > 0.5
                                                || (existing.1 - delta.1).abs() > 0.5
                                            {
                                                consistent = false;
                                                break;
                                            }
                                        } else {
                                            group_deltas.insert(owner, delta);
                                        }
                                    }

                                    if consistent {
                                        let neighbour_commands = group_deltas
                                            .into_iter()
                                            .filter_map(|(neighbour, delta)| {
                                                let floating =
                                                    controller.layout().floating_group(neighbour)?;
                                                let origin = Point::new(
                                                    floating.bounds.origin.x + delta.0,
                                                    floating.bounds.origin.y + delta.1,
                                                );
                                                (origin != floating.bounds.origin).then_some(
                                                    DockCommand::MoveFloating {
                                                        group: neighbour,
                                                        origin,
                                                    },
                                                )
                                            })
                                            .collect::<Vec<_>>();
                                        if !neighbour_commands.is_empty() {
                                            if let Err(error) =
                                                controller.dispatch_sequence(neighbour_commands)
                                            {
                                                tracing::warn!(
                                                    ?group,
                                                    ?column,
                                                    %error,
                                                    "could not reflow the external Column WindowGrid after native Resize"
                                                );
                                            }
                                        }
                                    } else {
                                        tracing::warn!(
                                            ?group,
                                            ?column,
                                            "Column WindowGrid resize produced inconsistent offsets within one Floating composition"
                                        );
                                    }
                                } else {
                                    tracing::warn!(
                                        ?group,
                                        ?column,
                                        "could not calculate the affected Column WindowGrid region after native Resize"
                                    );
                                }
                            } else {
                                tracing::warn!(
                                    ?group,
                                    ?column,
                                    "could not measure the accepted Column HWND frame after native Resize"
                                );
                            }
                        }
                    }
                }
                Err(error) => tracing::warn!(
                    ?group,
                    ?column,
                    %error,
                    "could not synchronize Floating Column native bounds with Core"
                ),
            }
            controller.flush_events(controller_cx);
            controller_cx.notify();
        });
    }

    fn finish_native_resize_transaction(
        &mut self,
        state: NativeResizeSession,
        cx: &mut Context<Self>,
    ) {
        self.native_resize = None;
        self.controller.update(cx, |controller, controller_cx| {
            if state.transaction_started {
                controller.finish_native_resize_transaction();
            }
            controller.end_native_resize_session();
            controller_cx.notify();
        });

        // A resize can finish on a monitor with a different DPI. Re-read the
        // final window scale after the modal loop has exited and resize the
        // native frame to the preserved Core logical size at that scale.
        self.reconcile(cx);
    }

    fn core_bounds_from_native_window(&self, native_window: NativeWindowId) -> Option<Rect> {
        let rect = self.native_ops.window_rect(native_window).ok()?;
        let client_size = self.native_ops.client_size(native_window).ok()?;
        let scale = self.native_ops.scale_factor(native_window).ok()?;
        DockGeometry::core_bounds_from_native(rect, client_size, scale)
    }

    /// A single-tab window moves natively only when it is not joined to other windows.
    ///
    /// For a chained window, tab and group gestures must stay on the content-drag
    /// path so they can detach into a separate Floating window.
    pub(crate) fn begin_single_tab_window_move(
        &mut self,
        group: FloatingGroupId,
        column: NodeId,
        pointer: Point,
        cx: &mut Context<Self>,
    ) -> bool {
        let member = WindowGridMember::Column(column);
        if self.window_chains.is_chained(member) {
            return false;
        }
        self.begin_existing_window_grid_move(group, column, pointer, cx)
    }

    pub(crate) fn begin_existing_window_grid_move(
        &mut self,
        group: FloatingGroupId,
        column: NodeId,
        pointer: Point,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.native_move.is_some() || self.native_resize.is_some() {
            return false;
        }

        let source_member = WindowGridMember::Column(column);
        let Some(source_entry) = self.registry.members.get(&source_member) else {
            return false;
        };
        if source_entry.host_group != Some(group)
            || source_entry.visibility != PanelWindowVisibility::Visible
        {
            return false;
        }
        let Some(source_window) = source_entry.native_window else {
            return false;
        };
        let source_window_id = source_entry.window_id;
        let Ok(cursor) = self.native_ops.cursor_position() else {
            return false;
        };
        let Ok(source_rect) = self.native_ops.window_rect(source_window) else {
            return false;
        };
        let scale = self
            .native_ops
            .scale_factor(source_window)
            .ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        let snapshot = self.controller.update(cx, |controller, _| {
            let floating = controller.layout().floating_group(group)?;
            Some((floating.bounds, controller.metrics()))
        });
        let Some((start_bounds, metrics)) = snapshot else {
            return false;
        };

        // Keep WindowGrid's stable component order here. Native front-to-back
        // ordering is for Z-order, while this order also preserves group order
        // when the complete chain is committed back into Main.
        let anchor = self.grid_anchor_member_for_group(group);
        let members = self.window_chains.members_for(anchor);
        if !members.contains(&source_member) || members.is_empty() {
            return false;
        }

        let mut move_windows = Vec::with_capacity(members.len());
        for member in members {
            let Some(member_group) = self.floating_group_for_member(member) else {
                return false;
            };
            let Some(entry) = self.panel_window_for_grid_member(member) else {
                return false;
            };
            if entry.host_group != Some(member_group)
                || entry.visibility != PanelWindowVisibility::Visible
            {
                return false;
            }
            let Some(native_window) = entry.native_window else {
                return false;
            };
            let rect = if native_window == source_window {
                source_rect
            } else {
                match self.native_ops.window_rect(native_window) {
                    Ok(rect) => rect,
                    Err(_) => return false,
                }
            };
            let origin = PhysicalPoint {
                x: rect.left,
                y: rect.top,
            };
            move_windows.push(NativeMoveWindow {
                group: member_group,
                member,
                native_window,
                drag_origin: origin,
                cancel_origin: origin,
            });
        }

        if !self.controller.update(cx, |controller, _| {
            controller.begin_floating_window_move(group, pointer)
        }) {
            return false;
        }

        self.native_move_windows = move_windows;
        let high_resolution_timer = self.native_ops.begin_high_resolution_timer().is_ok();
        self.native_move = Some(NativeMoveSession {
            group,
            native_window: source_window,
            initial_cursor: cursor,
            start_bounds,
            start_pointer: pointer,
            drag_threshold: metrics.drag_threshold * scale,
            threshold_crossed: false,
            high_resolution_timer,
            drop_target: None,
            source_column_host: column,
        });
        self.schedule_native_move(cx, group, source_window_id);
        true
    }

    fn schedule_native_move(&self, cx: &mut Context<Self>, group: FloatingGroupId, window_id: WindowId) {
        let manager = cx.entity().downgrade();
        cx.spawn(async move |_, app| {
            loop {
                app.background_executor().timer(Duration::from_millis(4)).await;
                let Ok(keep_moving) = manager.update(app, |manager, manager_cx| {
                    manager.tick_native_move(group, window_id, manager_cx)
                }) else {
                    break;
                };
                if !keep_moving {
                    break;
                }
            }
        }).detach();
    }

    fn tick_native_move(&mut self, group: FloatingGroupId, window_id: WindowId, cx: &mut Context<Self>) -> bool {
        let Some(mut state) = self.native_move.filter(|state| state.group == group) else { return false; };
        let source_member = WindowGridMember::Column(state.source_column_host);
        let source_is_current = self.registry.members.get(&source_member).is_some_and(|entry| {
            entry.window_id == window_id
                && entry.native_window == Some(state.native_window)
                && entry.host_group == Some(group)
        });
        if !source_is_current || self.native_move_windows.is_empty() {
            self.cancel_native_move(state, cx);
            return false;
        }
        let pointer_interaction_active = self.controller.update(cx, |controller, _| {
            controller.has_active_pointer_interaction()
        });
        if !pointer_interaction_active {
            self.cancel_native_move(state, cx);
            return false;
        }
        if self.native_ops.key_down(NativeKey::Escape).unwrap_or(false) {
            self.cancel_native_move(state, cx);
            return false;
        }
        let Ok(cursor) = self.native_ops.cursor_position() else {
            self.cancel_native_move(state, cx);
            return false;
        };
        let dx = cursor.x.saturating_sub(state.initial_cursor.x);
        let dy = cursor.y.saturating_sub(state.initial_cursor.y);
        if !state.threshold_crossed {
            let distance = ((dx as f64).powi(2) + (dy as f64).powi(2)).sqrt();
            if distance < state.drag_threshold as f64 {
                if !self.native_ops.left_button_down().unwrap_or(false) {
                    self.finish_native_move(state, cx);
                    return false;
                }
                return true;
            }
            state.threshold_crossed = true;
            let scale = self.native_ops.scale_factor(state.native_window).ok()
                .filter(|scale| scale.is_finite() && *scale > 0.0).unwrap_or(1.0);
            let Some(delta) = DockGeometry::logical_delta_from_physical(
                PhysicalPoint { x: dx, y: dy },
                scale,
            ) else {
                self.cancel_native_move(state, cx);
                return false;
            };
            let Ok(point) = state.start_pointer.checked_offset(delta.x, delta.y) else {
                self.cancel_native_move(state, cx);
                return false;
            };
            self.controller.update(cx, |controller, _| {
                if matches!(controller.drag().session().state(), DragState::Pressed { .. }) {
                    let _ = controller.drag_mut().session_mut().pointer_move(point);
                }
            });
        }

        let moving_windows = self.native_move_windows.clone();
        for member in moving_windows {
            let position = PhysicalPoint {
                x: member.drag_origin.x.saturating_add(dx),
                y: member.drag_origin.y.saturating_add(dy),
            };
            if let Err(error) = self.native_ops.move_window(member.native_window, position) {
                tracing::warn!(?group, moving_group = ?member.group, %error, "native floating chain move failed");
                self.cancel_native_move(state, cx);
                return false;
            }
        }

        // Resolve Core drop zones while all connected Column HWNDs move together.
        self.update_native_drop_target(&mut state, cursor, cx);
        self.native_move = Some(state);
        if !self.native_ops.left_button_down().unwrap_or(false) {
            self.finish_native_move(state, cx);
            return false;
        }
        true
    }

    fn update_native_drop_target(
        &self,
        state: &mut NativeMoveSession,
        cursor: PhysicalPoint,
        cx: &mut Context<Self>,
    ) {
        let source = self.controller.update(cx, |controller, _| {
            let payload = match controller.drag().session().state() {
                DragState::Dragging { payload, .. } => payload,
                _ => return None,
            };
            match payload {
                DragPayload::FloatingGroup { group } if group == state.group => controller
                    .layout()
                    .floating_group(group)
                    .and_then(|floating| controller.layout().dock_area(floating.area))
                    .map(|area| DropSource::Composition { root: area.root }),
                _ => None,
            }
        });
        let Some(source) = source else {
            self.reject_native_drop_target(state, cx);
            return;
        };

        let moving_groups = floating_groups_in_move_order(&self.native_move_windows);
        // A GroupGrip move is a Core node drag even when its Column has a Header
        // HWND alongside it; it must never be converted into a physical WindowGrid join.
        let moving_chain = moving_groups.len() > 1;
        let mut known = Vec::new();
        if let Some(main) = self.main_native_window {
            known.push(main);
        }
        known.extend(self.all_native_window_entries().filter_map(|entry| {
            let native_window = entry.native_window?;
            let is_moving_member = self
                .native_move_windows
                .iter()
                .any(|moving| moving.native_window == native_window);
            (!is_moving_member).then_some(native_window)
        }));
        let target_window = match self.native_ops.window_from_point(
            cursor,
            &known,
            Some(state.native_window),
        ) {
            Ok(NativeWindowAtPoint::Known(window)) => window,
            // Desktop is a valid tear-off destination for a Group. A foreign
            // application window is not: keep the original Group and HWND.
            Ok(NativeWindowAtPoint::Empty) => {
                state.drop_target = None;
                self.clear_native_drag_target(cx);
                return;
            }
            Ok(NativeWindowAtPoint::Foreign) | Err(_) => {
                self.reject_native_drop_target(state, cx);
                return;
            }
        };

        let moving_chain_can_dock_to_main = moving_chain
            && self.controller.update(cx, |controller, _| {
                moving_groups.iter().all(|moving_group| {
                    controller
                        .layout()
                        .floating_group(*moving_group)
                        .and_then(|floating| controller.layout().dock_area(floating.area))
                        .is_some_and(|area| {
                            matches!(controller.layout().node(area.root), Some(Node::Column(_)))
                        })
                })
            });

        let resolved = if Some(target_window) == self.main_native_window {
            let Some(point) = self.core_point_for_main(cursor) else {
                self.reject_native_drop_target(state, cx);
                return;
            };
            self.controller.update(cx, |controller, controller_cx| {
                let target = match controller
                    .prepare_drop_resolver(source)
                    .and_then(|resolver| resolver.resolve(point))
                {
                    Ok(target) => target,
                    Err(_) => {
                        controller.drag_mut().clear_resolver();
                        let _ = controller.drag_mut().session_mut().pointer_move(point);
                        let _ = controller.drag_mut().session_mut().set_drop_target(None);
                        return None;
                    }
                };
                let target = target.filter(|target| {
                    !moving_chain
                        || (moving_chain_can_dock_to_main
                            && floating_chain_drop_target_is_supported(*target, true))
                });
                let target = target.map(|target| {
                    target
                });
                controller.drag_mut().session_mut().pointer_move(point).ok()?;
                controller.drag_mut().session_mut().set_drop_target(target).ok()?;
                controller_cx.notify();
                target.map(|target| NativeMoveTarget::MainDock { target, point })
            })
        } else {
            // Permanent Floating Column hosts are keyed by Core member,
            // not by the legacy PanelId composition registry.
            let typed_target = self.registry.members.iter().find_map(|(member, entry)| {
                if entry.native_window == Some(target_window)
                    && entry.visibility == PanelWindowVisibility::Visible
                {
                    Some((*member, entry.host_group?, entry.window_id))
                } else {
                    None
                }
            });
            let Some((target_member, target_group, target_window_id)) = typed_target else {
                self.reject_native_drop_target(state, cx);
                return;
            };
            let WindowGridMember::Column(target_column) = target_member else {
                self.reject_native_drop_target(state, cx);
                return;
            };
            let Some(point) = self.core_point_for_floating_member(
                WindowGridMember::Column(target_column),
                target_group,
                target_window_id,
                target_window,
                cursor,
                cx,
            ) else {
                self.reject_native_drop_target(state, cx);
                return;
            };

            // Resolve the target Floating's own edge zones. For a multi-composition
            // WindowGrid, cardinal GroupEdge zones extend the physical grid; for one
            // Core composition, the existing Core resolver owns the docking action.
            let resolved_target = self.controller.update(cx, |controller, controller_cx| {
                let resolved = controller
                    .prepare_floating_window_drop_resolver(target_window_id, target_group, source)
                    .and_then(|resolver| resolver.resolve(point));
                let target = match resolved {
                    Ok(target) => target.filter(|target| {
                        !moving_chain || floating_chain_drop_target_is_supported(*target, false)
                    }).map(|target| {
                        target
                    }),
                    Err(error) => {
                        tracing::debug!(
                            ?target_group,
                            %error,
                            "could not resolve Floating-to-Floating drop zone"
                        );
                        controller.drag_mut().clear_resolver();
                        None
                    }
                };

                if controller.drag_mut().session_mut().pointer_move(point).is_err()
                    || controller.drag_mut().session_mut().set_drop_target(target).is_err()
                {
                    controller.drag_mut().clear_resolver();
                    controller_cx.notify();
                    return None;
                }
                controller_cx.notify();
                target
            });

            resolved_target
                .and_then(|target| {
                    let target_column = match target {
                        DropTarget::GroupEdge {
                            group: target_dock_group,
                            ..
                        } if moving_chain => self.controller.update(cx, |controller, _| {
                            floating_column_for_group(
                                controller.layout(),
                                target_group,
                                target_dock_group,
                            )
                        }),
                        _ => None,
                    };
                    floating_window_drop_destination(
                        target_group,
                        target,
                        point,
                        moving_chain,
                        target_column,
                    )
                })
        };
        state.drop_target = resolved;
    }

    fn reject_native_drop_target(
        &self,
        state: &mut NativeMoveSession,
        cx: &mut Context<Self>,
    ) {
        state.drop_target = None;
        self.clear_native_drag_target(cx);
    }

    fn clear_native_drag_target(&self, cx: &mut Context<Self>) {
        self.controller.update(cx, |controller, controller_cx| {
            controller.drag_mut().clear_resolver();
            let _ = controller.drag_mut().session_mut().set_drop_target(None);
            controller_cx.notify();
        });
    }

    /// Creates one temporary native Column window for every Column in a Main DockArea.
    /// The Core tree remains in Main until the drag is released.
    pub(crate) fn open_main_drag_columns(
        &mut self,
        side: DockSide,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.pending_main_drag_columns.is_empty() || self.pending_main_drag_origin.is_some() {
            return false;
        }

        let source = self.controller.update(cx, |controller, _| {
            let area = controller.layout().main_area(side)?;
            let mut node_ids = HashSet::new();
            let mut column_ids = HashSet::new();
            let mut column_groups = HashMap::new();
            crate::dock_host::collect_subtree_nodes(
                controller.layout(),
                area.root,
                &mut node_ids,
                &mut column_ids,
                &mut column_groups,
            );
            let rects = controller.geometry().ok()?.clone();
            let columns = rects.columns.iter()
                .filter(|column| column_ids.contains(&column.column))
                .copied()
                .collect::<Vec<_>>();
            if columns.len() != column_ids.len() {
                return None;
            }
            let first = columns.first()?.rect;
            let mut left = first.origin.x;
            let mut top = first.origin.y;
            let mut right = first.origin.x + first.size.width;
            let mut bottom = first.origin.y + first.size.height;
            for column in columns.iter().skip(1) {
                left = left.min(column.rect.origin.x);
                top = top.min(column.rect.origin.y);
                right = right.max(column.rect.origin.x + column.rect.size.width);
                bottom = bottom.max(column.rect.origin.y + column.rect.size.height);
            }
            let area_bounds = Rect::new(
                Point::new(left, top),
                Size::new(right - left, bottom - top),
            );

            let mut column_windows = Vec::with_capacity(columns.len());
            for column_rect in &columns {
                column_windows.push((WindowGridMember::Column(column_rect.column), column_rect.rect));
            }

            Some((area_bounds, column_windows))
        });
        let Some((area_bounds, column_windows)) = source else {
            return false;
        };
        if column_windows.iter().any(|(member, _)| self.registry.members.contains_key(member)) {
            return false;
        }
        let Some(screen_area_bounds) = self.screen_bounds_from_main_local(area_bounds) else {
            return false;
        };
        let area_origin = PhysicalPoint {
            x: screen_area_bounds.origin.x.round() as i32,
            y: screen_area_bounds.origin.y.round() as i32,
        };

        let mut created: HashMap<WindowGridMember, PanelWindowEntry> = HashMap::new();
        for (member, local_bounds) in column_windows {
            let Some(bounds) = self.screen_bounds_from_main_local(local_bounds) else {
                for entry in created.into_values() {
                    if let Some(native) = entry.native_window {
                        let _ = self.native_ops.hide(native);
                    }
                    entry.window.remove_window(cx);
                }
                return false;
            };
            let spec = FloatingColumnWindowSnapshot {
                member,
                floating_group: None,
                main_drag_side: Some(side),
                bounds,
                floating_origin: Point::ZERO,
                local_origin: local_bounds.origin,
                contains_transferred_panels: false,
            };
            let Some(mut entry) = self.create_column_window(spec, cx) else {
                for entry in created.into_values() {
                    if let Some(native) = entry.native_window {
                        let _ = self.native_ops.hide(native);
                    }
                    entry.window.remove_window(cx);
                }
                return false;
            };
            entry.host_group = None;
            created.insert(member, entry);
        }

        if created.is_empty() {
            return false;
        }
        self.pending_main_drag_columns = created;
        self.pending_main_drag_origin = Some(area_origin);
        true
    }

    /// Shows the temporary Main-origin Column windows after Main has detached
    /// every transferred PanelView.
    pub(crate) fn show_main_drag_columns(&mut self, cx: &mut Context<Self>) -> bool {
        if self.pending_main_drag_columns.is_empty() {
            return false;
        }
        for (member, entry) in &mut self.pending_main_drag_columns {
            let Some(native_window) = entry.native_window else {
                tracing::warn!(?member, "temporary Main drag part has no native HWND");
                return false;
            };
            if let Err(error) = self.native_ops.show_no_activate(native_window) {
                tracing::warn!(?member, %error, "could not show a temporary Main drag part");
                return false;
            }
            entry.visibility = PanelWindowVisibility::Visible;
            if let Err(error) = entry.window.notify_host(cx) {
                tracing::debug!(?member, %error, "could not request the first frame for a temporary Main drag part");
            }
        }
        true
    }

    /// Moves the whole temporary DockArea by one cursor delta while preserving
    /// every Column window's original offset inside the composition.
    pub(crate) fn move_main_drag_columns(
        &self,
        cursor: PhysicalPoint,
        grab_offset: Point,
    ) -> bool {
        let Some(start_origin) = self.pending_main_drag_origin else {
            return false;
        };
        let scale = self.pending_main_drag_columns.values()
            .filter_map(|entry| entry.native_window)
            .find_map(|native_window| {
                self.native_ops.scale_factor(native_window).ok()
                    .filter(|scale| scale.is_finite() && *scale > 0.0)
            })
            .or_else(|| {
                self.main_native_window
                    .and_then(|main| self.native_ops.scale_factor(main).ok())
                    .filter(|scale| scale.is_finite() && *scale > 0.0)
            });
        let Some(scale) = scale else {
            return false;
        };
        let Some(physical_grab_offset) =
            DockGeometry::physical_offset_from_logical(grab_offset, scale)
        else {
            return false;
        };
        let target_origin =
            DockGeometry::physical_position_for_grab(cursor, physical_grab_offset);
        let dx = target_origin.x.saturating_sub(start_origin.x);
        let dy = target_origin.y.saturating_sub(start_origin.y);

        for (member, entry) in &self.pending_main_drag_columns {
            let Some(native_window) = entry.native_window else {
                return false;
            };
            let original = PhysicalPoint {
                x: entry.bounds.origin.x.round() as i32,
                y: entry.bounds.origin.y.round() as i32,
            };
            let position = PhysicalPoint {
                x: original.x.saturating_add(dx),
                y: original.y.saturating_add(dy),
            };
            if let Err(error) = self.native_ops.move_window_to_top(native_window, position) {
                tracing::warn!(?member, %error, "could not move a temporary Main drag part to the foreground");
                return false;
            }
        }
        true
    }

    /// Converts temporary Main-origin Column windows into permanent Floating members.
    /// The HWNDs and GPUI entities are retained; reconciliation only updates their bounds.
    pub(crate) fn adopt_main_drag_columns(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.pending_main_drag_columns.is_empty() {
            return false;
        }

        let scale = self.main_native_window
            .and_then(|main| self.native_ops.scale_factor(main).ok())
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        let desired = self.controller.update(cx, |controller, _| {
            collect_floating_column_windows(
                controller.layout(),
                &controller.metrics(),
                scale,
                &HashSet::new(),
            )
        });
        let desired_by_member = desired.iter()
            .map(|spec| (spec.member, *spec))
            .collect::<HashMap<_, _>>();
        // Every Core member must already have a permanent window or a temporary
        // window from this drag; reconciliation must never need to open a missing part.
        if desired.iter().any(|spec| {
            let pending = self.pending_main_drag_columns.contains_key(&spec.member);
            let existing = self.registry.members.get(&spec.member);
            match (pending, existing) {
                (true, Some(_)) => true,
                (true, None) => false,
                (false, Some(entry)) => entry
                    .native_window
                    .is_none_or(|window| self.native_ops.window_rect(window).is_err()),
                (false, None) => true,
            }
        }) {
            return false;
        }

        // Preflight every member before changing its host identity. This keeps
        // the pending set intact on failure, so the caller can close all windows.
        let pending_members = self.pending_main_drag_columns.iter()
            .map(|(member, entry)| {
                let spec = desired_by_member.get(member).copied()?;
                let native_window = entry.native_window?;
                let available = self.native_ops.window_rect(native_window).is_ok();
                available.then_some((*member, spec, native_window))
            })
            .collect::<Option<Vec<_>>>();
        let Some(pending_members) = pending_members else {
            return false;
        };

        let mut pending = std::mem::take(&mut self.pending_main_drag_columns);
        for (member, spec, native_window) in pending_members {
            let Some(mut entry) = pending.remove(&member) else {
                continue;
            };
            let current_scale = self.native_ops.scale_factor(native_window).ok()
                .filter(|scale| scale.is_finite() && *scale > 0.0)
                .or(entry.native_scale_factor);
            let target_bounds = self
                .column_outer_bounds_for_target(spec, native_window, current_scale)
                .unwrap_or(spec.bounds);
            self.sync_column_window_bounds(native_window, target_bounds);
            entry.bounds = target_bounds;
            entry.native_scale_factor = current_scale;
            entry.native_bounds_dirty = false;
            entry.host_group = Some(spec.floating_group());
            entry.visibility = PanelWindowVisibility::Visible;
            self.registry.members.insert(member, entry);
        }

        // There should be no leftovers because every pending member was preflighted.
        // Keep cleanup safe if a future change adds a member between the two passes.
        for (member, entry) in pending {
            if let Some(native_window) = entry.native_window {
                let _ = self.native_ops.hide(native_window);
            }
            entry.window.remove_window(cx);
            tracing::warn!(?member, "discarded an unadopted Main drag part");
        }
        self.pending_main_drag_origin = None;
        true
    }

    /// Destroys temporary Column windows when a Main DockArea drag is cancelled
    /// or released onto a non-Floating target.
    pub(crate) fn close_main_drag_columns(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let pending = std::mem::take(&mut self.pending_main_drag_columns);
        self.pending_main_drag_origin = None;
        for (member, entry) in pending {
            if let Some(native_window) = entry.native_window
                && let Err(error) = self.native_ops.hide(native_window)
            {
                tracing::debug!(?member, %error, "could not hide a temporary Main drag part");
            }
            entry.window.remove_window(cx);
        }
    }

    /// Finds the topmost managed target and converts the screen point into that target's
    /// own Core-local logical coordinates. A foreign topmost window is reported distinctly.
    pub(crate) fn docking_window_at_point(
        &mut self,
        screen: PhysicalPoint,
        drag_host: NativeWindowId,
        excluded_source_window: Option<WindowId>,
        cx: &mut Context<Self>,
    ) -> Option<DockingWindowAtPoint> {
        let Some(main_window_id) = self.main_window_id else {
            return Some(DockingWindowAtPoint::ForeignWindow);
        };
        let Some(main_native_window) = self.main_native_window else {
            return Some(DockingWindowAtPoint::ForeignWindow);
        };
        let mut known_windows = Vec::with_capacity(
            self.registry.members.len()
                + self.pending_main_drag_columns.len()
                + 1,
        );

        if excluded_source_window != Some(main_window_id) {
            known_windows.push(main_native_window);
        }
        known_windows.extend(self.all_native_window_entries().filter_map(|entry| {
            (entry.visibility == PanelWindowVisibility::Visible
                && Some(entry.window_id) != excluded_source_window)
                .then_some(entry.native_window)
                .flatten()
        }));
        let mut temporary_part_windows = self.pending_main_drag_columns.values()
            .filter(|entry| entry.visibility == PanelWindowVisibility::Visible)
            .filter_map(|entry| entry.native_window)
            .collect::<HashSet<_>>();
        known_windows.extend(temporary_part_windows.iter().copied());

        // The DragHost and every moving temporary Column HWND are transport surfaces,
        // never drop targets. Exclude them one by one so hit testing reaches the
        // persistent Main/Floating window underneath the temporary composition.
        let mut excluded = Some(drag_host);
        let mut resolved_target = None;
        for _ in 0..=temporary_part_windows.len() {
            match self.native_ops.window_from_point(screen, &known_windows, excluded) {
                Ok(NativeWindowAtPoint::Known(window))
                    if temporary_part_windows.remove(&window) =>
                {
                    excluded = Some(window);
                }
                Ok(NativeWindowAtPoint::Known(window)) => {
                    resolved_target = Some(window);
                    break;
                }
                Ok(NativeWindowAtPoint::Foreign) | Err(_) => {
                    return Some(DockingWindowAtPoint::ForeignWindow);
                }
                Ok(NativeWindowAtPoint::Empty) => return None,
            }
        }
        let Some(target_native) = resolved_target else {
            return Some(DockingWindowAtPoint::ForeignWindow);
        };

        if target_native == main_native_window {
            return self
                .core_point_for_main(screen)
                .map(|point| DockingWindowAtPoint::Main { point })
                .or(Some(DockingWindowAtPoint::ForeignWindow));
        }

        if let Some((member, entry)) = self.registry.members.iter().find(|(_, entry)| {
            entry.visibility == PanelWindowVisibility::Visible
                && entry.native_window == Some(target_native)
        }) {
            let Some(group) = entry.host_group else {
                return Some(DockingWindowAtPoint::ForeignWindow);
            };
            let Some(point) = self.core_point_for_floating_member(
                *member,
                group,
                entry.window_id,
                target_native,
                screen,
                cx,
            ) else {
                return Some(DockingWindowAtPoint::ForeignWindow);
            };
            let WindowGridMember::Column(column) = *member else {
                return Some(DockingWindowAtPoint::ForeignWindow);
            };
            return Some(DockingWindowAtPoint::FloatingColumn { group, window_id: entry.window_id, column, point });
        }

        Some(DockingWindowAtPoint::ForeignWindow)
    }

    /// Returns true when the cursor is over a managed Floating Column or legacy host.
    pub(crate) fn cursor_over_floating_window_from_main(&self) -> bool {
        let Some(main) = self.main_native_window else {
            return false;
        };
        let Ok(cursor) = self.native_ops.cursor_position() else {
            return false;
        };
        let mut known = vec![main];
        known.extend(self.all_native_window_entries().filter_map(|entry| {
            (entry.visibility == PanelWindowVisibility::Visible)
                .then_some(entry.native_window)
                .flatten()
        }));
        matches!(
            self.native_ops.window_from_point(cursor, &known, None),
            Ok(NativeWindowAtPoint::Known(window)) if window != main
        )
    }

    /// Converts the screen cursor into the original source host's Core-local logical
    /// coordinates. This can differ from the target point returned by docking_window_at_point.
    pub(crate) fn core_point_for_drag_source(
        &self,
        source_host: DragSourceHost,
        source_window_id: Option<WindowId>,
        screen: PhysicalPoint,
        cx: &mut Context<Self>,
    ) -> Option<Point> {
        match source_host {
            DragSourceHost::Main => {
                if source_window_id.is_some_and(|id| Some(id) != self.main_window_id) {
                    return None;
                }
                self.core_point_for_main(screen)
            }
            DragSourceHost::FloatingColumn { group, column } => {
                let member = WindowGridMember::Column(column);
                let entry = self.registry.members.get(&member)?;
                if entry.host_group != Some(group) || source_window_id.is_some_and(|id| id != entry.window_id) { return None; }
                self.core_point_for_floating_member_drag_source(member, group, entry.window_id, entry.native_window?, screen, cx)
            }
        }
    }

    /// Converts bounds expressed in the original source host's Core-local coordinates
    /// into physical screen coordinates.
    pub(crate) fn screen_bounds_for_drag_source(
        &self,
        source_host: DragSourceHost,
        bounds: Rect,
        cx: &mut Context<Self>,
    ) -> Option<Rect> {
        match source_host {
            DragSourceHost::Main => self.screen_bounds_from_main_local(bounds),
            DragSourceHost::FloatingColumn { group, column } => {
                self.screen_bounds_from_floating_column(group, column, bounds, cx)
            }
        }
    }

    pub(crate) fn core_point_for_main(&self, screen: PhysicalPoint) -> Option<Point> {
        let main = self.main_native_window?;
        let client = self.native_ops.client_origin(main).ok()?;
        let scale = self.native_ops.scale_factor(main).ok()?;
        DockGeometry::host_local_point_from_screen(
            screen,
            client,
            self.main_host_origin,
            scale,
        )
    }

    fn core_point_for_floating_member_drag_source(
        &self,
        member: WindowGridMember,
        group: FloatingGroupId,
        window_id: WindowId,
        native_window: NativeWindowId,
        screen: PhysicalPoint,
        cx: &mut Context<Self>,
    ) -> Option<Point> {
        let core_bounds = self.floating_member_core_bounds(member, group, window_id, cx)?;
        let client_origin = self.native_ops.client_origin(native_window).ok()?;
        let scale = self.native_ops.scale_factor(native_window).ok()?;
        floating_member_drag_point(screen, client_origin, core_bounds, scale)
    }

    fn floating_member_core_bounds(
        &self,
        member: WindowGridMember,
        group: FloatingGroupId,
        window_id: WindowId,
        cx: &mut Context<Self>,
    ) -> Option<Rect> {
        self.controller.update(cx, |controller, _| {
            let rects = controller.floating_geometry(window_id, group).ok()?;
            match member {
                WindowGridMember::Column(column) => rects.columns.iter().find(|rect| rect.column == column).map(|rect| rect.rect),
                WindowGridMember::FloatingComposition(_) => None,
            }
        })
    }

    fn core_point_for_floating_member(
        &self,
        member: WindowGridMember,
        group: FloatingGroupId,
        window_id: WindowId,
        native_window: NativeWindowId,
        screen: PhysicalPoint,
        cx: &mut Context<Self>,
    ) -> Option<Point> {
        // window_from_point() selects this HWND using GetWindowRect, which includes
        // its non-client frame. Map that same physical rectangle onto the Core part
        // bounds so a pointer on the visible/native frame still reaches the outer
        // Core drop zones instead of producing coordinates beyond the part.
        let native_bounds = self.native_ops.window_rect(native_window).ok()?;
        let native_width = (i64::from(native_bounds.right) - i64::from(native_bounds.left)) as f32;
        let native_height = (i64::from(native_bounds.bottom) - i64::from(native_bounds.top)) as f32;
        if native_width <= 0.0 || native_height <= 0.0 {
            return None;
        }

        let core_bounds = self.floating_member_core_bounds(member, group, window_id, cx)?;

        const EDGE_EPSILON: f32 = 0.001;
        let x_fraction = ((i64::from(screen.x) - i64::from(native_bounds.left)) as f32
            / native_width)
            .clamp(0.0, 1.0 - EDGE_EPSILON);
        let y_fraction = ((i64::from(screen.y) - i64::from(native_bounds.top)) as f32
            / native_height)
            .clamp(0.0, 1.0 - EDGE_EPSILON);

        Some(Point::new(
            core_bounds.origin.x + core_bounds.size.width * x_fraction,
            core_bounds.origin.y + core_bounds.size.height * y_fraction,
        ))
    }

    fn finish_native_move(&mut self, state: NativeMoveSession, cx: &mut Context<Self>) {
        if !state.threshold_crossed { self.cancel_native_move(state, cx); return; }
        if state.high_resolution_timer { let _ = self.native_ops.end_high_resolution_timer(); }
        let moving_windows = std::mem::take(&mut self.native_move_windows);
        let moving_groups = floating_groups_in_move_order(&moving_windows);
        let foreground_at_release = self.native_ops.foreground_window().ok().flatten();
        let mut docked_chain_into_main = false;
        let mut floating_bounds = state.start_bounds;
        let move_delta = self.native_move_delta(&moving_windows).or_else(|| {
            moving_windows.iter().find(|moving| moving.native_window == state.native_window).and_then(|moving| {
                let rect = self.native_ops.window_rect(moving.native_window).ok()?;
                Some(Point::new(rect.left.saturating_sub(moving.drag_origin.x) as f32, rect.top.saturating_sub(moving.drag_origin.y) as f32))
            })
        });
        if let Some(delta) = move_delta {
            floating_bounds.origin = Point::new(state.start_bounds.origin.x + delta.x, state.start_bounds.origin.y + delta.y);
        } else {
            tracing::warn!(?state.group, "could not determine the native move delta; preserving the original Floating bounds");
        }
        self.native_move = None;
        let result = match state.drop_target {
            Some(NativeMoveTarget::FloatingChain { anchor, anchor_member, direction }) => {
                let moving_member = WindowGridMember::Column(state.source_column_host);
                self.finish_floating_window_chain_move(moving_member, anchor, anchor_member, direction, &moving_windows, cx)
            }
            Some(NativeMoveTarget::MainDock { target: DropTarget::MainColumn { side, index }, .. }) if moving_groups.len() > 1 => {
                match self.controller.update(cx, |controller, controller_cx| controller.finish_floating_window_chain_dock_in(&moving_groups, side, index, controller_cx)) {
                    Ok(()) => { docked_chain_into_main = true; Ok(()) }
                    Err(error) => { tracing::warn!(?state.group, %error, "could not dock the Floating WindowGrid into Main"); Err(error) }
                }
            }
            _ if moving_groups.len() > 1 => {
                match self.floating_group_bounds_after_native_move(&moving_windows, cx) {
                    Some(bounds) => self.controller.update(cx, |controller, controller_cx| controller.finish_floating_window_chain_move_in(&bounds, controller_cx)),
                    None => Err(dock_core::CommandError::InvalidTarget { reason: "could not calculate one consistent move delta for every Core FloatingGroup in the WindowGrid" }),
                }
            }
            drop_target => {
                let dock_target = match drop_target {
                    Some(NativeMoveTarget::MainDock { target, point }) | Some(NativeMoveTarget::FloatingDock { target, point }) => Some((target, point)),
                    _ => None,
                };
                self.controller.update(cx, |controller, controller_cx| controller.finish_floating_window_move_in(state.group, floating_bounds, dock_target, controller_cx))
            }
        };
        if let Err(error) = result {
            tracing::warn!(?state.group, %error, "could not commit native Floating WindowGrid move");
            self.cancel_controller_move(cx);
            for moving in &moving_windows {
                if let Some(entry) = self.registry.members.get_mut(&moving.member) { entry.native_bounds_dirty = true; }
                else { self.mark_group_native_bounds_dirty(moving.group); }
            }
            if moving_windows.is_empty() { self.mark_group_native_bounds_dirty(state.group); }
            self.schedule_reconcile(cx);
        }
        if matches!(state.drop_target, Some(NativeMoveTarget::FloatingDock { .. }) | Some(NativeMoveTarget::FloatingChain { .. }) | Some(NativeMoveTarget::MainDock { target: DropTarget::MainColumn { .. }, .. })) {
            self.schedule_reconcile(cx);
            if docked_chain_into_main {
                let moving_native_windows = moving_windows.iter().map(|moving| moving.native_window).collect();
                self.schedule_main_activation_after_window_chain_dock(moving_native_windows, foreground_at_release, cx);
            }
        } else { self.redraw_visible_hosts(cx); }
        cx.notify();
    }


    /// Reads the common physical translation applied to every HWND in one move.
    fn native_move_delta(
        &self,
        moving_windows: &[NativeMoveWindow],
    ) -> Option<Point> {
        let mut common_delta: Option<Point> = None;
        for moving in moving_windows {
            let rect = self.native_ops.window_rect(moving.native_window).ok()?;
            let delta = Point::new(
                rect.left.saturating_sub(moving.drag_origin.x) as f32,
                rect.top.saturating_sub(moving.drag_origin.y) as f32,
            );
            if let Some(previous) = common_delta {
                if (previous.x - delta.x).abs() > 1.0
                    || (previous.y - delta.y).abs() > 1.0
                {
                    return None;
                }
            } else {
                common_delta = Some(delta);
            }
        }
        common_delta
    }

    /// Converts one native WindowGrid drag into one Core bounds update per
    /// distinct FloatingGroup, preserving each composition's existing size.
    fn floating_group_bounds_after_native_move(
        &self,
        moving_windows: &[NativeMoveWindow],
        cx: &mut Context<Self>,
    ) -> Option<Vec<(FloatingGroupId, Rect)>> {
        let groups = floating_groups_in_move_order(moving_windows);
        let delta = self.native_move_delta(moving_windows)?;
        if groups.is_empty() {
            return None;
        }

        self.controller.update(cx, |controller, _| {
            groups
                .into_iter()
                .map(|group| {
                    let mut bounds = controller.layout().floating_group(group)?.bounds;
                    bounds.origin = Point::new(
                        bounds.origin.x + delta.x,
                        bounds.origin.y + delta.y,
                    );
                    Some((group, bounds))
                })
                .collect()
        })
    }

    /// Converts arranged native-member bounds back into Core composition bounds.
    /// All HWNDs owned by one FloatingGroup must require the same translation.
    fn core_bounds_from_arranged_grid_members(
        &self,
        original: &[(WindowGridMember, Rect)],
        arranged: &[(WindowGridMember, Rect)],
        moving_windows: &[NativeMoveWindow],
        cx: &mut Context<Self>,
    ) -> Option<Vec<(FloatingGroupId, Rect)>> {
        let drag_delta = self.native_move_delta(moving_windows)?;
        let moving_groups = floating_groups_in_move_order(moving_windows);
        let mut translations = Vec::<(FloatingGroupId, Point)>::new();
        for (member, next_bounds) in arranged {
            let group = self.floating_group_for_member(*member)?;
            let previous = original
                .iter()
                .find(|(candidate, _)| candidate == member)
                .map(|(_, bounds)| *bounds)?;
            let delta = Point::new(
                next_bounds.origin.x - previous.origin.x,
                next_bounds.origin.y - previous.origin.y,
            );
            if let Some((_, previous_delta)) =
                translations.iter().find(|(candidate, _)| *candidate == group)
            {
                if (previous_delta.x - delta.x).abs() > 0.5
                    || (previous_delta.y - delta.y).abs() > 0.5
                {
                    return None;
                }
            } else {
                translations.push((group, delta));
            }
        }

        self.controller.update(cx, |controller, _| {
            translations
                .into_iter()
                .map(|(group, correction)| {
                    let mut bounds = controller.layout().floating_group(group)?.bounds;
                    let move_delta = if moving_groups.contains(&group) {
                        drag_delta
                    } else {
                        Point::ZERO
                    };
                    bounds.origin = Point::new(
                        bounds.origin.x + move_delta.x + correction.x,
                        bounds.origin.y + move_delta.y + correction.y,
                    );
                    Some((group, bounds))
                })
                .collect()
        })
    }

    fn finish_floating_window_chain_move(
        &mut self,
        moving_member: WindowGridMember,
        anchor: FloatingGroupId,
        anchor_member: WindowGridMember,
        direction: WindowGridDirection,
        moving_windows: &[NativeMoveWindow],
        cx: &mut Context<Self>,
    ) -> Result<(), dock_core::CommandError> {
        if self.floating_group_for_member(anchor_member) != Some(anchor) {
            return Err(dock_core::CommandError::InvalidTarget {
                reason: "the selected Floating drop target is no longer part of its WindowGrid",
            });
        }

        let mut next_chains = self.window_chains.clone();
        let moving_members = next_chains.members_for(moving_member);
        if moving_members.contains(&anchor_member) {
            let bounds = self
                .floating_group_bounds_after_native_move(moving_windows, cx)
                .ok_or(dock_core::CommandError::InvalidTarget {
                    reason: "could not read consistent native bounds for the existing WindowGrid",
                })?;
            return self.controller.update(cx, |controller, controller_cx| {
                controller.finish_floating_window_chain_move_in(&bounds, controller_cx)
            });
        }

        if !next_chains.link_to_side(anchor_member, moving_member, direction) {
            return Err(dock_core::CommandError::InvalidTarget {
                reason: "could not join the moving WindowGrid at an unambiguous boundary",
            });
        }

        let members = next_chains.members_for(anchor_member);
        let member_bounds = self
            .native_bounds_for_grid_members(&members)
            .ok_or(dock_core::CommandError::InvalidTarget {
                reason: "could not read the native bounds of every Column in the joined WindowGrid",
            })?;
        let links = next_chains
            .links_for(anchor_member)
            .ok_or(dock_core::CommandError::InvalidTarget {
                reason: "the joined WindowGrid has no adjacency links",
            })?
            .to_vec();
        let arranged_bounds = self
            .arrange_window_grid_bounds(member_bounds.clone(), &links, anchor_member, cx)
            .ok_or(dock_core::CommandError::InvalidTarget {
                reason: "could not calculate native bounds for the joined Floating WindowGrid",
            })?;
        let core_bounds = self
            .core_bounds_from_arranged_grid_members(
                &member_bounds,
                &arranged_bounds,
                moving_windows,
                cx,
            )
            .ok_or(dock_core::CommandError::InvalidTarget {
                reason: "the joined native members require inconsistent positions inside one Core FloatingGroup",
            })?;

        let result = self.controller.update(cx, |controller, controller_cx| {
            controller.finish_floating_window_chain_move_in(&core_bounds, controller_cx)
        });
        if result.is_ok() {
            self.window_chains = next_chains;
            self.pending_window_chain_alignment.insert(anchor_member);
            self.schedule_reconcile(cx);
            self.schedule_pending_window_chain_alignment(cx);
        }
        result
    }

    fn cancel_native_move(&mut self, state: NativeMoveSession, cx: &mut Context<Self>) {
        self.native_move = None;
        let moving_windows = std::mem::take(&mut self.native_move_windows);
        if state.high_resolution_timer {
            let _ = self.native_ops.end_high_resolution_timer();
        }
        if let Err(error) = self.controller.update(cx, |controller, controller_cx| {
            let result = controller.cancel_pointer_interaction_on_escape();
            controller.flush_events(controller_cx);
            controller_cx.notify();
            result
        }) {
            tracing::warn!(
                ?state.group,
                %error,
                "could not cancel Core pointer interaction for native Floating move"
            );
        }
        for member in moving_windows {
            if let Err(error) = self.native_ops.move_window(member.native_window, member.cancel_origin) {
                tracing::warn!(?state.group, moving_group = ?member.group, %error, "could not restore floating chain member after cancellation");
            }
        }
        cx.notify();
    }

    fn cancel_controller_move(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.controller.update(cx, |controller, controller_cx| {
            let result = controller.cancel_pointer_interaction();
            controller.flush_events(controller_cx);
            controller_cx.notify();
            result
        }) {
            tracing::warn!(
                %error,
                "could not cancel Core pointer interaction after Floating move failure"
            );
        }
    }

    /// Applies pending WindowChain bounds after the current GPUI update unwinds.
    ///
    /// Publishing LayoutChanged while this entity is updating would re-enter
    /// FloatingWindowManager through its controller subscription.
    fn schedule_window_chain_bounds_update(
        &self,
        commands: Vec<DockCommand>,
        cx: &mut Context<Self>,
    ) {
        if commands.is_empty() {
            return;
        }

        let controller = self.controller.clone();
        cx.spawn(async move |_, app| {
            app.background_executor()
                .timer(Duration::from_millis(1))
                .await;
            let result = controller.update(app, |controller, controller_cx| {
                controller.dispatch_sequence_in(commands, controller_cx)
            });
            match result {
                Ok(_) => {}
                Err(error) => tracing::warn!(
                    %error,
                    "could not commit the arranged native bounds for a Floating WindowChain"
                ),
            }
        })
        .detach();
    }

    /// Runs host reconciliation after the active GPUI update has unwound.
    ///
    /// Floating Core drops can change the set of permanent Column hosts.
    /// Updating those hosts synchronously from the controller LayoutChanged
    /// subscription can re-enter the same GPUI entity.
    fn schedule_reconcile(&mut self, cx: &mut Context<Self>) {
        if self.reconcile_scheduled {
            return;
        }
        self.reconcile_scheduled = true;
        let manager = cx.entity().downgrade();
        cx.spawn(async move |_, app| {
            app.background_executor()
                .timer(Duration::from_millis(1))
                .await;
            let _ = manager.update(app, |manager, manager_cx| {
                manager.reconcile_scheduled = false;
                manager.reconcile(manager_cx);
                manager.redraw_visible_hosts(manager_cx);
            });
        })
        .detach();
    }

    fn mark_group_native_bounds_dirty(&mut self, group: FloatingGroupId) {
        let Some(members) = self.column_members_by_group.get(&group) else {
            return;
        };
        for member in members {
            if let Some(entry) = self.registry.members.get_mut(member) {
                entry.native_bounds_dirty = true;
            }
        }
    }

    fn physical_origin_for_bounds(&self, bounds: Rect) -> Option<PhysicalPoint> {
        DockGeometry::physical_point_from_core(bounds.origin)
    }

    /// WindowOptions bounds use GPUI logical screen coordinates while Core
    /// stores physical desktop origins. The main-window scale is only the
    /// creation-time approximation; after the HWND is placed, sync_window_bounds
    /// reads its destination monitor's DPI and corrects the native size.
    fn gpui_origin_for_bounds(&self, bounds: Rect) -> Point {
        let scale = self.main_native_window
            .and_then(|main| self.native_ops.scale_factor(main).ok())
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        DockGeometry::logical_point_from_core_physical(bounds.origin, scale)
            .unwrap_or(Point::ZERO)
    }

    /// Makes the typed Column window registry match the current Core Layout.
    ///
    /// Each Floating Column owns one typed native host containing its Header and Group stack.
    /// Main remains in its original MainDockView and never enters this registry.
    pub fn reconcile(&mut self, cx: &mut Context<Self>) {
        if self.reconciling {
            return;
        }
        self.reconciling = true;

        let (groups, transferred_panels) = self.controller.update(cx, |controller, _| {
            let transferred_panels = controller
                .drag()
                .host_session()
                .map(|session| session.transferred_panels().to_vec())
                .unwrap_or_default();
            let mut groups = controller
                .layout()
                .floating_groups()
                .map(|(id, group)| FloatingGroupSnapshot {
                    id,
                    z_order: group.z_order,
                })
                .collect::<Vec<_>>();
            groups.sort_by_key(|group| group.z_order);
            (groups, transferred_panels)
        });

        let prune_result = self.reconcile_column_windows(&transferred_panels, cx);
        if prune_result.removed_members > 0 {
            tracing::debug!(
                removed_members = prune_result.removed_members,
                "removed stale Floating Column HWND entries"
            );
        }
        for candidate in prune_result.reconnect_candidates {
            if let Some(candidate) = self.migrate_window_grid_link(candidate)
                && !self.pending_window_grid_reconnections.contains(&candidate)
            {
                self.pending_window_grid_reconnections.push(candidate);
            }
        }

        self.synchronize_native_column_z_order(&groups);
        let live_members = self.registry.members.keys().copied().collect::<HashSet<_>>();

        // Reconnect surviving physical Column HWNDs only after their replacement
        // hosts are visible and the full candidate grid passes native geometry checks.
        let mut reconnected_grid = false;
        let reconnect_candidates = std::mem::take(&mut self.pending_window_grid_reconnections);
        for candidate in reconnect_candidates {
            if !live_members.contains(&candidate.from) || !live_members.contains(&candidate.to) {
                continue;
            }
            match self.try_reconnect_pruned_window_grid(candidate, cx) {
                Some(true) => {
                    reconnected_grid = true;
                    tracing::debug!(
                        from = ?candidate.from,
                        to = ?candidate.to,
                        direction = ?candidate.direction,
                        "reconnected surviving WindowGrid members after Column removal"
                    );
                }
                Some(false) => tracing::debug!(
                    from = ?candidate.from,
                    to = ?candidate.to,
                    direction = ?candidate.direction,
                    "kept surviving WindowGrid components separate because reconnect geometry was invalid"
                ),
                None => self.pending_window_grid_reconnections.push(candidate),
            }
        }

        if reconnected_grid {
            self.synchronize_native_column_z_order(&groups);
        }
        self.schedule_pending_window_chain_alignment(cx);
        self.reconciling = false;
    }

    fn synchronize_native_column_z_order(&self, groups: &[FloatingGroupSnapshot]) {
        let moving_members = self
            .native_move
            .map(|_| {
                self.native_move_windows
                    .iter()
                    .map(|moving| moving.member)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let mut member_z_orders = Vec::new();
        for group in groups {
            if let Some(members) = self.column_members_by_group.get(&group.id) {
                member_z_orders.extend(members.iter().copied().map(|member| (member, group.z_order)));
            }
        }
        let ordered = grid_window_members_in_native_z_order(&member_z_orders, &self.window_chains);

        for member in ordered {
            if moving_members.contains(&member) {
                continue;
            }
            let Some(entry) = self.registry.members.get(&member) else {
                continue;
            };
            if entry.visibility != PanelWindowVisibility::Visible {
                continue;
            }
            let Some(native_window) = entry.native_window else {
                continue;
            };
            let position = match self.native_ops.window_rect(native_window) {
                Ok(rect) => PhysicalPoint {
                    x: rect.left,
                    y: rect.top,
                },
                Err(error) => {
                    tracing::debug!(
                        ?member,
                        %error,
                        "could not read Floating member bounds before Z-order synchronization"
                    );
                    continue;
                }
            };
            if let Err(error) = self.native_ops.move_window_to_top(native_window, position) {
                tracing::debug!(
                    ?member,
                    %error,
                    "could not synchronize a Floating member's native Z-order"
                );
            }
        }
    }

    /// Attempts a straight-through reconnection after a deleted WindowGrid member.
    ///
    /// None means that native hosts are not ready yet and the candidate should
    /// be retried. Some(false) means that the graph or measured geometry
    /// rejected the connection, so the surviving components remain independent.
    fn try_reconnect_pruned_window_grid(
        &mut self,
        candidate: WindowGridLink,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        let mut next_chains = self.window_chains.clone();
        if !next_chains.link_to_side(candidate.from, candidate.to, candidate.direction) {
            return Some(false);
        }
        let members = next_chains.members_for(candidate.from);
        if !members.iter().all(|member| {
            self.panel_window_for_grid_member(*member).is_some_and(|entry| {
                entry.visibility == PanelWindowVisibility::Visible && entry.native_window.is_some()
            })
        }) {
            return None;
        }
        let Some(member_bounds) = self.native_bounds_for_grid_members(&members) else {
            return None;
        };
        let Some(links) = next_chains.links_for(candidate.from).map(|links| links.to_vec()) else {
            return Some(false);
        };
        if self
            .arrange_window_grid_bounds(member_bounds, &links, candidate.from, cx)
            .is_none()
        {
            return Some(false);
        }
        self.window_chains = next_chains;
        for member in members {
            self.pending_window_chain_alignment.remove(&member);
        }
        self.pending_window_chain_alignment.insert(candidate.from);
        Some(true)
    }

    /// Defers initial Column frame alignment until the current reconcile update ends.
    ///
    /// Reading native frame geometry and then applying the correction in a separate
    /// controller update avoids re-entering this FloatingWindowManager through its
    /// LayoutChanged subscription.
    fn schedule_pending_window_chain_alignment(&mut self, cx: &mut Context<Self>) {
        if self.window_chain_alignment_scheduled
            || self.pending_window_chain_alignment.is_empty()
        {
            return;
        }
        self.window_chain_alignment_scheduled = true;
        let manager = cx.entity().downgrade();
        let controller = self.controller.clone();

        cx.spawn(async move |_, app| {
            // Let ShowWindow/SetWindowPos reach DWM before sampling extended
            // frame bounds. A 1 ms delay can still read the previous invisible
            // resize-frame geometry and leave a visible seam between the windows.
            app.background_executor()
                .timer(Duration::from_millis(32))
                .await;

            let pending = manager.update(app, |manager, manager_cx| {
                manager.window_chain_alignment_scheduled = false;
                manager.collect_pending_window_chain_corrections(manager_cx)
            });
            let Ok(pending) = pending else {
                return;
            };

            for (anchor, corrections) in pending {
                match controller.update(app, |controller, controller_cx| {
                    controller.apply_window_chain_position_corrections_in(
                        &corrections,
                        controller_cx,
                    )
                }) {
                    Ok(()) => {
                        let _ = manager.update(app, |manager, _| {
                            manager.pending_window_chain_alignment.remove(&anchor);
                        });
                    }
                    Err(error) => {
                        tracing::debug!(
                            ?anchor,
                            %error,
                            "could not commit initial native-frame alignment for the Floating WindowGrid"
                        );
                    }
                }
            }
        })
        .detach();
    }

    /// Calculates seam corrections while every native Column frame is available.
    ///
    /// This stage only reads the controller. The resulting positions are applied later,
    /// after this manager update has unwound.
    fn collect_pending_window_chain_corrections(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Vec<(WindowGridMember, Vec<(FloatingGroupId, Point)>)> {
        let anchors = self.pending_window_chain_alignment.iter().copied().collect::<Vec<_>>();
        let mut pending = Vec::new();
        for anchor in anchors {
            if self.floating_group_for_member(anchor).is_none() {
                self.pending_window_chain_alignment.remove(&anchor);
                continue;
            }
            if !self.window_chains.is_chained(anchor) {
                self.pending_window_chain_alignment.remove(&anchor);
                continue;
            }

            let members = self.window_chains.members_for(anchor);
            if members.len() < 2 {
                self.pending_window_chain_alignment.remove(&anchor);
                continue;
            }
            if !members.iter().all(|member| {
                self.panel_window_for_grid_member(*member).is_some_and(|entry| {
                    entry.visibility == PanelWindowVisibility::Visible
                        && entry.native_window.is_some()
                })
            }) {
                continue;
            }

            // The graph now describes individual native HWNDs, not Core
            // FloatingGroups. Read each member's actual client size and outer
            // origin instead of reusing the composition's shared Core bounds.
            let Some(member_bounds) = self.native_bounds_for_grid_members(&members) else {
                continue;
            };
            let Some(links) = self.window_chains.links_for(anchor).map(|links| links.to_vec())
            else {
                self.pending_window_chain_alignment.remove(&anchor);
                continue;
            };
            let Some(aligned_bounds) =
                self.arrange_window_grid_bounds(member_bounds.clone(), &links, anchor, cx)
            else {
                tracing::debug!(
                    ?anchor,
                    "native frame measurements are not ready for Floating WindowGrid alignment"
                );
                continue;
            };

            // Internal Column links are already described by Core local
            // geometry. External links may translate a whole Core composition,
            // but all members belonging to that composition must agree on one
            // common translation; otherwise Core cannot represent the result.
            let mut translations = HashMap::<FloatingGroupId, Point>::new();
            let mut inconsistent = false;
            for (member, aligned) in &aligned_bounds {
                let Some(group) = self.floating_group_for_member(*member) else {
                    inconsistent = true;
                    break;
                };
                let Some(original) = member_bounds
                    .iter()
                    .find(|(candidate, _)| candidate == member)
                    .map(|(_, bounds)| *bounds)
                else {
                    inconsistent = true;
                    break;
                };
                let delta = Point::new(
                    aligned.origin.x - original.origin.x,
                    aligned.origin.y - original.origin.y,
                );
                match translations.get(&group).copied() {
                    Some(previous)
                        if (previous.x - delta.x).abs() > 0.5
                            || (previous.y - delta.y).abs() > 0.5 =>
                    {
                        inconsistent = true;
                        break;
                    }
                    Some(_) => {}
                    None => {
                        translations.insert(group, delta);
                    }
                }
            }
            if inconsistent {
                tracing::warn!(
                    ?anchor,
                    "WindowGrid alignment would require independent positions inside one Core FloatingGroup"
                );
                self.pending_window_chain_alignment.remove(&anchor);
                continue;
            }

            let current_origins = self.controller.update(cx, |controller, _| {
                translations
                    .keys()
                    .map(|group| {
                        controller
                            .layout()
                            .floating_group(*group)
                            .map(|floating| (*group, floating.bounds.origin))
                    })
                    .collect::<Option<HashMap<_, _>>>()
            });
            let Some(current_origins) = current_origins else {
                continue;
            };
            let corrections = translations
                .into_iter()
                .filter_map(|(group, delta)| {
                    let origin = current_origins.get(&group).copied()?;
                    let next = Point::new(origin.x + delta.x, origin.y + delta.y);
                    (next != origin).then_some((group, next))
                })
                .collect::<Vec<_>>();

            if corrections.is_empty() {
                self.pending_window_chain_alignment.remove(&anchor);
            } else {
                pending.push((anchor, corrections));
            }
        }
        pending
    }


    pub(crate) fn show_handoff_target(
        &mut self,
        group: FloatingGroupId,
        expected_groups: &[FloatingGroupId],
        cx: &mut Context<Self>,
    ) -> bool {
        // DragHostManager supplies this snapshot before updating FloatingWindowManager.
        // Do not re-enter DragHostManager here: it is already handling the release.
        let expected_groups = expected_groups.iter().copied().collect::<HashSet<_>>();
        if expected_groups.is_empty() {
            return false;
        }

        // LayoutChanged reconciliation is deferred to avoid entity re-entry.
        // At handoff time the DragHost has released its PanelViews, so reconcile
        // now to ensure the destination Column HWNDs exist before showing them.
        self.reconcile(cx);

        let part_windows = self
            .registry
            .members
            .iter()
            .filter(|(member, entry)| {
                matches!(member, WindowGridMember::Column(_))
                    && entry.host_group.is_some_and(|owner| expected_groups.contains(&owner))
            })
            .map(|(member, entry)| {
                (*member, entry.host_group, entry.window, entry.native_window, entry.visibility)
            })
            .collect::<Vec<_>>();
        let has_typed_part_windows = !part_windows.is_empty();
        let mut ready_groups = HashSet::new();
        let mut all_windows_ready = true;

        for (member, owner, window, native_window, visibility) in part_windows {
            let Some(native_window) = native_window else {
                all_windows_ready = false;
                continue;
            };
            if visibility == PanelWindowVisibility::Hidden {
                if let Err(error) = self.native_ops.show_no_activate(native_window) {
                    tracing::warn!(
                        ?group,
                        ?member,
                        %error,
                        "could not show a Floating Column/Group host during DragHost handoff"
                    );
                    all_windows_ready = false;
                    continue;
                }
                if let Some(entry) = self.registry.members.get_mut(&member) {
                    entry.visibility = PanelWindowVisibility::Visible;
                }
            }

            self.request_member_redraw(member);
            if let Err(error) = window.notify_host(cx) {
                tracing::warn!(
                    ?group,
                    ?member,
                    %error,
                    "could not notify a Floating Column HWND after DragHost handoff"
                );
                all_windows_ready = false;
                continue;
            }
            if let Some(owner) = owner {
                ready_groups.insert(owner);
            }
        }

        let all_destinations_ready = has_typed_part_windows
            && all_windows_ready
            && expected_groups
                .iter()
                .all(|expected| ready_groups.contains(expected));
        if all_destinations_ready {
            cx.notify();
        }
        all_destinations_ready
    }

    fn request_member_redraw(&self, member: WindowGridMember) {
        if let Some(native_window) = self
            .registry
            .members
            .get(&member)
            .and_then(|entry| entry.native_window)
            && let Err(error) = self.native_ops.request_redraw(native_window)
        {
            tracing::debug!(?member, %error, "could not request a Floating member redraw");
        }
    }

    /// Notifies and invalidates every visible Floating Column host.
    pub(crate) fn redraw_visible_hosts(&self, cx: &mut Context<Self>) {
        let member_hosts = self
            .registry
            .members
            .iter()
            .filter(|(_, entry)| entry.visibility == PanelWindowVisibility::Visible)
            .map(|(member, entry)| (*member, entry.window))
            .collect::<Vec<_>>();

        for (member, window) in member_hosts {
            if let Err(error) = window.notify_host(cx) {
                tracing::debug!(?member, %error, "could not notify a visible Floating Column host");
            }
            self.request_member_redraw(member);
        }
    }

    /// Restores Main as the foreground window after a native Floating WindowChain
    /// has been docked into it. Reconciliation first hides the former Floating hosts.
    ///
    /// Only reclaim focus while it still belongs to the just-docked chain; a window
    /// activated by the user after release must remain in the foreground.
    fn schedule_main_activation_after_window_chain_dock(
        &self,
        moving_windows: Vec<NativeWindowId>,
        foreground_at_release: Option<NativeWindowId>,
        cx: &mut Context<Self>,
    ) {
        let manager = cx.entity().downgrade();
        cx.spawn(async move |_, app| {
            // schedule_reconcile uses a 1 ms deferred update. Leave enough time
            // for the removed Floating hosts to be hidden before raising Main.
            app.background_executor()
                .timer(Duration::from_millis(32))
                .await;

            let _ = manager.update(app, |manager, _| {
                let Some(main_window) = manager.main_native_window else {
                    return;
                };
                let foreground = match manager.native_ops.foreground_window() {
                    Ok(foreground) => foreground,
                    Err(error) => {
                        tracing::debug!(
                            %error,
                            "could not inspect foreground window after Floating chain docking"
                        );
                        return;
                    }
                };

                if foreground == Some(main_window) {
                    return;
                }
                if !chain_drop_allows_main_activation(
                    foreground,
                    main_window,
                    foreground_at_release,
                    &moving_windows,
                ) {
                    return;
                }

                if let Err(error) = manager.native_ops.activate(main_window) {
                    tracing::warn!(
                        %error,
                        "could not activate Main after Floating WindowChain docking"
                    );
                }
            });
        })
        .detach();
    }

    /// Restores Main after a Floating tab has been dropped into it.
    ///
    /// Wait until the source Floating HWND and its DragHost have been hidden,
    /// then restore Main only while focus still belongs to this drag handoff.
    pub(crate) fn schedule_main_activation_after_drag_host_drop(
        &self,
        closing_drag_host: NativeWindowId,
        foreground_at_release: Option<NativeWindowId>,
        cx: &mut Context<Self>,
    ) {
        let manager = cx.entity().downgrade();
        cx.spawn(async move |_, app| {
            app.background_executor()
                .timer(Duration::from_millis(32))
                .await;

            let _ = manager.update(app, |manager, _| {
                let Some(main_window) = manager.main_native_window else {
                    return;
                };
                let foreground = match manager.native_ops.foreground_window() {
                    Ok(foreground) => foreground,
                    Err(error) => {
                        tracing::debug!(
                            %error,
                            "could not inspect foreground window after Floating tab dock"
                        );
                        return;
                    }
                };

                if foreground == Some(main_window) {
                    return;
                }
                if foreground.is_some()
                    && foreground != Some(closing_drag_host)
                    && foreground != foreground_at_release
                {
                    // A different window became foreground after release.
                    return;
                }

                if let Err(error) = manager.native_ops.activate(main_window) {
                    tracing::warn!(
                        %error,
                        "could not activate Main after Floating tab dock"
                    );
                }
            });
        })
        .detach();
    }

    /// Activates the persistent destination after its temporary DragHost has closed.
    ///
    /// Closing the owned DragHost can restore Main or the window that was
    /// foreground at release. Activate the destination while focus is unchanged;
    /// preserve a different focus choice made after release.
    pub(crate) fn activate_handoff_target(
        &self,
        group: FloatingGroupId,
        closing_drag_host: NativeWindowId,
        foreground_at_release: Option<NativeWindowId>,
    ) -> bool {
        let native_window = self
            .column_members_by_group
            .get(&group)
            .into_iter()
            .flatten()
            .filter(|member| {
                matches!(member, WindowGridMember::Column(_))
            })
            .find_map(|member| {
                self.registry.members.get(member).and_then(|entry| {
                    (entry.host_group == Some(group)
                        && entry.visibility == PanelWindowVisibility::Visible)
                        .then_some(entry.native_window)
                        .flatten()
                })
            })
;
        let Some(native_window) = native_window else {
            return false;
        };

        let foreground = match self.native_ops.foreground_window() {
            Ok(foreground) => foreground,
            Err(error) => {
                tracing::debug!(
                    ?group,
                    %error,
                    "could not inspect foreground window after DragHost close"
                );
                return false;
            }
        };

        if foreground == Some(native_window) {
            return true;
        }
        if foreground == Some(closing_drag_host) {
            return false;
        }
        if !handoff_foreground_allows_activation(
            foreground,
            native_window,
            self.main_native_window,
            foreground_at_release,
        ) {
            // A different window became foreground after mouse release. Respect
            // that explicit user action instead of reclaiming focus.
            return true;
        }

        if let Err(error) = self.native_ops.activate(native_window) {
            tracing::warn!(
                ?group,
                %error,
                "could not activate a Floating Column HWND after DragHost close"
            );
            return false;
        }

        matches!(
            self.native_ops.foreground_window(),
            Ok(Some(foreground)) if foreground == native_window
        )
    }

    /// Returns the transferred active panels belonging to one ready Column host
    /// or transitional Group host. The host calls this during render before
    /// scheduling a next-frame handoff confirmation.
    pub(crate) fn pending_drag_host_handoff_panels(
        &self,
        group: FloatingGroupId,
        window_id: WindowId,
        cx: &mut Context<Self>,
    ) -> Option<Vec<PanelId>> {
        let host_node = self.registry.members.iter().find_map(|(member, entry)| {
            match member {
                WindowGridMember::Column(column_node)
                    if entry.window_id == window_id
                        && entry.host_group == Some(group)
                        && entry.visibility == PanelWindowVisibility::Visible =>
                {
                    Some(*column_node)
                }
                _ => None,
            }
        });

        if let Some(host_node) = host_node {
            let host_panels = self.controller.update(cx, |controller, _| {
                let mut panels = Vec::new();
                let mut seen = HashSet::new();
                collect_panel_order(controller.layout(), host_node, &mut panels, &mut seen);
                panels
            });
            let drag_host = self.drag_host_manager.as_ref()?.upgrade()?;
            let required = drag_host.update(cx, |manager, _| {
                manager.pending_handoff_panels(group)
            })?;
            let local_required = required
                .into_iter()
                .filter(|panel| host_panels.contains(panel))
                .collect::<Vec<_>>();
            return Some(local_required);
        }

        None
    }

    /// Called after one destination host has rendered the transferred active
    /// PanelViews belonging to its Column or Group subtree. DragHostManager closes
    /// only after every required host across all detached FloatingGroups has rendered.
    pub(crate) fn notify_drag_host_handoff_rendered(
        &mut self,
        group: FloatingGroupId,
        window_id: WindowId,
        rendered_panels: &[PanelId],
        cx: &mut Context<Self>,
    ) {
        let column_member = self.registry.members.iter().find_map(|(member, entry)| {
            match member {
                WindowGridMember::Column(_)
                    if entry.window_id == window_id
                        && entry.host_group == Some(group)
                        && entry.visibility == PanelWindowVisibility::Visible =>
                {
                    Some(*member)
                }
                _ => None,
            }
        });

        if let Some(drag_host) = self.drag_host_manager.as_ref().and_then(|manager| manager.upgrade()) {
            let _ = drag_host.update(cx, |manager, manager_cx| {
                manager.complete_floating_handoff(group, rendered_panels, manager_cx);
            });
        }

        let Some(member) = column_member else {
            return;
        };
        self.request_member_redraw(member);
    }

    fn create_column_window(
        &mut self,
        spec: FloatingColumnWindowSnapshot,
        cx: &mut Context<Self>,
    ) -> Option<PanelWindowEntry> {
        let WindowGridMember::Column(column) = spec.member else {
            return None;
        };
        let controller = self.controller.clone();
        let theme = self.theme;
        let manager = cx.entity().downgrade();
        let close_button_visible = Rc::new(Cell::new(false));
        let host_close_button_visible = close_button_visible.clone();
        let logical_origin = self.gpui_origin_for_bounds(spec.bounds);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(logical_origin.x), px(logical_origin.y)),
                size(px(spec.bounds.size.width), px(spec.bounds.size.height)),
            ))),
            titlebar: None,
            focus: false,
            show: false,
            kind: WindowKind::Floating,
            // Column windows are positioned and resized by FloatingWindowManager.
            // Axis::X splitter input uses the shared Core interaction pipeline.
            is_movable: false,
            app_owns_titlebar_drag: false,
            is_resizable: spec.main_drag_side.is_none(),
            is_minimizable: false,
            ..Default::default()
        };

        let window = match cx.open_window(options, move |_, app| {
            let host = if let Some(side) = spec.main_drag_side {
                FloatingColumnHost::new_main_drag(controller, side, column, theme, manager, host_close_button_visible.clone())
            } else {
                FloatingColumnHost::new_column(controller, spec.floating_group(), column, theme, manager, host_close_button_visible.clone())
            };
            app.new(|_| host)
        }) {
            Ok(window) => window,
            Err(error) => {
                tracing::error!(
                    ?spec.member,
                    %error,
                    "GPUI could not create a Floating Column window"
                );
                return None;
            }
        };

        let window_id = match window.update(cx, |_, window, _| {
            window.window_handle().window_id()
        }) {
            Ok(window_id) => window_id,
            Err(error) => {
                tracing::error!(
                    ?spec.member,
                    %error,
                    "could not inspect the new Floating Column window identifier"
                );
                let _ = window.update(cx, |_, window, _| window.remove_window());
                return None;
            }
        };

        let native_ops = self.native_ops;
        let native_window = match window.update(cx, move |_, window, _| {
            native_ops.hwnd_of(window)
        }) {
            Ok(Ok(native_window)) => Some(native_window),
            Ok(Err(error)) => {
                tracing::error!(
                    ?spec.member,
                    %error,
                    "Floating Column window has no native handle"
                );
                let _ = window.update(cx, |_, window, _| window.remove_window());
                return None;
            }
            Err(error) => {
                tracing::error!(
                    ?spec.member,
                    %error,
                    "could not inspect a newly created Floating Column window"
                );
                let _ = window.update(cx, |_, window, _| window.remove_window());
                return None;
            }
        };

        let Some(native_window) = native_window else {
            let _ = window.update(cx, |_, window, _| window.remove_window());
            return None;
        };
        let Some(owner) = self.main_native_window else {
            tracing::error!(
                ?spec.member,
                "could not create a Floating Column window without the main native owner"
            );
            let _ = window.update(cx, |_, window, _| window.remove_window());
            return None;
        };
        if let Err(error) = self.native_ops.set_owner(native_window, owner) {
            tracing::error!(
                ?spec.member,
                %error,
                "could not assign the main window as Floating Column owner"
            );
            let _ = window.update(cx, |_, window, _| window.remove_window());
            return None;
        }
        if !hide_from_taskbar(native_window) {
            tracing::warn!(
                ?spec.member,
                "could not hide a Floating Column window from the taskbar"
            );
        }

        let scale = self
            .native_ops
            .scale_factor(native_window)
            .ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0);
        let outer_bounds = if spec.main_drag_side.is_some() {
            spec.bounds
        } else {
            self.column_outer_bounds_for_target(spec, native_window, scale)
            .unwrap_or(spec.bounds)
        };
        self.sync_column_window_bounds(native_window, outer_bounds);

        Some(PanelWindowEntry {
            window: PanelWindowHandle::Column(window),
            window_id,
            native_window: Some(native_window),
            bounds: outer_bounds,
            native_min_size: None,
            native_horizontal_resize_min_widths: None,
            native_left_resize_enabled: None,
            native_bottom_resize_enabled: None,
            native_right_resize_enabled: None,
            native_top_resize_enabled: None,
            native_scale_factor: scale,
            native_bounds_dirty: false,
            native_vertical_origin_offset: 0.0,
            native_vertical_height: None,
            core_vertical_height: spec.bounds.size.height,
            resize_enabled: false,
            host_group: if spec.main_drag_side.is_some() {
                None
            } else {
                Some(spec.floating_group())
            },
            visibility: PanelWindowVisibility::Hidden,
            close_button_visible: Some(close_button_visible),
        })
    }

    fn sync_column_window_bounds(&self, native_window: NativeWindowId, bounds: Rect) {
        let Some(position) = self.physical_origin_for_bounds(bounds) else {
            return;
        };
        let scale = self
            .native_ops
            .scale_factor(native_window)
            .ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        let Some(size) = DockGeometry::physical_size_from_logical(bounds.size, scale) else {
            return;
        };
        if let Err(error) = self.native_ops.set_window_bounds(native_window, position, size) {
            tracing::debug!(
                ?native_window,
                %error,
                "could not synchronize Floating Column native bounds"
            );
        }
    }

    fn release_member_window(
        &mut self,
        member: WindowGridMember,
        cx: &mut Context<Self>,
    ) {
        let Some(mut entry) = self.registry.members.remove(&member) else {
            return;
        };

        if let Some(native_window) = entry.native_window {
            if entry.native_horizontal_resize_min_widths.is_some() {
                let _ = self.native_ops.set_horizontal_resize_min_widths(native_window, None);
                entry.native_horizontal_resize_min_widths = None;
            }
            if entry.resize_enabled {
                let _ = self.native_ops.disable_resize(native_window);
                entry.resize_enabled = false;
            }
            if entry.native_min_size.is_some() {
                let _ = self.native_ops.set_min_size(native_window, None);
                entry.native_min_size = None;
            }
            let _ = self.native_ops.hide(native_window);
        }

        entry.window.remove_window(cx);
    }

    /// Detaches one selected Column stack from every previous WindowGrid neighbour.
    /// Rebuilds internal Column edges from Core and preserves only links
    /// that were external both before and after the current Layout transition.
    fn rebuild_floating_column_window_grid(
        &mut self,
        desired: &[FloatingColumnWindowSnapshot],
        previous_column_members_by_group: &HashMap<FloatingGroupId, Vec<WindowGridMember>>,
    ) -> bool {
        let previous_links = self.window_chains.all_links();
        let previous_owners = previous_column_members_by_group
            .iter()
            .flat_map(|(group, members)| members.iter().map(move |member| (*member, *group)))
            .collect::<HashMap<_, _>>();
        let mut owners = HashMap::<WindowGridMember, FloatingGroupId>::new();
        for spec in desired {
            owners.insert(spec.member, spec.floating_group());
        }

        let mut links = floating_column_internal_links(desired);
        for previous in previous_links {
            if !previous_window_grid_link_is_external(previous, &previous_owners, &owners) {
                // Internal Column edges are regenerated from the current
                // Core layout; keep only relationships between compositions.
                continue;
            }

            let from = match previous.from {
                WindowGridMember::FloatingComposition(group) => {
                    floating_column_boundary_member(desired, group, previous.direction)
                }
                member => owners.contains_key(&member).then_some(member),
            };
            let to = match previous.to {
                WindowGridMember::FloatingComposition(group) => {
                    floating_column_boundary_member(desired, group, previous.direction.opposite())
                }
                member => owners.contains_key(&member).then_some(member),
            };
            let (Some(from), Some(to)) = (from, to) else {
                continue;
            };
            if from != to {
                links.push(WindowGridLink {
                    from,
                    to,
                    direction: previous.direction,
                });
            }
        }

        let member_order = desired
            .iter()
            .enumerate()
            .map(|(index, spec)| (spec.member, index))
            .collect::<HashMap<_, _>>();
        links.sort_by_key(|link| {
            (
                member_order.get(&link.from).copied().unwrap_or(usize::MAX),
                member_order.get(&link.to).copied().unwrap_or(usize::MAX),
                match link.direction {
                    WindowGridDirection::Left => 0u8,
                    WindowGridDirection::Right => 1,
                    WindowGridDirection::Above => 2,
                    WindowGridDirection::Below => 3,
                },
            )
        });

        if self.window_chains.replace_all_links(&links) {
            true
        } else {
            tracing::warn!(
                "could not rebuild WindowGrid from the current Floating Column topology"
            );
            false
        }
    }

    fn reconcile_column_windows(
        &mut self,
        transferred_panels: &[PanelId],
        cx: &mut Context<Self>,
    ) -> crate::window_chain::WindowGridPruneResult {
        let scale = self
            .main_native_window
            .and_then(|main| self.native_ops.scale_factor(main).ok())
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        let transferred_panels = transferred_panels.iter().copied().collect::<HashSet<_>>();
        let desired = self.controller.update(cx, |controller, _| {
            collect_floating_column_windows(
                controller.layout(),
                &controller.metrics(),
                scale,
                &transferred_panels,
            )
        });
        let desired_members = desired
            .iter()
            .map(|spec| spec.member)
            .collect::<HashSet<_>>();
        let unavailable_members = self
            .registry
            .members
            .iter()
            .filter_map(|(member, entry)| {
                let native_available = entry
                    .native_window
                    .is_some_and(|window| self.native_ops.window_rect(window).is_ok());
                (!native_available).then_some(*member)
            })
            .collect::<Vec<_>>();
        let stale_members = self
            .registry
            .members
            .keys()
            .filter(|member| !desired_members.contains(member))
            .copied()
            .chain(unavailable_members)
            .collect::<HashSet<_>>();
        for member in stale_members {
            self.release_member_window(member, cx);
        }

        let mut members_by_group = HashMap::<FloatingGroupId, Vec<WindowGridMember>>::new();
        for spec in &desired {
            members_by_group
                .entry(spec.floating_group())
                .or_default()
                .push(spec.member);
        }
        let previous_column_members_by_group =
            std::mem::replace(&mut self.column_members_by_group, members_by_group);

        for spec in desired.iter().copied() {
            if !self.registry.members.contains_key(&spec.member) {
                if let Some(entry) = self.create_column_window(spec, cx) {
                    self.registry.members.insert(spec.member, entry);
                } else {
                    continue;
                }
            }

            // A persistent Column HWND may survive a Core Float operation
            // while its FloatingGroup owner changes. Keep the registry owner in sync
            // with the current Layout snapshot instead of the original creation owner.
            if let Some(entry) = self.registry.members.get_mut(&spec.member) {
                entry.host_group = Some(spec.floating_group());
            }

            // Persistent Core height changes (Group splitter, Undo, or Redo)
            // replace a previous native resize override for permanent Columns.
            let core_height_changed = if spec.main_drag_side.is_none()
                && matches!(spec.member, WindowGridMember::Column(_))
            {
                if let Some(entry) = self.registry.members.get_mut(&spec.member) {
                    if (entry.core_vertical_height - spec.bounds.size.height).abs() > 0.5 {
                        entry.core_vertical_height = spec.bounds.size.height;
                        entry.native_vertical_height = Some(spec.bounds.size.height);
                        entry.native_bounds_dirty = true;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };

            let current = self.registry.members.get(&spec.member).map(|entry| {
                (entry.bounds, entry.native_bounds_dirty, entry.native_window, entry.native_scale_factor)
            });
            let Some((current_bounds, native_bounds_dirty, native_window, previous_scale)) = current else {
                continue;
            };
            let current_scale = native_window
                .and_then(|native_window| self.native_ops.scale_factor(native_window).ok())
                .filter(|scale| scale.is_finite() && *scale > 0.0)
                .or(previous_scale);
            if core_height_changed
                && let WindowGridMember::Column(column) = spec.member
            {
                let viewport = self.registry.members.get(&spec.member).map(|entry| {
                    (
                        entry.window_id,
                        entry.native_vertical_origin_offset,
                    )
                });
                if let Some((window_id, origin_y_offset)) = viewport {
                    let scale = current_scale.unwrap_or(1.0);
                    self.controller.update(cx, |controller, controller_cx| {
                        controller.set_floating_column_viewport(
                            window_id,
                            spec.floating_group(),
                            column,
                            spec.bounds.size.height,
                            origin_y_offset / scale,
                        );
                        controller_cx.notify();
                    });
                }
            }
            let mut target_spec = spec;
            let mut target_vertical_origin_offset = None;
            if let Some(entry) = self.registry.members.get(&spec.member)
                && let Some(height) = entry.native_vertical_height
            {
                let stored_scale = entry.native_scale_factor.unwrap_or(1.0);
                let target_scale = current_scale.unwrap_or(stored_scale);
                let logical_offset = entry.native_vertical_origin_offset / stored_scale;
                let physical_offset = logical_offset * target_scale;
                target_spec.bounds.size.height = height;
                target_spec.floating_origin.y += physical_offset;
                target_vertical_origin_offset = Some(physical_offset);
            }
            let target_bounds = native_window
                .and_then(|native_window| {
                    self.column_outer_bounds_for_target(
                        target_spec,
                        native_window,
                        current_scale,
                                )
                })
                .unwrap_or_else(|| {
                    if native_window.is_some() { current_bounds } else { target_spec.bounds }
                });
            let core_width_changed =
                (current_bounds.size.width - spec.bounds.size.width).abs() > 0.5;
            let resizing_this_member = self.native_resize.is_some_and(|session| {
                session.group == spec.floating_group() && session.member == spec.member
            });
            let actual_native_client_width = native_window
                .and_then(|native_window| self.core_bounds_from_native_window(native_window))
                .map(|bounds| bounds.size.width);
            if actual_native_client_width.is_some_and(|actual| {
                (actual - target_bounds.size.width).abs() > 0.5
            }) {
                eprintln!(
                    "[FLOATING_RESIZE_DIAG] event=reconcile_width_mismatch group={:?} member={:?} hwnd_client_width={:?} core_column_width={:.1} reconcile_target_width={:.1} registry_width={:.1} native_bounds_dirty={} resizing_this_member={}",
                    spec.floating_group(),
                    spec.member,
                    actual_native_client_width,
                    spec.bounds.size.width,
                    target_bounds.size.width,
                    current_bounds.size.width,
                    native_bounds_dirty,
                    resizing_this_member,
                );
            }
            let scale_changed = current_scale.is_some_and(|scale| {
                previous_scale.is_none_or(|previous| (previous - scale).abs() > f32::EPSILON)
            });
            if current_bounds != target_bounds || native_bounds_dirty || scale_changed {
                if !resizing_this_member
                    && let Some(native_window) = native_window
                {
                    self.sync_column_window_bounds(native_window, target_bounds);
                }
                if let Some(entry) = self.registry.members.get_mut(&spec.member) {
                    entry.bounds = target_bounds;
                    if let Some(offset) = target_vertical_origin_offset {
                        entry.native_vertical_origin_offset = offset;
                    }
                    entry.native_scale_factor = current_scale;
                    entry.native_bounds_dirty = resizing_this_member;
                }
            }

            // Core-driven resize changes the HWND outside GPUI's own window
            // layout loop. Refresh GPUI's cached client bounds for either axis;
            // otherwise the native window can change size while its rendered
            // viewport keeps the previous client dimensions.
            if core_width_changed || core_height_changed {
                eprintln!(
                    "[FLOATING_RESIZE_DIAG] event=refresh_gpui_bounds group={:?} member={:?} registry_width={:.1} core_width={:.1} width_changed={} height_changed={}",
                    spec.floating_group(),
                    spec.member,
                    current_bounds.size.width,
                    spec.bounds.size.width,
                    core_width_changed,
                    core_height_changed,
                );
                if let Some(window) = self.registry.members.get(&spec.member).map(|entry| entry.window) {
                    window.refresh_bounds_later(cx);
                }
            }

            let native_window = self
                .registry
                .members
                .get(&spec.member)
                .and_then(|entry| entry.native_window);
            let visibility = self.registry.members
                .get(&spec.member)
                .map(|entry| entry.visibility);
            match (spec.contains_transferred_panels, visibility, native_window) {
                (false, Some(PanelWindowVisibility::Hidden), Some(native_window)) => {
                    match self.native_ops.show_no_activate(native_window) {
                        Ok(()) => {
                            if let Some(entry) = self.registry.members.get_mut(&spec.member) {
                                entry.visibility = PanelWindowVisibility::Visible;
                            }
                        }
                        Err(error) => tracing::warn!(
                            ?spec.member,
                            %error,
                            "could not show a Floating Column window"
                        ),
                    }
                }
                (true, Some(PanelWindowVisibility::Visible), Some(native_window)) => {
                    if let Err(error) = self.native_ops.hide(native_window) {
                        tracing::warn!(
                            ?spec.member,
                            %error,
                            "could not hide a Floating Column while its PanelViews are transported"
                        );
                    } else if let Some(entry) = self.registry.members.get_mut(&spec.member) {
                        entry.visibility = PanelWindowVisibility::Hidden;
                    }
                }
                _ => {}
            }
        }

        // Retain legacy FloatingComposition graph endpoints for this pass so
        // their external links can be migrated to real Column boundaries.
        // A Column may have an independent native Y origin and height, so boundary
        // selection must use its current viewport rather than Core's shared rect.
        let boundary_specs = desired
            .iter()
            .copied()
            .map(|mut spec| {
                if let Some(entry) = self.registry.members.get(&spec.member) {
                    let scale = entry
                        .native_scale_factor
                        .filter(|scale| scale.is_finite() && *scale > 0.0)
                        .unwrap_or(1.0);
                    spec.local_origin.y += entry.native_vertical_origin_offset / scale;
                    if let Some(height) = entry.native_vertical_height {
                        spec.bounds.size.height = height;
                    }
                }
                spec
            })
            .collect::<Vec<_>>();

        // Stale typed members are still pruned first, allowing WindowGrid to
        // produce straight-through candidates before the Core topology is rebuilt.
        let mut live_members = self.registry.members.keys().copied().collect::<HashSet<_>>();
        let live_floating_groups = desired
            .iter()
            .map(|spec| spec.floating_group())
            .collect::<HashSet<_>>();
        for link in self.window_chains.all_links() {
            for member in [link.from, link.to] {
                if let WindowGridMember::FloatingComposition(group) = member
                    && live_floating_groups.contains(&group)
                {
                    live_members.insert(member);
                }
            }
        }
        let prune_result = self.window_chains.retain_members(&live_members);
        let stale_alignment_anchors = self
            .pending_window_chain_alignment
            .iter()
            .filter(|anchor| {
                !live_members.contains(anchor) || !self.window_chains.is_chained(**anchor)
            })
            .copied()
            .collect::<Vec<_>>();
        for anchor in stale_alignment_anchors {
            self.pending_window_chain_alignment.remove(&anchor);
        }
        self.pending_window_chain_alignment
            .extend(prune_result.components_to_align.iter().copied());

        // Pending requests may still carry legacy composition identities from
        // register_window_chain or a previous prune. Rebuild first, then migrate
        // those identities to the current Column boundary members.
        if self.rebuild_floating_column_window_grid(&boundary_specs, &previous_column_members_by_group) {
            self.migrate_pending_window_grid_state();
            let rebuilt_links = self.window_chains.all_links().into_iter().collect::<HashSet<_>>();
            self.pending_window_grid_reconnections
                .retain(|candidate| !rebuilt_links.contains(candidate));
        }

        // Native resizing belongs to each persistent Column HWND. Configure it
        // only after the WindowGrid links are rebuilt, so shared edges are disabled
        // on both sides of every internal Axis::X boundary.
        let resize_columns = desired
            .iter()
            .filter_map(|spec| match spec.member {
                WindowGridMember::Column(column) => {
                    Some((spec.member, spec.floating_group(), column))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for (member, group, column) in resize_columns {
            let Some(mut entry) = self.registry.members.remove(&member) else {
                continue;
            };
            if entry.visibility == PanelWindowVisibility::Visible {
                self.sync_native_column_resize(&mut entry, group, column, cx);
            }
            self.registry.members.insert(member, entry);
        }

        // Permanent Column hosts share their close-button state with the manager,
        // so changing the owner never updates a FloatingColumnHost Entity.
        let column_windows = self
            .registry
            .members
            .iter()
            .filter_map(|(member, entry)| {
                let WindowGridMember::Column(column) = member else {
                    return None;
                };
                Some((
                    *member,
                    *column,
                    entry.window,
                    entry.close_button_visible.clone(),
                ))
            })
            .collect::<Vec<_>>();
        for (member, column, window, close_button_visible) in column_windows {
            let show_close_button = self.is_rightmost_column(column);
            let state_changed = close_button_visible
                .is_some_and(|state| state.replace(show_close_button) != show_close_button);
            if state_changed {
                // Refresh without leasing FloatingColumnHost; it may be handling the
                // same drag/resize event that caused this reconciliation.
                if let Err(error) = window.notify_host(cx) {
                    tracing::debug!(?column, %error, "could not refresh the close-button owner");
                }
                self.request_member_redraw(member);
            }
            if let Err(error) = window.set_column_close_button(show_close_button, cx) {
                tracing::debug!(
                    ?column,
                    %error,
                    "could not update a promoted DragHost close-button owner"
                );
            }
        }

        prune_result
    }

    fn sync_native_column_resize_constraints(
        &mut self,
        column: NodeId,
        group: FloatingGroupId,
        window_id: WindowId,
        cx: &mut Context<Self>,
    ) {
        let member = WindowGridMember::Column(column);
        let Some(mut entry) = self.registry.members.remove(&member) else {
            return;
        };
        if entry.window_id != window_id || entry.host_group != Some(group) {
            self.registry.members.insert(member, entry);
            return;
        }
        self.sync_native_column_resize(&mut entry, group, column, cx);
        self.registry.members.insert(member, entry);
    }

    fn sync_native_column_resize(
        &self,
        entry: &mut PanelWindowEntry,
        group: FloatingGroupId,
        column: NodeId,
        cx: &mut Context<Self>,
    ) {
        let Some(native_window) = entry.native_window else {
            return;
        };
        if !entry.resize_enabled {
            match self.native_ops.enable_resize(native_window) {
                Ok(()) => entry.resize_enabled = true,
                Err(error) => {
                    tracing::warn!(
                        ?group,
                        ?column,
                        %error,
                        "could not enable native Floating Column resize"
                    );
                    return;
                }
            }
        }

        let member = WindowGridMember::Column(column);
        let (is_first_column, is_last_column, has_multiple_columns) =
            self.controller.update(cx, |controller, _| {
                let root_columns = floating_root_columns(controller.layout(), group);
                let is_first = root_columns
                    .as_ref()
                    .and_then(|columns| columns.first())
                    .is_some_and(|first| *first == column);
                let is_last = root_columns
                    .as_ref()
                    .and_then(|columns| columns.last())
                    .is_some_and(|last| *last == column);
                let has_multiple = root_columns.is_some_and(|columns| columns.len() > 1);
                (is_first, is_last, has_multiple)
            });
        let desired_left_enabled = !self
            .window_chains
            .has_neighbor_on_side(member, WindowGridDirection::Left);
        let has_right_neighbor = self
            .window_chains
            .has_neighbor_on_side(member, WindowGridDirection::Right);
        let desired_right_enabled = floating_column_right_resize_enabled(
            is_first_column,
            is_last_column,
            has_multiple_columns,
            has_right_neighbor,
        );
        let desired_top_enabled = !self
            .window_chains
            .has_neighbor_on_side(member, WindowGridDirection::Above);
        let desired_bottom_enabled = !self
            .window_chains
            .has_neighbor_on_side(member, WindowGridDirection::Below);

        if entry.native_left_resize_enabled != Some(desired_left_enabled) {
            match self.native_ops.set_left_resize_enabled(native_window, desired_left_enabled) {
                Ok(()) => entry.native_left_resize_enabled = Some(desired_left_enabled),
                Err(error) => tracing::warn!(
                    ?group,
                    ?column,
                    %error,
                    "could not synchronize the Floating Column left resize edge"
                ),
            }
        }
        // The first Column's right edge is the internal handle; the last
        // Column's right edge remains the outer resize edge when unlinked.
        // Intermediate right edges stay disabled by floating_column_right_resize_enabled.
        if entry.native_right_resize_enabled != Some(desired_right_enabled) {
            match self.native_ops.set_right_resize_enabled(native_window, desired_right_enabled) {
                Ok(()) => entry.native_right_resize_enabled = Some(desired_right_enabled),
                Err(error) => tracing::warn!(
                    ?group,
                    ?column,
                    %error,
                    "could not synchronize the Floating Column right resize edge"
                ),
            }
        }
        if entry.native_top_resize_enabled != Some(desired_top_enabled) {
            match self.native_ops.set_top_resize_enabled(native_window, desired_top_enabled) {
                Ok(()) => entry.native_top_resize_enabled = Some(desired_top_enabled),
                Err(error) => tracing::warn!(
                    ?group,
                    ?column,
                    %error,
                    "could not synchronize the Floating Column top resize edge"
                ),
            }
        }
        if entry.native_bottom_resize_enabled != Some(desired_bottom_enabled) {
            match self.native_ops.set_bottom_resize_enabled(native_window, desired_bottom_enabled) {
                Ok(()) => entry.native_bottom_resize_enabled = Some(desired_bottom_enabled),
                Err(error) => tracing::warn!(
                    ?group,
                    ?column,
                    %error,
                    "could not synchronize the Floating Column bottom resize edge"
                ),
            }
        }

        let minimum = self.controller.update(cx, |controller, _| {
            let metrics = controller.metrics();
            controller
                .layout()
                .floating_column_minimum_size(group, column, &metrics)
                .ok()
        });
        let scale = self
            .native_ops
            .scale_factor(native_window)
            .ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        let desired_minimum = minimum.and_then(|minimum| {
            DockGeometry::physical_size_from_logical(minimum, scale)
        });
        if entry.native_min_size != desired_minimum {
            match self.native_ops.set_min_size(native_window, desired_minimum) {
                Ok(()) => entry.native_min_size = desired_minimum,
                Err(error) => tracing::warn!(
                    ?group,
                    ?column,
                    %error,
                    "could not synchronize the Floating Column minimum size"
                ),
            }
        }

        let desired_edge_minimums = minimum.and_then(|minimum| {
            let width = DockGeometry::physical_coordinate(minimum.width, scale)?;
            Some((width, width))
        });
        if entry.native_horizontal_resize_min_widths != desired_edge_minimums {
            match self.native_ops.set_horizontal_resize_min_widths(native_window, desired_edge_minimums) {
                Ok(()) => entry.native_horizontal_resize_min_widths = desired_edge_minimums,
                Err(error) => tracing::warn!(
                    ?group,
                    ?column,
                    %error,
                    "could not synchronize Floating Column horizontal minimum widths"
                ),
            }
        }
    }
    
}

fn chain_drop_allows_main_activation(
    foreground: Option<NativeWindowId>,
    main_window: NativeWindowId,
    foreground_at_release: Option<NativeWindowId>,
    moving_windows: &[NativeWindowId],
) -> bool {
    foreground.is_none()
        || foreground == Some(main_window)
        || foreground == foreground_at_release
        || foreground.is_some_and(|window| moving_windows.contains(&window))
}

fn handoff_foreground_allows_activation(
    foreground: Option<NativeWindowId>,
    target_window: NativeWindowId,
    main_window: Option<NativeWindowId>,
    foreground_at_release: Option<NativeWindowId>,
) -> bool {
    foreground.is_none()
        || foreground == Some(target_window)
        || foreground == main_window
        || foreground == foreground_at_release
}

fn is_managed_foreground_window(
    foreground: NativeWindowId,
    main_window: Option<NativeWindowId>,
    floating_windows: impl IntoIterator<Item = Option<NativeWindowId>>,
    drag_host: Option<NativeWindowId>,
) -> bool {
    main_window == Some(foreground)
        || floating_windows
            .into_iter()
            .flatten()
            .any(|window| window == foreground)
        || drag_host == Some(foreground)
}

fn collect_panel_order(
    layout: &dock_core::Layout,
    root: NodeId,
    panels: &mut Vec<PanelId>,
    seen: &mut HashSet<PanelId>,
) {
    match layout.node(root) {
        Some(Node::Split(split)) => {
            for child in &split.children {
                collect_panel_order(layout, *child, panels, seen);
            }
        }
        Some(Node::Column(column)) => {
            for group in &column.groups {
                collect_panel_order(layout, *group, panels, seen);
            }
        }
        Some(Node::Tabs(tabs)) => {
            for child in &tabs.children {
                collect_panel_order(layout, *child, panels, seen);
            }
        }
        Some(Node::Panel(panel)) => {
            if seen.insert(panel.panel_id) {
                panels.push(panel.panel_id);
            }
        }
        None => {}
    }
}

#[cfg(test)]
fn native_rect_has_reachable_title_area(
    rect: platform::PhysicalRect,
    work_areas: &[platform::PhysicalRect],
) -> bool {
    let width = rect.right.saturating_sub(rect.left).max(0);
    let height = rect.bottom.saturating_sub(rect.top).max(0);
    if width == 0 || height == 0 {
        return false;
    }

    work_areas.iter().any(|area| {
        let required_width = width.min(80).min(area.width().max(0));
        let required_height = height.min(40).min(area.height().max(0));
        let visible_width = rect.right.min(area.right).saturating_sub(rect.left.max(area.left));
        let visible_height = rect.top.saturating_add(required_height)
            .min(area.bottom)
            .saturating_sub(rect.top.max(area.top));
        visible_width >= required_width && visible_height >= required_height
    })
}

#[cfg(test)]
fn restored_window_origin(
    rect: platform::PhysicalRect,
    work_area: platform::PhysicalRect,
) -> Option<PhysicalPoint> {
    let width = rect.right.checked_sub(rect.left)?;
    let height = rect.bottom.checked_sub(rect.top)?;
    let area_width = work_area.right.checked_sub(work_area.left)?;
    let area_height = work_area.bottom.checked_sub(work_area.top)?;
    if width <= 0 || height <= 0 || area_width <= 0 || area_height <= 0 {
        return None;
    }

    // Keep windows fully inside the work area when they fit. Larger
    // compositions may extend beyond it, but retain a reachable title strip.
    let (minimum_x, maximum_x) = if width <= area_width {
        (
            work_area.left as i64,
            work_area.right as i64 - width as i64,
        )
    } else {
        (
            work_area.left as i64 - width as i64 + 80,
            work_area.right as i64 - 80,
        )
    };
    let (minimum_y, maximum_y) = if height <= area_height {
        (
            work_area.top as i64,
            work_area.bottom as i64 - height as i64,
        )
    } else {
        (
            work_area.top as i64,
            work_area.bottom as i64 - 40,
        )
    };
    let x = (rect.left as i64).clamp(minimum_x, maximum_x);
    let y = (rect.top as i64).clamp(minimum_y, maximum_y);

    Some(PhysicalPoint {
        x: i32::try_from(x).ok()?,
        y: i32::try_from(y).ok()?,
    })
}

impl Drop for FloatingWindowManager {
    fn drop(&mut self) {
        if self.native_move.is_some_and(|state| state.high_resolution_timer) {
            let _ = self.native_ops.end_high_resolution_timer();
        }
    }
}

#[cfg(test)]
mod focus_tests {
    use super::{
        chain_drop_allows_main_activation, handoff_foreground_allows_activation,
        is_managed_foreground_window,
    };
    use platform::NativeWindowId;

    fn window(raw: isize) -> NativeWindowId {
        NativeWindowId::from_raw(raw).expect("test HWND must be nonzero")
    }

    #[test]
    fn managed_main_floating_and_drag_hosts_preserve_cross_window_focus() {
        let main = window(1);
        let floating = window(2);
        let drag_host = window(3);

        assert!(is_managed_foreground_window(
            main,
            Some(main),
            [Some(floating)],
            Some(drag_host),
        ));
        assert!(is_managed_foreground_window(
            floating,
            Some(main),
            [Some(floating)],
            Some(drag_host),
        ));
        assert!(is_managed_foreground_window(
            drag_host,
            Some(main),
            [Some(floating)],
            Some(drag_host),
        ));
    }

    #[test]
    fn foreground_outside_all_managed_hosts_is_a_real_focus_loss() {
        let main = window(1);
        let floating = window(2);
        let drag_host = window(3);
        let external = window(4);

        assert!(!is_managed_foreground_window(
            external,
            Some(main),
            [Some(floating)],
            Some(drag_host),
        ));
    }

    #[test]
    fn chain_drop_can_activate_main_when_a_member_window_remains_foreground() {
        let main = window(1);
        let release_window = window(2);
        let other_chain_window = window(3);

        assert!(chain_drop_allows_main_activation(
            Some(release_window),
            main,
            Some(release_window),
            &[release_window, other_chain_window],
        ));
        assert!(chain_drop_allows_main_activation(
            Some(other_chain_window),
            main,
            Some(release_window),
            &[release_window, other_chain_window],
        ));
    }

    #[test]
    fn chain_drop_does_not_steal_focus_from_a_new_external_window() {
        let main = window(1);
        let release_window = window(2);
        let chain_window = window(3);
        let external = window(4);

        assert!(!chain_drop_allows_main_activation(
            Some(external),
            main,
            Some(release_window),
            &[release_window, chain_window],
        ));
    }

    #[test]
    fn chain_drop_can_activate_main_when_no_foreground_window_is_reported() {
        let main = window(1);

        assert!(chain_drop_allows_main_activation(
            None,
            main,
            None,
            &[],
        ));
    }

    #[test]
    fn handoff_can_activate_when_the_release_window_remains_foreground() {
        let main = window(1);
        let release_window = window(2);
        let target = window(3);

        assert!(handoff_foreground_allows_activation(
            Some(release_window),
            target,
            Some(main),
            Some(release_window),
        ));
    }

    #[test]
    fn handoff_can_activate_when_windows_restores_main_foreground() {
        let main = window(1);
        let target = window(2);

        assert!(handoff_foreground_allows_activation(
            Some(main),
            target,
            Some(main),
            None,
        ));
    }

    #[test]
    fn handoff_does_not_steal_focus_from_a_window_activated_after_release() {
        let main = window(1);
        let release_window = window(2);
        let target = window(3);
        let newly_activated = window(4);

        assert!(!handoff_foreground_allows_activation(
            Some(newly_activated),
            target,
            Some(main),
            Some(release_window),
        ));
    }

    #[test]
    fn handoff_can_activate_when_no_foreground_window_is_reported() {
        let main = window(1);
        let target = window(2);

        assert!(handoff_foreground_allows_activation(
            None,
            target,
            Some(main),
            None,
        ));
    }
}

#[cfg(test)]
mod coordinate_tests {
    use super::{
        floating_member_drag_point, native_rect_has_reachable_title_area,
        restored_window_origin,
    };
    use crate::geometry::DockGeometry;
    use dock_core::{Point, Rect, Size};
    use platform::{PhysicalPoint, PhysicalRect};

    #[test]
    fn off_screen_restored_window_is_placed_inside_the_preferred_work_area() {
        let window = PhysicalRect {
            left: 5000,
            top: 4000,
            right: 5600,
            bottom: 4400,
        };
        let work_area = PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };

        assert_eq!(
            restored_window_origin(window, work_area),
            Some(PhysicalPoint { x: 1320, y: 640 }),
        );
    }

    #[test]
    fn partly_visible_window_keeps_a_reachable_title_strip() {
        let work_area = PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        let window = PhysicalRect {
            left: 1840,
            top: 32,
            right: 2440,
            bottom: 432,
        };

        assert!(native_rect_has_reachable_title_area(window, &[work_area]));
    }

    #[test]
    fn window_below_the_work_area_does_not_count_as_title_reachable() {
        let work_area = PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        let window = PhysicalRect {
            left: 100,
            top: 1050,
            right: 700,
            bottom: 1450,
        };

        assert!(!native_rect_has_reachable_title_area(window, &[work_area]));
    }

    #[test]
    fn main_host_local_bounds_convert_to_virtual_desktop_physical_origin() {
        let screen = DockGeometry::screen_position_from_host_local(
            Point::new(120.0, 56.0),
            Point::new(32.0, 48.0),
            PhysicalPoint { x: 1400, y: 80 },
            1.5,
        ).unwrap();
        assert_eq!(screen, PhysicalPoint { x: 1628, y: 236 });
        assert_eq!(
            DockGeometry::host_local_point_from_screen(
                screen,
                PhysicalPoint { x: 1400, y: 80 },
                Point::new(32.0, 48.0),
                1.5,
            ),
            Some(Point::new(120.0, 56.0)),
        );
    }

    #[test]
    fn main_host_coordinates_round_trip_at_fractional_monitor_dpi() {
        let client_origin = PhysicalPoint { x: -1920, y: 120 };
        let host_origin = Point::new(16.0, 24.0);
        let local = Point::new(113.6, 77.0);
        let scale = 1.25;

        let screen =
            DockGeometry::screen_position_from_host_local(local, host_origin, client_origin, scale).unwrap();
        let restored =
            DockGeometry::host_local_point_from_screen(screen, client_origin, host_origin, scale).unwrap();

        assert!((restored.x - local.x).abs() <= 0.25);
        assert!((restored.y - local.y).abs() <= 0.25);
    }

    #[test]
    fn floating_host_coordinates_use_the_target_windows_client_origin_and_dpi() {
        let client_origin = PhysicalPoint { x: -1920, y: 120 };
        let local = Point::new(120.0, 56.0);
        let scale = 1.5;

        let screen = DockGeometry::screen_position_from_host_local(
            local,
            Point::ZERO,
            client_origin,
            scale,
        ).unwrap();
        assert_eq!(screen, PhysicalPoint { x: -1740, y: 204 });
        assert_eq!(
            DockGeometry::host_local_point_from_screen(
                screen,
                client_origin,
                Point::ZERO,
                scale,
            ),
            Some(local),
        );
    }

    #[test]
    fn floating_member_drag_point_tracks_cursor_beyond_source_window() {
        let point = floating_member_drag_point(
            PhysicalPoint { x: 10, y: 500 },
            PhysicalPoint { x: 100, y: 200 },
            Rect::new(Point::new(300.0, 400.0), Size::new(600.0, 500.0)),
            1.5,
        );

        assert_eq!(point, Some(Point::new(240.0, 600.0)));
    }

    #[test]
    fn screen_coordinate_conversion_rejects_invalid_monitor_scale() {
        assert!(
            DockGeometry::screen_position_from_host_local(
                Point::new(1.0, 1.0),
                Point::ZERO,
                PhysicalPoint::default(),
                0.0,
            )
            .is_none()
        );
        assert!(
            DockGeometry::host_local_point_from_screen(
                PhysicalPoint::default(),
                PhysicalPoint::default(),
                Point::ZERO,
                f32::NAN,
            )
            .is_none()
        );
    }
}

#[cfg(test)]
mod floating_window_chain_drop_destination_tests {
    use super::{
        floating_chain_drop_target_is_supported, floating_column_for_group,
        floating_window_chain_direction, floating_window_drop_destination, NativeMoveTarget,
    };
    use crate::window_chain::{WindowGridDirection, WindowGridMember};
    use dock_core::{
        create_initial_layout, DockCommand, DockMetrics, DockSide, DropTarget, DropZone,
        FloatingGroupId, Node, NodeId, PanelConstraints, PanelKey, Point, Profile, Rect, Size,
    };
    use slotmap::Key;

    #[test]
    fn group_edge_target_resolves_to_its_owning_floating_column() {
        let metrics = DockMetrics::default();
        let constraints = PanelConstraints::new(
            Size::new(80.0, 60.0),
            Size::new(280.0, 220.0),
            None,
            false,
        );
        let descriptors = ["layers", "color", "properties", "history"]
            .into_iter()
            .map(|key| (PanelKey::new(key).unwrap(), constraints))
            .collect::<Vec<_>>();
        let mut layout =
            create_initial_layout(Profile::Photoshop, &descriptors, metrics).unwrap();

        let column = layout.main_area(DockSide::Right).unwrap().root;
        let group = match layout.node(column).unwrap() {
            Node::Column(column) => column.groups[1],
            other => panic!("expected a Column root, got {other:?}"),
        };
        layout
            .apply(
                DockCommand::Float {
                    node: column,
                    bounds: Rect::new(Point::new(120.0, 80.0), Size::new(420.0, 360.0)),
                },
                &metrics,
            )
            .unwrap();
        let (floating, _) = layout.floating_groups().next().unwrap();

        assert_eq!(
            floating_column_for_group(&layout, floating, group),
            Some(column),
        );
    }

    #[test]
    fn floating_window_drop_maps_all_four_zones_to_grid_directions() {
        let group = NodeId::null();
        let left = DropTarget::GroupEdge { group, zone: DropZone::Left };
        let right = DropTarget::GroupEdge { group, zone: DropZone::Right };
        let top = DropTarget::GroupEdge { group, zone: DropZone::Top };
        let bottom = DropTarget::GroupEdge { group, zone: DropZone::Bottom };
        let floating_top = DropTarget::GroupEdge { group, zone: DropZone::FloatingTop };
        let floating_bottom = DropTarget::GroupEdge { group, zone: DropZone::FloatingBottom };
        let center = DropTarget::GroupTabs { group, index: 0 };

        for (target, direction) in [
            (left, WindowGridDirection::Left),
            (right, WindowGridDirection::Right),
            (top, WindowGridDirection::Above),
            (bottom, WindowGridDirection::Below),
            (floating_top, WindowGridDirection::Above),
            (floating_bottom, WindowGridDirection::Below),
        ] {
            assert_eq!(floating_window_chain_direction(target), Some(direction));
            assert!(floating_chain_drop_target_is_supported(target, false));
            assert!(!floating_chain_drop_target_is_supported(target, true));
            let column = NodeId::null();
            assert!(matches!(
                floating_window_drop_destination(
                    FloatingGroupId::null(),
                    target,
                    Point::ZERO,
                    true,
                    Some(column),
                ),
                Some(NativeMoveTarget::FloatingChain {
                    anchor_member: actual_member,
                    direction: actual,
                    ..
                }) if actual == direction
                    && actual_member == WindowGridMember::Column(column)
            ));
        }

        assert_eq!(floating_window_chain_direction(center), None);
        assert!(!floating_chain_drop_target_is_supported(center, false));
        assert!(matches!(
            floating_window_drop_destination(
                FloatingGroupId::null(),
                center,
                Point::ZERO,
                true,
                None,
            ),
            Some(NativeMoveTarget::FloatingDock { target, .. }) if target == center
        ));
        assert!(matches!(
            floating_window_drop_destination(
                FloatingGroupId::null(),
                right,
                Point::ZERO,
                false,
                None,
            ),
            Some(NativeMoveTarget::FloatingDock { target, .. }) if target == right
        ));
        assert!(
            floating_window_drop_destination(
                FloatingGroupId::null(),
                right,
                Point::ZERO,
                true,
                None,
            )
            .is_none()
        );
    }
}
#[cfg(test)]
mod floating_chain_drop_target_tests {
    use super::floating_chain_drop_target_is_supported;
    use dock_core::{DockSide, DropTarget, DropZone, NodeId};
    use slotmap::Key;

    #[test]
    fn complete_window_chains_only_accept_main_column_insertions_on_main() {
        let target = DropTarget::MainColumn {
            side: DockSide::Left,
            index: 1,
        };
        assert!(floating_chain_drop_target_is_supported(target, true));
        assert!(!floating_chain_drop_target_is_supported(target, false));

        let top = DropTarget::GroupEdge {
            group: NodeId::null(),
            zone: DropZone::Top,
        };
        assert!(!floating_chain_drop_target_is_supported(top, true));

        let left = DropTarget::GroupEdge {
            group: NodeId::null(),
            zone: DropZone::Left,
        };
        assert!(!floating_chain_drop_target_is_supported(left, true));
        assert!(floating_chain_drop_target_is_supported(left, false));

        let right = DropTarget::GroupEdge {
            group: NodeId::null(),
            zone: DropZone::Right,
        };
        assert!(floating_chain_drop_target_is_supported(right, false));
    }
}

#[cfg(test)]
mod floating_column_window_grid_rebuild_tests {
    use super::previous_window_grid_link_is_external;
    use crate::window_chain::{WindowGridLink, WindowGridMember, WindowGridDirection};
    use dock_core::{FloatingGroupId, NodeId};
    use slotmap::SlotMap;
    use std::collections::HashMap;

    #[test]
    fn links_internal_to_one_old_composition_are_not_migrated_as_external() {
        let mut nodes = SlotMap::<NodeId, ()>::with_key();
        let mut groups = SlotMap::<FloatingGroupId, ()>::with_key();
        let floating = groups.insert(());
        let first = nodes.insert(());
        let second = nodes.insert(());
        let link = WindowGridLink {
            from: WindowGridMember::Column(first),
            to: WindowGridMember::Column(second),
            direction: WindowGridDirection::Right,
        };
        let previous = HashMap::from([(WindowGridMember::Column(first), floating), (WindowGridMember::Column(second), floating)]);
        let current = HashMap::from([
            (WindowGridMember::Column(first), groups.insert(())),
            (WindowGridMember::Column(second), groups.insert(())),
        ]);
        assert!(!previous_window_grid_link_is_external(link, &previous, &current));
    }

    #[test]
    fn links_between_two_old_compositions_are_preserved() {
        let mut nodes = SlotMap::<NodeId, ()>::with_key();
        let mut groups = SlotMap::<FloatingGroupId, ()>::with_key();
        let floating_a = groups.insert(());
        let floating_b = groups.insert(());
        let first = nodes.insert(());
        let second = nodes.insert(());
        let link = WindowGridLink {
            from: WindowGridMember::Column(first),
            to: WindowGridMember::Column(second),
            direction: WindowGridDirection::Right,
        };
        let previous = HashMap::from([(WindowGridMember::Column(first), floating_a), (WindowGridMember::Column(second), floating_b)]);
        let current = HashMap::from([
            (WindowGridMember::Column(first), floating_a),
            (WindowGridMember::Column(second), floating_b),
        ]);
        assert!(previous_window_grid_link_is_external(link, &previous, &current));
    }

    #[test]
    fn links_that_become_internal_after_composition_merge_are_rebuilt_from_core() {
        let mut nodes = SlotMap::<NodeId, ()>::with_key();
        let mut groups = SlotMap::<FloatingGroupId, ()>::with_key();
        let floating_a = groups.insert(());
        let floating_b = groups.insert(());
        let merged = groups.insert(());
        let first = nodes.insert(());
        let second = nodes.insert(());
        let link = WindowGridLink {
            from: WindowGridMember::Column(first),
            to: WindowGridMember::Column(second),
            direction: WindowGridDirection::Right,
        };
        let previous = HashMap::from([
            (WindowGridMember::Column(first), floating_a),
            (WindowGridMember::Column(second), floating_b),
        ]);
        let current = HashMap::from([
            (WindowGridMember::Column(first), merged),
            (WindowGridMember::Column(second), merged),
        ]);

        assert!(
            !previous_window_grid_link_is_external(link, &previous, &current),
            "a link that becomes internal to one Core FloatingGroup must be regenerated from its current Column topology"
        );
    }
}

#[cfg(test)]
mod floating_column_topology_tests {
    use super::{
        FloatingColumnWindowSnapshot, floating_column_boundary_member,
        floating_column_internal_links,
    };
    use crate::window_chain::{WindowGridDirection, WindowGridLink, WindowGridMember, WindowGridRegistry};
    use dock_core::{FloatingGroupId, NodeId, Point, Rect, Size};
    use slotmap::SlotMap;

    fn column(
        column: NodeId,
        floating_group: FloatingGroupId,
        x: f32,
    ) -> FloatingColumnWindowSnapshot {
        FloatingColumnWindowSnapshot {
            member: WindowGridMember::Column(column),
            floating_group: Some(floating_group),
            main_drag_side: None,
            bounds: Rect::new(Point::new(x, 0.0), Size::new(240.0, 420.0)),
            floating_origin: Point::new(100.0, 200.0),
            local_origin: Point::new(x, 0.0),
            contains_transferred_panels: false,
        }
    }

    #[test]
    fn two_floating_columns_build_one_horizontal_window_grid_link() {
        let mut node_ids = SlotMap::<NodeId, ()>::with_key();
        let mut floating_ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let floating = floating_ids.insert(());
        let column_a = node_ids.insert(());
        let column_b = node_ids.insert(());

        let member_a = WindowGridMember::Column(column_a);
        let member_b = WindowGridMember::Column(column_b);
        let specs = [
            column(column_a, floating, 0.0),
            column(column_b, floating, 244.0),
        ];

        let links = floating_column_internal_links(&specs);
        assert_eq!(
            links,
            vec![WindowGridLink {
                from: member_a,
                to: member_b,
                direction: WindowGridDirection::Right,
            }],
        );

        let mut registry = WindowGridRegistry::default();
        assert!(registry.replace_all_links(&links));
        assert_eq!(registry.members_for(member_a), vec![member_a, member_b]);
        assert_eq!(registry.all_links().len(), 1);
    }

    #[test]
    fn external_composition_link_migrates_to_the_outer_column_members() {
        let mut node_ids = SlotMap::<NodeId, ()>::with_key();
        let mut floating_ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let floating_a = floating_ids.insert(());
        let floating_b = floating_ids.insert(());
        let column_a = node_ids.insert(());
        let column_b = node_ids.insert(());
        let column_c = node_ids.insert(());
        let member_a = WindowGridMember::Column(column_a);
        let member_b = WindowGridMember::Column(column_b);
        let member_c = WindowGridMember::Column(column_c);
        let specs = [
            column(column_a, floating_a, 0.0),
            column(column_b, floating_a, 244.0),
            column(column_c, floating_b, 0.0),
        ];

        let mut links = floating_column_internal_links(&specs);
        let migrated = WindowGridLink {
            from: floating_column_boundary_member(&specs, floating_a, WindowGridDirection::Right)
                .expect("right boundary of the first composition"),
            to: floating_column_boundary_member(&specs, floating_b, WindowGridDirection::Left)
                .expect("left boundary of the second composition"),
            direction: WindowGridDirection::Right,
        };
        assert_eq!(migrated.from, member_b);
        assert_eq!(migrated.to, member_c);
        links.push(migrated);

        let mut registry = WindowGridRegistry::default();
        assert!(registry.replace_all_links(&links));
        assert_eq!(registry.members_for(member_a).len(), specs.len());
        assert!(registry.all_links().contains(&migrated));
    }

    #[test]
    fn composition_boundaries_resolve_to_outermost_columns() {
        let mut node_ids = SlotMap::<NodeId, ()>::with_key();
        let mut floating_ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let floating = floating_ids.insert(());
        let column_a = node_ids.insert(());
        let column_b = node_ids.insert(());
        let member_a = WindowGridMember::Column(column_a);
        let member_b = WindowGridMember::Column(column_b);
        let specs = [
            column(column_a, floating, 0.0),
            column(column_b, floating, 244.0),
        ];

        assert_eq!(
            floating_column_boundary_member(&specs, floating, WindowGridDirection::Left),
            Some(member_a),
        );
        assert_eq!(
            floating_column_boundary_member(&specs, floating, WindowGridDirection::Right),
            Some(member_b),
        );
        assert_eq!(
            floating_column_boundary_member(&specs, floating, WindowGridDirection::Above),
            Some(member_a),
        );
        assert_eq!(
            floating_column_boundary_member(&specs, floating, WindowGridDirection::Below),
            Some(member_b),
        );
    }

    #[test]
    fn vertical_composition_boundaries_follow_each_columns_native_viewport() {
        let mut node_ids = SlotMap::<NodeId, ()>::with_key();
        let mut floating_ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let floating = floating_ids.insert(());
        let column_a = node_ids.insert(());
        let column_b = node_ids.insert(());
        let column_c = node_ids.insert(());
        let member_b = WindowGridMember::Column(column_b);
        let member_c = WindowGridMember::Column(column_c);

        let mut first = column(column_a, floating, 0.0);
        first.local_origin.y = 40.0;
        first.bounds.size.height = 180.0;
        let mut second = column(column_b, floating, 244.0);
        second.local_origin.y = 0.0;
        second.bounds.size.height = 220.0;
        let mut third = column(column_c, floating, 488.0);
        third.local_origin.y = 24.0;
        third.bounds.size.height = 420.0;

        let specs = [first, second, third];
        assert_eq!(
            floating_column_boundary_member(&specs, floating, WindowGridDirection::Above),
            Some(member_b),
        );
        assert_eq!(
            floating_column_boundary_member(&specs, floating, WindowGridDirection::Below),
            Some(member_c),
        );
    }
}
#[cfg(test)]
mod floating_column_native_resize_tests {
    use super::{
        floating_column_core_resize_edge, floating_column_resize_target_bounds,
        floating_column_right_resize_enabled,
    };
    use dock_core::{FloatingResizeEdge, Point, Rect, Size};
    use platform::NativeHorizontalResizeEdge;

    #[test]
    fn left_native_edge_resizes_the_first_core_column() {
        assert_eq!(
            floating_column_core_resize_edge(
                Some(NativeHorizontalResizeEdge::Left),
                true,
                2,
                true,
                true,
            ),
            Some(FloatingResizeEdge::Left),
        );
    }

    #[test]
    fn first_columns_right_edge_is_the_internal_first_column_resize_handle() {
        assert_eq!(
            floating_column_core_resize_edge(
                Some(NativeHorizontalResizeEdge::Right),
                true,
                2,
                true,
                false,
            ),
            Some(FloatingResizeEdge::Left),
        );
    }

    #[test]
    fn single_columns_right_edge_keeps_the_outer_right_resize_semantics() {
        assert_eq!(
            floating_column_core_resize_edge(
                Some(NativeHorizontalResizeEdge::Right),
                true,
                1,
                true,
                false,
            ),
            Some(FloatingResizeEdge::Right),
        );
    }

    #[test]
    fn last_columns_right_edge_resizes_the_last_core_column() {
        assert_eq!(
            floating_column_core_resize_edge(
                Some(NativeHorizontalResizeEdge::Right),
                false,
                3,
                true,
                false,
            ),
            Some(FloatingResizeEdge::Right),
        );
    }

    #[test]
    fn last_column_right_edge_is_enabled_when_the_outer_edge_is_free() {
        assert!(floating_column_right_resize_enabled(false, true, true, false));
    }

    #[test]
    fn middle_column_right_edge_stays_disabled() {
        assert!(!floating_column_right_resize_enabled(false, false, true, true));
    }

    #[test]
    fn last_column_right_edge_is_disabled_when_an_external_window_is_linked() {
        assert!(!floating_column_right_resize_enabled(false, true, true, true));
    }

    #[test]
    fn first_column_keeps_its_internal_right_resize_handle() {
        assert!(floating_column_right_resize_enabled(true, false, true, true));
    }

    #[test]
    fn vertical_resize_does_not_change_shared_floating_bounds() {
        let current = Rect::new(
            Point::new(100.0, 200.0),
            Size::new(500.0, 400.0),
        );
        let native = Rect::new(
            Point::new(100.0, 200.0),
            Size::new(500.0, 460.0),
        );

        assert_eq!(
            floating_column_resize_target_bounds(current, native, None, 0.0),
            current,
        );
    }

    #[test]
    fn corner_resize_only_updates_shared_horizontal_bounds() {
        let current = Rect::new(
            Point::new(100.0, 200.0),
            Size::new(500.0, 400.0),
        );
        let native = Rect::new(
            Point::new(100.0, 200.0),
            Size::new(470.0, 430.0),
        );

        assert_eq!(
            floating_column_resize_target_bounds(
                current,
                native,
                Some(FloatingResizeEdge::Left),
                -30.0,
            ),
            Rect::new(Point::new(100.0, 200.0), Size::new(470.0, 400.0)),
        );
    }

    #[test]
    fn left_corner_resize_only_updates_shared_horizontal_origin() {
        let current = Rect::new(
            Point::new(100.0, 200.0),
            Size::new(500.0, 400.0),
        );
        let native = Rect::new(
            Point::new(130.0, 170.0),
            Size::new(470.0, 430.0),
        );

        assert_eq!(
            floating_column_resize_target_bounds(
                current,
                native,
                Some(FloatingResizeEdge::Left),
                -30.0,
            ),
            Rect::new(Point::new(130.0, 200.0), Size::new(470.0, 400.0)),
        );
    }
}


//! Runtime owner of native windows for floating dock compositions.
//!
//! Main-dock panels do not allocate native surfaces. A floating Column owns one
//! Header host and each Core Group owns one content host. The shared DockController
//! remains the sole owner of Layout and panel views.

use std::{collections::{HashMap, HashSet}, time::Duration};

use dock_core::{
    DockCommand, DockSide, DragPayload, DragState, DropSource, DropTarget, FloatingGroupId,
    Node, NodeId, PanelId, Point, Rect, Size,
};
use gpui::{
    AppContext, Bounds, Context, Entity, Subscription, WeakEntity, Window, WindowBounds,
    WindowHandle, WindowId, WindowKind, WindowOptions, point, px, size,
};
use platform::{
    hide_from_taskbar, NativeKey, NativeWindowAtPoint, NativeWindowId,
    NativeWindowOps, PhysicalPoint, PhysicalSize, SystemNativeWindowOps,
};

use crate::{
    DockController, DockEvent, DockTheme, DragHostManager, DragSourceHost,
    components::{FloatingHost, floating::{FloatingPartHost, FloatingPartKind}},
    geometry::DockGeometry,
    window_chain::{WindowGridDirection, WindowGridLink, WindowGridMember, WindowGridRegistry},
    window_chain_geometry::{
        NativeFrameGeometry, arrange_window_grid_bounds as arrange_grid_geometry_bounds,
        match_moving_window_width_to_target,
        floating_grid_members_in_native_front_to_back,
        window_grid_members_in_native_z_order as grid_window_members_in_native_z_order,
        native_resize_right_edge_delta, native_resize_bottom_edge_delta,
        shift_window_grid_after_resize, floating_part_outer_bounds,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelLayoutLocation {
    Main,
    Floating(FloatingGroupId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelWindowVisibility {
    Hidden,
    Visible,
}

#[derive(Clone, Copy)]
enum PanelWindowHandle {
    Floating(WindowHandle<FloatingHost>),
    Part(WindowHandle<FloatingPartHost>),
}

impl PanelWindowHandle {
    fn is_available(&self, cx: &mut Context<FloatingWindowManager>) -> bool {
        match self {
            Self::Floating(window) => window.update(cx, |_, _, _| ()).is_ok(),
            Self::Part(window) => window.update(cx, |_, _, _| ()).is_ok(),
        }
    }

    fn set_group(
        &self,
        group: Option<FloatingGroupId>,
        cx: &mut Context<FloatingWindowManager>,
    ) -> Result<(), String> {
        match self {
            Self::Floating(window) => window
                .update(cx, |host, window, host_cx| {
                    host.set_host_group(group, window);
                    host_cx.notify();
                })
                .map_err(|error| error.to_string()),
            Self::Part(_) => Err("Floating part windows have immutable Core ownership".into()),
        }
    }

    /// Requests a redraw from the exact GPUI window that owns this host.
    fn notify_host(&self, cx: &mut Context<FloatingWindowManager>) -> Result<(), String> {
        match self {
            Self::Floating(window) => window
                .update(cx, |_, _, host_cx| host_cx.notify())
                .map_err(|error| error.to_string()),
            Self::Part(window) => window
                .update(cx, |_, _, host_cx| host_cx.notify())
                .map_err(|error| error.to_string()),
        }
    }

    fn remove_window(&self, cx: &mut Context<FloatingWindowManager>) {
        match self {
            Self::Floating(window) => {
                let _ = window.update(cx, |host, window, _| {
                    host.set_host_group(None, window);
                    window.remove_window();
                });
            }
            Self::Part(window) => {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            }
        }
    }

}

struct PanelWindowEntry {
    window: PanelWindowHandle,
    window_id: WindowId,
    native_window: Option<NativeWindowId>,
    bounds: Rect,
    native_min_size: Option<PhysicalSize>,
    native_horizontal_resize_min_widths: Option<(i32, i32)>,
    native_left_resize_enabled: Option<bool>,
    native_top_resize_enabled: Option<bool>,
    native_scale_factor: Option<f32>,
    /// The HWND may differ from Core bounds after the OS-owned resize loop.
    /// A final reconciliation must write Core's canonical bounds back.
    native_bounds_dirty: bool,
    resize_enabled: bool,
    host_group: Option<FloatingGroupId>,
    visibility: PanelWindowVisibility,
}

/// Legacy native-window records keyed by a PanelId host.
///
/// Permanent Floating Header/Group windows are stored separately in `members`;
/// this map remains only for transitional PanelId-based host paths.
#[derive(Default)]
pub struct PanelWindowRegistry {
    /// Transitional PanelId entries are retained only for an active DragHost adoption.
    windows: HashMap<PanelId, PanelWindowEntry>,
    /// Permanent Floating Header/Group windows are keyed by their Core-node identity.
    members: HashMap<WindowGridMember, PanelWindowEntry>,
    panel_locations: HashMap<PanelId, PanelLayoutLocation>,
}

impl PanelWindowRegistry {
    pub fn panel_count(&self) -> usize {
        self.windows.len()
    }

    pub fn visible_count(&self) -> usize {
        self.windows
            .values()
            .chain(self.members.values())
            .filter(|entry| entry.visibility == PanelWindowVisibility::Visible)
            .count()
    }

    pub(crate) fn member_count(&self) -> usize {
        self.members.len()
    }

    pub(crate) fn window_for_member(
        &self,
        member: WindowGridMember,
    ) -> Option<WindowHandle<FloatingPartHost>> {
        match self.members.get(&member)?.window {
            PanelWindowHandle::Part(window) => Some(window),
            _ => None,
        }
    }

    pub(crate) fn window_id_for_member(&self, member: WindowGridMember) -> Option<WindowId> {
        self.members.get(&member).map(|entry| entry.window_id)
    }

    pub(crate) fn bounds_for_member(&self, member: WindowGridMember) -> Option<Rect> {
        self.members.get(&member).map(|entry| entry.bounds)
    }

    pub(crate) fn native_window_for_member(
        &self,
        member: WindowGridMember,
    ) -> Option<NativeWindowId> {
        self.members.get(&member)?.native_window
    }

    pub(crate) fn visibility_for_member(
        &self,
        member: WindowGridMember,
    ) -> Option<PanelWindowVisibility> {
        self.members.get(&member).map(|entry| entry.visibility)
    }

    pub fn window_for_panel(&self, panel: PanelId) -> Option<WindowHandle<FloatingHost>> {
        match self.windows.get(&panel)?.window {
            PanelWindowHandle::Floating(window) => Some(window),
            PanelWindowHandle::Part(_) => None,
        }
    }

    pub fn window_id_for_panel(&self, panel: PanelId) -> Option<WindowId> {
        self.windows.get(&panel).map(|entry| entry.window_id)
    }

    pub fn bounds_for_panel(&self, panel: PanelId) -> Option<Rect> {
        self.windows.get(&panel).map(|entry| entry.bounds)
    }

    pub fn native_min_size_for_panel(&self, panel: PanelId) -> Option<PhysicalSize> {
        self.windows.get(&panel)?.native_min_size
    }

    pub fn native_scale_factor_for_panel(&self, panel: PanelId) -> Option<f32> {
        self.windows.get(&panel)?.native_scale_factor
    }

    pub fn resize_enabled_for_panel(&self, panel: PanelId) -> Option<bool> {
        self.windows.get(&panel).map(|entry| entry.resize_enabled)
    }

    pub fn native_window_for_panel(&self, panel: PanelId) -> Option<NativeWindowId> {
        self.windows.get(&panel)?.native_window
    }

    pub fn layout_location_for_panel(&self, panel: PanelId) -> Option<PanelLayoutLocation> {
        self.panel_locations.get(&panel).copied()
    }

    pub fn host_group_for_panel(&self, panel: PanelId) -> Option<FloatingGroupId> {
        self.windows.get(&panel)?.host_group
    }

    pub fn visibility_for_panel(&self, panel: PanelId) -> Option<PanelWindowVisibility> {
        self.windows.get(&panel).map(|entry| entry.visibility)
    }
}

/// Legacy mapping from a Core Floating composition to its PanelId host.
/// Permanent Floating windows no longer use this registry; Main still uses its
/// single shared MainWindow.
#[derive(Default)]
pub struct CompositionHostRegistry {
    main_window_id: Option<WindowId>,
    floating_groups: HashMap<FloatingGroupId, PanelId>,
}

impl CompositionHostRegistry {
    pub fn main_window_id(&self) -> Option<WindowId> {
        self.main_window_id
    }

    pub fn panel_for_group(&self, group: FloatingGroupId) -> Option<PanelId> {
        self.floating_groups.get(&group).copied()
    }

    pub fn group_for_panel(&self, panel: PanelId) -> Option<FloatingGroupId> {
        self.floating_groups
            .iter()
            .find_map(|(group, host)| (*host == panel).then_some(*group))
    }

    pub fn floating_count(&self) -> usize {
        self.floating_groups.len()
    }
}

/// A registered persistent DockHost under the physical cursor, together with
/// the pointer converted to that host's Core-local logical coordinate system.
/// The point is always target-host-local; it is not the original drag source point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DockingWindowAtPoint {
    ForeignWindow,
    /// The pointer in MainHost-local logical coordinates.
    Main {
        point: Point,
    },
    /// The pointer in this FloatingHost's local logical coordinates.
    Floating {
        group: FloatingGroupId,
        window_id: WindowId,
        point: Point,
    },
}

impl DockingWindowAtPoint {
    pub(crate) const fn role(self) -> Option<crate::DockWindowRole> {
        match self {
            Self::Main { .. } => Some(crate::DockWindowRole::Main),
            Self::Floating { group, .. } => Some(crate::DockWindowRole::Floating { group }),
            Self::ForeignWindow => None,
        }
    }
}

#[derive(Clone, Copy)]
enum NativeMoveTarget {
    MainDock {
        target: DropTarget,
        point: Point,
    },
    FloatingDock {
        target: DropTarget,
        point: Point,
    },
    FloatingChain {
        anchor: FloatingGroupId,
        direction: WindowGridDirection,
    },
}

#[derive(Clone, Copy)]
struct NativeMoveWindow {
    group: FloatingGroupId,
    native_window: NativeWindowId,
    /// Actual outer-window origin used as the drag baseline.
    drag_origin: PhysicalPoint,
    /// Original origin restored when the gesture is cancelled.
    cancel_origin: PhysicalPoint,
}

#[derive(Clone, Copy)]
struct NativeMoveSession {
    group: FloatingGroupId,
    native_window: NativeWindowId,
    hide_header: bool,
    initial_cursor: PhysicalPoint,
    start_bounds: Rect,
    start_pointer: Point,
    drag_threshold: f32,
    threshold_crossed: bool,
    high_resolution_timer: bool,
    drop_target: Option<NativeMoveTarget>,
}

#[derive(Clone)]
struct FloatingGroupSnapshot {
    id: FloatingGroupId,
    bounds: Rect,
    z_order: i32,
    minimum_size: Option<Size>,
    horizontal_resize_minimum_widths: Option<(f32, f32)>,
    panels: Vec<PanelId>,
}

#[derive(Clone, Copy)]
struct FloatingPartWindowSnapshot {
    member: WindowGridMember,
    floating_group: FloatingGroupId,
    kind: FloatingPartKind,
    /// Initial client bounds, using the main-monitor scale until the HWND exists.
    bounds: Rect,
    /// Core FloatingGroup origin is already in physical virtual-desktop pixels.
    floating_origin: Point,
    /// Core layout origin inside the Floating client area remains logical.
    local_origin: Point,
    contains_transferred_panels: bool,
}

fn floating_part_screen_bounds(
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

fn collect_floating_part_windows(
    layout: &dock_core::Layout,
    metrics: &dock_core::DockMetrics,
    scale_factor: f32,
    transferred_panels: &HashSet<PanelId>,
) -> Vec<FloatingPartWindowSnapshot> {
    let mut result = Vec::new();

    for (floating_group, floating) in layout.floating_groups() {
        let Ok(rects) = layout.compute_floating_rects(floating_group, metrics) else {
            continue;
        };

        for column_rect in &rects.columns {
            let Some(Node::Column(column)) = layout.node(column_rect.column) else {
                continue;
            };
            let header_local = Rect::new(
                column_rect.rect.origin,
                Size::new(
                    column_rect.rect.size.width,
                    metrics.column_header_height,
                ),
            );
            let header_bounds =
                floating_part_screen_bounds(floating.bounds, header_local, scale_factor);
            if header_bounds.validate_non_empty().is_ok() {
                result.push(FloatingPartWindowSnapshot {
                    member: WindowGridMember::Header(column_rect.column),
                    floating_group,
                    kind: FloatingPartKind::Header {
                        column: column_rect.column,
                    },
                    bounds: header_bounds,
                    floating_origin: floating.bounds.origin,
                    local_origin: header_local.origin,
                    contains_transferred_panels: false,
                });
            }

            for group_id in &column.groups {
                let Some(group_rect) = rects.groups.iter().find(|group| group.group == *group_id)
                else {
                    continue;
                };
                let mut panels = Vec::new();
                let mut seen = HashSet::new();
                collect_panel_order(layout, *group_id, &mut panels, &mut seen);
                let contains_transferred_panels = panels
                    .iter()
                    .any(|panel| transferred_panels.contains(panel));
                let bounds = floating_part_screen_bounds(
                    floating.bounds,
                    group_rect.rect,
                    scale_factor,
                );
                if bounds.validate_non_empty().is_err() {
                    continue;
                }

                result.push(FloatingPartWindowSnapshot {
                    member: WindowGridMember::Group(*group_id),
                    floating_group,
                    kind: FloatingPartKind::Group {
                        column: column_rect.column,
                        group: *group_id,
                    },
                    bounds,
                    floating_origin: floating.bounds.origin,
                    local_origin: group_rect.rect.origin,
                    contains_transferred_panels,
                });
            }
        }
    }

    result
}

fn floating_part_internal_links(
    specs: &[FloatingPartWindowSnapshot],
) -> Vec<WindowGridLink> {
    let mut floating_groups = Vec::new();
    for spec in specs {
        if !floating_groups.contains(&spec.floating_group) {
            floating_groups.push(spec.floating_group);
        }
    }

    let mut links = Vec::new();
    for floating_group in floating_groups {
        let headers = specs
            .iter()
            .filter_map(|spec| {
                (spec.floating_group == floating_group
                    && matches!(spec.kind, FloatingPartKind::Header { .. }))
                .then_some(spec)
            })
            .collect::<Vec<_>>();

        for header in &headers {
            let FloatingPartKind::Header { column } = header.kind else {
                continue;
            };
            let mut previous = header.member;
            for group in specs.iter().filter(|spec| {
                spec.floating_group == floating_group
                    && matches!(
                        spec.kind,
                        FloatingPartKind::Group {
                            column: group_column,
                            ..
                        } if group_column == column
                    )
            }) {
                links.push(WindowGridLink {
                    from: previous,
                    to: group.member,
                    direction: WindowGridDirection::Below,
                });
                previous = group.member;
            }
        }

        for pair in headers.windows(2) {
            links.push(WindowGridLink {
                from: pair[0].member,
                to: pair[1].member,
                direction: WindowGridDirection::Right,
            });
        }
    }

    links
}

fn floating_part_boundary_member(
    specs: &[FloatingPartWindowSnapshot],
    floating_group: FloatingGroupId,
    side: WindowGridDirection,
) -> Option<WindowGridMember> {
    let members = specs
        .iter()
        .filter(|spec| spec.floating_group == floating_group)
        .collect::<Vec<_>>();
    let first_header = members.iter().find_map(|spec| {
        matches!(spec.kind, FloatingPartKind::Header { .. }).then_some(spec.member)
    });
    let last_header = members.iter().rev().find_map(|spec| {
        matches!(spec.kind, FloatingPartKind::Header { .. }).then_some(spec.member)
    });
    let first_group = members.iter().find_map(|spec| {
        matches!(spec.kind, FloatingPartKind::Group { .. }).then_some(spec.member)
    });
    let last_group = members.iter().rev().find_map(|spec| {
        matches!(spec.kind, FloatingPartKind::Group { .. }).then_some(spec.member)
    });

    match side {
        WindowGridDirection::Left => first_header.or(first_group),
        WindowGridDirection::Right => last_header.or(last_group),
        WindowGridDirection::Above => first_header.or(first_group),
        WindowGridDirection::Below => last_group.or(last_header),
    }
}

#[derive(Clone, Copy)]
struct NativeResizeSession {
    group: FloatingGroupId,
    window_id: WindowId,
    start_bounds: Rect,
    transaction_started: bool,
}


fn window_grid_member(group: FloatingGroupId) -> WindowGridMember {
    WindowGridMember::floating_composition(group)
}
fn core_floating_bounds_from_grid_members(
    bounds: &[(WindowGridMember, Rect)],
) -> Option<Vec<(FloatingGroupId, Rect)>> {
    bounds.iter().map(|(member, bounds)| member.floating_group().map(|group| (group, *bounds))).collect()
}
fn floating_groups_in_native_z_order(
    groups: &[FloatingGroupSnapshot],
    grids: &WindowGridRegistry,
) -> Vec<FloatingGroupId> {
    let z_orders = groups.iter().map(|group| (window_grid_member(group.id), group.z_order)).collect::<Vec<_>>();
    grid_window_members_in_native_z_order(&z_orders, grids).into_iter().filter_map(WindowGridMember::floating_group).collect()
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
) -> NativeMoveTarget {
    match floating_window_chain_direction(target) {
        Some(direction) => NativeMoveTarget::FloatingChain {
            anchor,
            direction,
        },
        None => NativeMoveTarget::FloatingDock { target, point },
    }
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
    host_registry: CompositionHostRegistry,
    window_chains: WindowGridRegistry,
    /// Ordered permanent Header/Group members belonging to each Core FloatingGroup.
    part_members_by_group: HashMap<FloatingGroupId, Vec<WindowGridMember>>,
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
            host_registry: CompositionHostRegistry::default(),
            window_chains: WindowGridRegistry::default(),
            part_members_by_group: HashMap::new(),
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

    pub fn host_registry(&self) -> &CompositionHostRegistry {
        &self.host_registry
    }

    fn floating_group_for_member(&self, member: WindowGridMember) -> Option<FloatingGroupId> {
        match member {
            WindowGridMember::FloatingComposition(group) => Some(group),
            WindowGridMember::Header(_) | WindowGridMember::Group(_) => self
                .part_members_by_group
                .iter()
                .find_map(|(group, members)| members.contains(&member).then_some(*group)),
        }
    }

    fn grid_anchor_member_for_group(&self, group: FloatingGroupId) -> WindowGridMember {
        self.part_members_by_group
            .get(&group)
            .and_then(|members| {
                members
                    .iter()
                    .find(|member| matches!(member, WindowGridMember::Header(_)))
                    .or_else(|| {
                        members
                            .iter()
                            .find(|member| matches!(member, WindowGridMember::Group(_)))
                    })
                    .copied()
            })
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

    /// Returns every real Header/Group HWND in stable native front-to-back order.
    /// Legacy FloatingComposition members are used only before part hosts exist.
    pub(crate) fn window_grid_members_in_native_order(
        &self,
        group: FloatingGroupId,
    ) -> Vec<WindowGridMember> {
        let key = self.grid_anchor_member_for_group(group);
        let members = self.window_chains.members_for(key);
        let links = self.window_chains.links_for(key).unwrap_or_default();
        floating_grid_members_in_native_front_to_back(&members, links).unwrap_or(members)
    }

    /// Inserts the moving window's complete chain immediately before the anchor.
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
    /// This method deliberately does not reconcile: DragHost must establish
    /// the chain before its native HWND is adopted and reconciliation creates
    /// the remaining persistent Floating windows.
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
        for member in members {
            if let Some(group) = member.floating_group()
                && let Some(panel) = self.host_registry.panel_for_group(group)
                && let Some(mut entry) = self.registry.windows.remove(&panel)
            {
                self.sync_native_resize_edge_policy(&mut entry, group);
                self.registry.windows.insert(panel, entry);
            }
        }
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
    /// Header/Group resolve directly from the Core-node registry. FloatingComposition
    /// remains a temporary bridge and resolves to the first Group window in that composition.
    fn panel_window_for_grid_member(&self, member: WindowGridMember) -> Option<&PanelWindowEntry> {
        match member {
            WindowGridMember::Header(_) | WindowGridMember::Group(_) => self
                .registry
                .members
                .get(&member)
                .or_else(|| self.panel_window_for_group(member.floating_group()?)),
            WindowGridMember::FloatingComposition(group) => self.panel_window_for_group(group),
        }
    }

    fn panel_window_for_group(&self, group: FloatingGroupId) -> Option<&PanelWindowEntry> {
        if let Some(panel) = self.host_registry.panel_for_group(group)
            && let Some(entry) = self.registry.windows.get(&panel)
        {
            return Some(entry);
        }
        let members = self.part_members_by_group.get(&group)?;
        members
            .iter()
            .find(|member| matches!(member, WindowGridMember::Group(_)))
            .or_else(|| members.first())
            .and_then(|member| self.registry.members.get(member))
    }

    fn all_native_window_entries(&self) -> impl Iterator<Item = &PanelWindowEntry> {
        self.registry
            .windows
            .values()
            .chain(self.registry.members.values())
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
    fn part_outer_bounds_for_target(
        &self,
        spec: FloatingPartWindowSnapshot,
        native_window: NativeWindowId,
        fallback_scale: Option<f32>,
    ) -> Option<Rect> {
        let frame = self.native_frame_geometry(native_window, fallback_scale)?;
        floating_part_outer_bounds(
            spec.floating_origin,
            spec.local_origin,
            spec.bounds.size,
            frame,
        )
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

    /// Computes a complete grid from native frame measurements and the current
    /// CompositionHeader metric. The target anchor never moves during a Drop.
    fn arrange_window_grid_bounds(
        &self,
        bounds: Vec<(WindowGridMember, Rect)>,
        links: &[WindowGridLink],
        anchor: WindowGridMember,
        cx: &mut Context<Self>,
    ) -> Option<Vec<(WindowGridMember, Rect)>> {
        let frames = self.frame_geometries_for_bounds(&bounds)?;
        arrange_grid_geometry_bounds(bounds, &frames, links, anchor)
    }

    /// Connects the manager to the main GPUI window and the shared controller.
    ///
    /// The main HWND is retained as the native owner for each floating host.
    /// The first reconciliation creates windows only for floating compositions
    /// already present in the initial Layout.
    pub(crate) fn attach_main_window_for_drag_routing(&mut self, window: &mut Window) {
        self.host_registry.main_window_id = Some(window.window_handle().window_id());

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
                            // LayoutChanged can be emitted from inside a Floating part
                            // host update. Reconcile updates native hosts, so always defer
                            // it until the active GPUI host update unwinds.
                            manager.schedule_reconcile(manager_cx);
                        }
                        DockEvent::MetricsChanged { .. } => {
                            // Updated metrics change every Header/Group client
                            // rectangle. Queue one real member per connected WindowGrid.
                            let mut anchors = HashSet::new();
                            for group in manager.part_members_by_group.keys().copied() {
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
    pub(crate) fn screen_bounds_from_floating_local(
        &self,
        source_group: FloatingGroupId,
        bounds: Rect,
    ) -> Option<Rect> {
        bounds.validate_non_empty().ok()?;
        let native_window = self.panel_window_for_group(source_group)?.native_window?;
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

    /// Returns true when the physical cursor is over another registered
    /// docking window, even if the source window still owns pointer capture.
    pub(crate) fn cursor_over_other_docking_window(
        &self,
        source_group: FloatingGroupId,
    ) -> bool {
        let Some(source_window) = self
            .panel_window_for_group(source_group)
            .and_then(|entry| entry.native_window)
        else {
            return false;
        };
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

    pub(crate) fn hide_floating_header(&self, group: FloatingGroupId) -> bool {
        self.native_move
            .is_some_and(|state| state.group == group && state.hide_header)
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
    pub(crate) fn observe_native_resize(
        &mut self,
        group: FloatingGroupId,
        window_id: WindowId,
        cx: &mut Context<Self>,
    ) {
        self.ensure_native_resize_session(group, window_id, cx);
    }

    /// Samples current native bounds after the GPUI window-bounds observer fires.
    /// All geometry components are read from the HWND at deferred execution time.
    pub(crate) fn sync_native_resize_bounds(
        &mut self,
        group: FloatingGroupId,
        window_id: WindowId,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_native_resize_session(group, window_id, cx) {
            return;
        }
        let Some(native_window) = self
            .panel_window_for_group(group)
            .and_then(|entry| (entry.window_id == window_id).then_some(entry.native_window).flatten())
        else {
            return;
        };
        let Some(bounds) = self.core_bounds_from_native_window(native_window) else {
            return;
        };
        let Some(mut state) = self.native_resize.filter(|state| {
            state.group == group && state.window_id == window_id
        }) else {
            return;
        };
        self.apply_native_resize_bounds(group, bounds, &mut state, cx);
    }

    fn ensure_native_resize_session(
        &mut self,
        group: FloatingGroupId,
        window_id: WindowId,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.native_move.is_some() {
            return false;
        }
        if let Some(state) = self.native_resize {
            return state.group == group && state.window_id == window_id;
        }
        let Some(entry) = self.panel_window_for_group(group) else {
            return false;
        };
        if entry.window_id != window_id {
            return false;
        }
        let Some(native_window) = entry.native_window else {
            return false;
        };
        if !self.native_ops.is_resizing(native_window).unwrap_or(false) {
            return false;
        }
        let Some(start_bounds) = self.controller.update(cx, |controller, _| {
            controller.layout().floating_group(group).map(|floating| floating.bounds)
        }) else {
            return false;
        };
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
            window_id,
            start_bounds,
            transaction_started: false,
        });
        // The OS may have changed the HWND before the first Core resize sample.
        // Force one canonical Core -> HWND reconciliation when the modal loop ends,
        // even if all intermediate samples were clamped to the existing layout.
        if let Some(panel) = self.host_registry.panel_for_group(group)
            && let Some(entry) = self.registry.windows.get_mut(&panel)
            && entry.window_id == window_id
        {
            entry.native_bounds_dirty = true;
        }
        self.sync_native_resize_constraints(group, window_id, cx);
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
        let Some(mut state) = self.native_resize.filter(|state| {
            state.group == group && state.window_id == window_id
        }) else {
            return false;
        };

        let native_window = self.panel_window_for_group(group).and_then(|entry| {
            (entry.window_id == window_id).then_some(entry.native_window).flatten()
        });
        let Some(native_window) = native_window else {
            self.finish_native_resize_transaction(state, cx);
            return false;
        };

        // The bounds observer is the fast path, but rapid native sizing
        // messages can be coalesced before GPUI reports a viewport change.
        // Sample the HWND here as a fallback so Core cannot lag behind the
        // actual client size during short, repeated resize movements.
        if self.native_ops.is_resizing(native_window).unwrap_or(false) {
            if let Some(bounds) = self.core_bounds_from_native_window(native_window) {
                self.apply_native_resize_bounds(group, bounds, &mut state, cx);
            }
            // Refresh even when HWND bounds did not change: that can mean a
            // stale Win32 minimum is clamping the proposed edge in both directions.
            self.sync_native_resize_constraints(group, window_id, cx);
            self.native_resize = Some(state);
            return true;
        }

        if let Some(bounds) = self.core_bounds_from_native_window(native_window) {
            self.apply_native_resize_bounds(group, bounds, &mut state, cx);
        }
        self.finish_native_resize_transaction(state, cx);
        false
    }

    fn apply_native_resize_bounds(
        &mut self,
        group: FloatingGroupId,
        bounds: Rect,
        state: &mut NativeResizeSession,
        cx: &mut Context<Self>,
    ) {
        let current = self.controller.update(cx, |controller, _| {
            controller.layout().floating_group(group).map(|floating| floating.bounds)
        });
        let Some(current_bounds) = current else {
            return;
        };
        if current_bounds == bounds {
            return;
        }

        if !state.transaction_started {
            state.transaction_started = self.controller.update(cx, |controller, _| {
                controller.begin_native_resize_transaction()
            });
        }
        // Mark the OS sizing loop authoritative before dispatch: reconciliation
        // must not write Core geometry back to the HWND during the drag.
        self.native_resize = Some(*state);
        if !state.transaction_started {
            return;
        }

        let native_window = self
            .panel_window_for_group(group)
            .and_then(|entry| entry.native_window);
        let native_edge = native_window
            .and_then(|window| self.native_ops.horizontal_resize_edge(window).ok())
            .flatten()
            .map(|edge| match edge {
                platform::NativeHorizontalResizeEdge::Left => dock_core::FloatingResizeEdge::Left,
                platform::NativeHorizontalResizeEdge::Right => dock_core::FloatingResizeEdge::Right,
            });
        // WM_SIZING is authoritative. Retain the origin comparison only as a
        // fallback for platform adapters that cannot report a native hit edge.
        let edge = native_edge.or_else(|| {
            let width_changed = (bounds.size.width - current_bounds.size.width).abs() > 0.5;
            width_changed.then(|| {
                if (bounds.origin.x - state.start_bounds.origin.x).abs() > 0.5 {
                    dock_core::FloatingResizeEdge::Left
                } else {
                    dock_core::FloatingResizeEdge::Right
                }
            })
        });
        let scale_factor = native_window
            .and_then(|window| self.native_ops.scale_factor(window).ok())
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        let command = match edge {
            Some(edge) => DockCommand::ResizeFloatingBoundsFromEdge {
                group,
                bounds,
                edge,
                scale_factor,
            },
            None => DockCommand::ResizeFloatingBounds { group, bounds },
        };
        let group_member = window_grid_member(group);
        let grid_links = self.window_chains.links_for(group_member).unwrap_or_default().to_vec();
        let grid_geometry = if grid_links.is_empty() {
            None
        } else {
            let members = self.window_chains.members_for(group_member);
            let bounds = self.controller.update(cx, |controller, _| {
                members.iter().map(|member| {
                    let member_group = member.floating_group()?;
                    controller.layout().floating_group(member_group).map(|floating| (*member, floating.bounds))
                }).collect::<Option<Vec<_>>>()
            });
            bounds.and_then(|bounds| self.frame_geometries_for_bounds(&bounds).map(|frames| (bounds, frames)))
        };

        self.controller.update(cx, |controller, controller_cx| {
            let primary_result = controller.dispatch(command);
            match primary_result {
                Ok(_) => {
                    let actual_bounds = controller
                        .layout()
                        .floating_group(group)
                        .map(|floating| floating.bounds);
                    if let Some(actual_bounds) = actual_bounds {
                        let right_edge_delta = native_resize_right_edge_delta(
                            current_bounds,
                            actual_bounds,
                            scale_factor,
                        )
                        .unwrap_or(0.0);
                        let bottom_edge_delta = native_resize_bottom_edge_delta(
                            current_bounds,
                            actual_bounds,
                            scale_factor,
                        )
                        .unwrap_or(0.0);

                        if right_edge_delta.abs() > 0.01 || bottom_edge_delta.abs() > 0.01 {
                            if let Some((grid_bounds, grid_frames)) = grid_geometry.as_ref() {
                                let actual_grid_bounds = grid_bounds
                                    .iter()
                                    .map(|(member, _)| {
                                        let member_group = member.floating_group()?;
                                        let member_bounds = if *member == group_member {
                                            Some(actual_bounds)
                                        } else {
                                            controller.layout().floating_group(member_group).map(|floating| floating.bounds)
                                        };
                                        member_bounds.map(|bounds| (*member, bounds))
                                    })
                                    .collect::<Option<Vec<_>>>();
                                let actual_grid_frames = grid_bounds
                                    .iter()
                                    .zip(grid_frames.iter())
                                    .map(|((member, _), frame)| {
                                        if *member == group_member {
                                            frame.with_logical_client_size(actual_bounds.size)
                                        } else {
                                            Some(*frame)
                                        }
                                    })
                                    .collect::<Option<Vec<_>>>();

                                if let (Some(actual_grid_bounds), Some(actual_grid_frames)) =
                                    (actual_grid_bounds, actual_grid_frames)
                                {
                                    if let Some(shifted_bounds) = shift_window_grid_after_resize(
                                        actual_grid_bounds,
                                        &actual_grid_frames,
                                        &grid_links,
                                        group_member,
                                        right_edge_delta,
                                        bottom_edge_delta,
                                    ) {
                                        let neighbour_commands = shifted_bounds
                                            .into_iter()
                                            .filter_map(|(member, member_bounds)| {
                                                if member == group_member { return None; }
                                                let member_group = member.floating_group()?;
                                                let current = controller.layout().floating_group(member_group)?.bounds;
                                                (current != member_bounds).then_some(DockCommand::ResizeFloatingBounds {
                                                    group: member_group,
                                                    bounds: member_bounds,
                                                })
                                            })
                                            .collect::<Vec<_>>();
                                        if !neighbour_commands.is_empty() {
                                            if let Err(error) =
                                                controller.dispatch_sequence(neighbour_commands)
                                            {
                                                tracing::warn!(
                                                    ?group,
                                                    %error,
                                                    "could not reflow the affected WindowGrid region after native Resize"
                                                );
                                            }
                                        }
                                    } else {
                                        tracing::warn!(
                                            ?group,
                                            "could not calculate the affected WindowGrid region after native Resize"
                                        );
                                    }
                                } else {
                                    tracing::warn!(
                                        ?group,
                                        "could not read all Core bounds or frame metrics for WindowGrid Resize"
                                    );
                                }
                            } else if !grid_links.is_empty() {
                                tracing::warn!(
                                    ?group,
                                    "could not measure native geometry for WindowGrid Resize"
                                );
                            }
                        }
                    }
                }
                Err(error) => tracing::warn!(
                    ?group,
                    %error,
                    "could not synchronize native floating-window bounds with Core"
                ),
            }

            // The primary Resize and all dependent neighbour moves stay inside
            // the same native-resize history transaction and one event flush.
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
        pointer: Point,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.window_chains.is_chained(window_grid_member(group)) {
            return false;
        }
        self.begin_existing_window_move(group, pointer, cx)
    }

    /// Starts a thresholded native move from the shared floating DockAreaHeader.
    pub(crate) fn begin_existing_window_move(
        &mut self,
        group: FloatingGroupId,
        pointer: Point,
        cx: &mut Context<Self>,
    ) -> bool {
        self.begin_existing_window_move_with_header(group, pointer, false, cx)
    }

    fn begin_existing_window_move_with_header(
        &mut self,
        group: FloatingGroupId,
        pointer: Point,
        hide_header: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.native_move.is_some() || self.native_resize.is_some() {
            return false;
        }
        let Some(entry) = self.panel_window_for_group(group) else {
            return false;
        };
        let (Some(native_window), window_id) = (entry.native_window, entry.window_id) else {
            return false;
        };
        let Ok(cursor) = self.native_ops.cursor_position() else {
            return false;
        };
        let Ok(native_rect) = self.native_ops.window_rect(native_window) else {
            return false;
        };
        let start_bounds = self.controller.update(cx, |controller, _| {
            controller.layout().floating_group(group).map(|floating| floating.bounds)
        });
        let Some(start_bounds) = start_bounds else {
            return false;
        };
        let metrics = self.controller.update(cx, |controller, _| controller.metrics());
        let scale = self.native_ops.scale_factor(native_window).ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        let mut move_windows = Vec::new();
        for member in self.window_grid_members_in_native_order(group) {
            let Some(member_group) = member.floating_group() else { return false; };
            let Some(member_entry) = self.panel_window_for_grid_member(member) else {
                return false;
            };
            let Some(member_window) = member_entry.native_window else {
                return false;
            };
            let member_rect = if member_window == native_window {
                native_rect
            } else {
                match self.native_ops.window_rect(member_window) {
                    Ok(rect) => rect,
                    Err(_) => return false,
                }
            };
            let origin = PhysicalPoint {
                x: member_rect.left,
                y: member_rect.top,
            };
            move_windows.push(NativeMoveWindow {
                group: member_group,
                native_window: member_window,
                drag_origin: origin,
                cancel_origin: origin,
            });
        }
        if !move_windows.iter().any(|member| member.group == group) {
            return false;
        }

        let hidden_header_offset = if hide_header {
            let Some(offset) = DockGeometry::physical_coordinate(metrics.column_header_height, scale) else {
                return false;
            };
            offset
        } else {
            0
        };
        if !self.controller.update(cx, |controller, _| {
            controller.begin_floating_window_move(group, pointer)
        }) {
            return false;
        }
        if hidden_header_offset != 0 {
            for member in &mut move_windows {
                member.drag_origin.y = member.drag_origin.y.saturating_add(hidden_header_offset);
            }

            let mut move_error = None;
            for member in &move_windows {
                if let Err(error) = self.native_ops.move_window(member.native_window, member.drag_origin) {
                    move_error = Some(error);
                    break;
                }
            }

            if let Some(error) = move_error {
                tracing::warn!(?group, %error, "could not align native chain after hiding ColumnHeader");
                for restore in &move_windows {
                    let _ = self.native_ops.move_window(restore.native_window, restore.cancel_origin);
                }
                self.cancel_controller_move(cx);
                return false;
            }
        }
        self.native_move_windows = move_windows;
        let high_resolution_timer = self.native_ops.begin_high_resolution_timer().is_ok();
        self.native_move = Some(NativeMoveSession {
            group,
            native_window,
            hide_header,
            initial_cursor: cursor,
            start_bounds,
            start_pointer: pointer,
            drag_threshold: metrics.drag_threshold * scale,
            threshold_crossed: false,
            high_resolution_timer,
            drop_target: None,
        });
        self.schedule_native_move(cx, group, window_id);
        if hide_header {
            cx.notify();
        }
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
        if self.panel_window_for_group(group).is_none_or(|entry| entry.window_id != window_id)
            || self.native_move_windows.is_empty()
        {
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

        self.update_native_drop_target(&mut state, cursor, cx);
        self.native_move = Some(state);
        if !self.native_ops.left_button_down().unwrap_or(false) {
            self.finish_native_move(state, cx);
            return false;
        }
        true
    }

    fn update_native_drop_target(&self, state: &mut NativeMoveSession, cursor: PhysicalPoint, cx: &mut Context<Self>) {
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
            state.drop_target = None;
            self.clear_native_drag_target(cx);
            return;
        };
        let mut known = Vec::new();
        if let Some(main) = self.main_native_window { known.push(main); }
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
            Ok(NativeWindowAtPoint::Foreign | NativeWindowAtPoint::Empty) | Err(_) => {
                state.drop_target = None;
                self.clear_native_drag_target(cx);
                return;
            }
        };
        let moving_chain = self.native_move_windows.len() > 1;
        let moving_chain_can_dock_to_main = moving_chain
            && self.controller.update(cx, |controller, _| {
                self.native_move_windows.iter().all(|moving| {
                    controller
                        .layout()
                        .floating_group(moving.group)
                        .and_then(|floating| controller.layout().dock_area(floating.area))
                        .is_some_and(|area| {
                            matches!(controller.layout().node(area.root), Some(Node::Column(_)))
                        })
                })
            });
        let resolved = if Some(target_window) == self.main_native_window {
            let Some(point) = self.core_point_for_main(cursor) else {
                state.drop_target = None;
                self.clear_native_drag_target(cx);
                return;
            };
            self.controller.update(cx, |controller, controller_cx| {
                let target = match controller.prepare_drop_resolver(source)
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
                controller.drag_mut().session_mut().pointer_move(point).ok()?;
                controller.drag_mut().session_mut().set_drop_target(target).ok()?;
                controller_cx.notify();
                target.map(|target| NativeMoveTarget::MainDock { target, point })
            })
        } else {
            let Some((target_group, target_window_id)) = self.host_registry.floating_groups.iter().find_map(|(group, panel)| {
                let entry = self.registry.windows.get(panel)?;
                (entry.native_window == Some(target_window) && *group != state.group)
                    .then_some((*group, entry.window_id))
            }) else {
                state.drop_target = None;
                self.clear_native_drag_target(cx);
                return;
            };
            let Some(point) = self.core_point_for_floating(target_window, cursor) else {
                state.drop_target = None;
                self.clear_native_drag_target(cx);
                return;
            };

            // Resolve the target Floating's own edge zones so the destination
            // window paints the same affordance used to choose the WindowGrid side.
            // The Core composition is not docked into the target; only HWND bounds
            // and WindowGrid membership are changed after release.
            let resolved_target = self.controller.update(cx, |controller, controller_cx| {
                let resolved = controller
                    .prepare_floating_window_drop_resolver(target_window_id, target_group, source)
                    .and_then(|resolver| resolver.resolve(point));
                let target = match resolved {
                    // Keep every legal target from the Floating resolver. The four
                    // cardinal GroupEdge zones are interpreted as native WindowGrid
                    // insertion on release; other zones retain Core docking semantics.
                    Ok(target) => target.filter(|target| {
                        !moving_chain || floating_chain_drop_target_is_supported(*target, false)
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

            resolved_target.map(|target| {
                floating_window_drop_destination(target_group, target, point)
            })
        };
        state.drop_target = resolved;
    }


    fn clear_native_drag_target(&self, cx: &mut Context<Self>) {
        self.controller.update(cx, |controller, controller_cx| {
            controller.drag_mut().clear_resolver();
            let _ = controller.drag_mut().session_mut().set_drop_target(None);
            controller_cx.notify();
        });
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
        let Some(main_window_id) = self.host_registry.main_window_id else {
            return Some(DockingWindowAtPoint::ForeignWindow);
        };
        let Some(main_native_window) = self.main_native_window else {
            return Some(DockingWindowAtPoint::ForeignWindow);
        };
        let mut known_windows = Vec::with_capacity(
            self.registry.windows.len() + self.registry.members.len() + 1,
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

        // The temporary DragHost is never an input target. The native resolver
        // skips it and continues through the desktop Z-order to a registered host.
        let target_native = match self
            .native_ops
            .window_from_point(screen, &known_windows, Some(drag_host))
        {
            Ok(NativeWindowAtPoint::Known(window)) => window,
            Ok(NativeWindowAtPoint::Foreign) | Err(_) => {
                return Some(DockingWindowAtPoint::ForeignWindow);
            }
            Ok(NativeWindowAtPoint::Empty) => return None,
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
            return Some(DockingWindowAtPoint::Floating {
                group,
                window_id: entry.window_id,
                point,
            });
        }

        let Some(entry) = self.registry.windows.values().find(|entry| {
            entry.visibility == PanelWindowVisibility::Visible
                && entry.native_window == Some(target_native)
        }) else {
            return Some(DockingWindowAtPoint::ForeignWindow);
        };
        let (Some(group), Some(point)) = (
            entry.host_group,
            self.core_point_for_floating(target_native, screen),
        ) else {
            return Some(DockingWindowAtPoint::ForeignWindow);
        };
        Some(DockingWindowAtPoint::Floating {
            group,
            window_id: entry.window_id,
            point,
        })
    }

    /// Returns true when the cursor is over a managed Floating Header/Group or legacy host.
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
                if source_window_id.is_some_and(|id| Some(id) != self.host_registry.main_window_id) {
                    return None;
                }
                self.core_point_for_main(screen)
            }
            DragSourceHost::Floating { group } => {
                if let Some((member, entry)) = self.registry.members.iter().find(|(_, entry)| {
                    source_window_id.is_some_and(|id| id == entry.window_id)
                }) {
                    let native_window = entry.native_window?;
                    return self.core_point_for_floating_member(
                        *member,
                        entry.host_group?,
                        entry.window_id,
                        native_window,
                        screen,
                        cx,
                    );
                }

                let entry = self.panel_window_for_group(group)?;
                if source_window_id.is_some_and(|id| id != entry.window_id) {
                    return None;
                }
                self.core_point_for_floating(entry.native_window?, screen)
            }
        }
    }

    /// Converts bounds expressed in the original source host's Core-local coordinates
    /// into physical screen coordinates.
    pub(crate) fn screen_bounds_for_drag_source(
        &self,
        source_host: DragSourceHost,
        bounds: Rect,
    ) -> Option<Rect> {
        match source_host {
            DragSourceHost::Main => self.screen_bounds_from_main_local(bounds),
            DragSourceHost::Floating { group } => {
                self.screen_bounds_from_floating_local(group, bounds)
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

    fn core_point_for_floating(&self, window: NativeWindowId, screen: PhysicalPoint) -> Option<Point> {
        let client = self.native_ops.client_origin(window).ok()?;
        let scale = self.native_ops.scale_factor(window).ok()?;
        DockGeometry::host_local_point_from_screen(screen, client, Point::ZERO, scale)
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
        let local = self.core_point_for_floating(native_window, screen)?;
        let origin = self.controller.update(cx, |controller, _| {
            let rects = controller.floating_geometry(window_id, group).ok()?;
            match member {
                WindowGridMember::Header(column) => rects
                    .columns
                    .iter()
                    .find(|rect| rect.column == column)
                    .map(|rect| rect.rect.origin),
                WindowGridMember::Group(group) => rects
                    .groups
                    .iter()
                    .find(|rect| rect.group == group)
                    .map(|rect| rect.rect.origin),
                WindowGridMember::FloatingComposition(_) => None,
            }
        })?;
        Some(Point::new(local.x + origin.x, local.y + origin.y))
    }

    fn finish_native_move(&mut self, state: NativeMoveSession, cx: &mut Context<Self>) {
        if state.high_resolution_timer {
            let _ = self.native_ops.end_high_resolution_timer();
        }

        let moving_windows = std::mem::take(&mut self.native_move_windows);
        let foreground_at_release = self.native_ops.foreground_window().ok().flatten();
        let mut docked_chain_into_main = false;
        let mut floating_bounds = self.controller.update(cx, |controller, _| {
            controller
                .layout()
                .floating_group(state.group)
                .map(|floating| floating.bounds)
        }).unwrap_or(state.start_bounds);

        if let Some(native_bounds) = self.core_bounds_from_native_window(state.native_window) {
            floating_bounds = native_bounds;
        } else if let Ok(rect) = self.native_ops.window_rect(state.native_window) {
            floating_bounds.origin = Point::new(rect.left as f32, rect.top as f32);
        }

        self.native_move = None;
        let result = match state.drop_target {
            Some(NativeMoveTarget::FloatingChain {
                anchor,
                direction,
            }) => self.finish_floating_window_chain_move(
                state.group,
                anchor,
                direction,
                &moving_windows,
                cx,
            ),
            Some(NativeMoveTarget::MainDock {
                target: DropTarget::MainColumn { side, index },
                ..
            }) if moving_windows.len() > 1 => {
                let ordered_groups = moving_windows
                    .iter()
                    .map(|moving| moving.group)
                    .collect::<Vec<_>>();
                match self.controller.update(cx, |controller, controller_cx| {
                    controller.finish_floating_window_chain_dock_in(
                        &ordered_groups,
                        side,
                        index,
                        controller_cx,
                    )
                }) {
                    Ok(()) => {
                        docked_chain_into_main = true;
                        Ok(())
                    }
                    Err(error) => {
                        tracing::warn!(
                            ?state.group,
                            %error,
                            "could not dock the Floating WindowChain into Main"
                        );
                        Err(error)
                    }
                }
            }
            _ if moving_windows.len() > 1 => {
                // A chain header can move the chain, but cannot detach one
                // member by dropping it into Main or onto empty desktop space.
                let bounds = self.native_bounds_for_move_windows(&moving_windows, cx);
                match bounds {
                    Some(bounds) => self.controller.update(cx, |controller, controller_cx| {
                        controller.finish_floating_window_chain_move_in(&bounds, controller_cx)
                    }),
                    None => Err(dock_core::CommandError::InvalidTarget {
                        reason: "could not read the final bounds of every window in the chain",
                    }),
                }
            }
            drop_target => {
                let dock_target = match drop_target {
                    Some(NativeMoveTarget::MainDock { target, point })
                    | Some(NativeMoveTarget::FloatingDock { target, point }) => {
                        Some((target, point))
                    }
                    _ => None,
                };
                self.controller.update(cx, |controller, controller_cx| {
                    controller.finish_floating_window_move_in(
                        state.group,
                        floating_bounds,
                        dock_target,
                        controller_cx,
                    )
                })
            }
        };

        if let Err(error) = result {
            tracing::warn!(?state.group, %error, "could not commit native floating-window move");
            self.cancel_controller_move(cx);
            for member in &moving_windows {
                self.mark_group_native_bounds_dirty(member.group);
            }
            if moving_windows.is_empty() {
                self.mark_group_native_bounds_dirty(state.group);
            }
            self.schedule_reconcile(cx);
        }

        if matches!(
            state.drop_target,
            Some(NativeMoveTarget::FloatingDock { .. })
                | Some(NativeMoveTarget::MainDock {
                    target: DropTarget::MainColumn { .. },
                    ..
                })
        ) {
            // Docking changes which native windows own visible compositions.
            // Reconcile after the active native-move update unwinds.
            self.schedule_reconcile(cx);

            if docked_chain_into_main {
                let moving_native_windows = moving_windows
                    .iter()
                    .map(|moving| moving.native_window)
                    .collect();
                self.schedule_main_activation_after_window_chain_dock(
                    moving_native_windows,
                    foreground_at_release,
                    cx,
                );
            }
        } else {
            self.redraw_visible_hosts(cx);
        }
        cx.notify();
    }

    fn native_bounds_for_move_windows(
        &self,
        moving_windows: &[NativeMoveWindow],
        cx: &mut Context<Self>,
    ) -> Option<Vec<(WindowGridMember, Rect)>> {
        let mut bounds = self.controller.update(cx, |controller, _| {
            moving_windows.iter().map(|moving| {
                controller.layout().floating_group(moving.group)
                    .map(|floating| (window_grid_member(moving.group), floating.bounds))
            }).collect::<Option<Vec<_>>>()
        })?;
        for (index, moving) in moving_windows.iter().enumerate() {
            if let Some(native_bounds) = self.core_bounds_from_native_window(moving.native_window) {
                bounds[index].1 = native_bounds;
            } else if let Ok(rect) = self.native_ops.window_rect(moving.native_window) {
                bounds[index].1.origin = Point::new(rect.left as f32, rect.top as f32);
            }
        }
        Some(bounds)
    }

    fn finish_floating_window_chain_move(
        &mut self,
        moving: FloatingGroupId,
        anchor: FloatingGroupId,
        direction: WindowGridDirection,
        moving_windows: &[NativeMoveWindow],
        cx: &mut Context<Self>,
    ) -> Result<(), dock_core::CommandError> {
        let moving_member = window_grid_member(moving);
        let anchor_member = window_grid_member(anchor);
        let mut next_chains = self.window_chains.clone();
        if next_chains.members_for(moving_member).contains(&anchor_member) {
            let window_bounds = self.native_bounds_for_move_windows(moving_windows, cx)
                .ok_or(dock_core::CommandError::InvalidTarget {
                    reason: "could not read the final bounds of every window in the existing WindowGrid",
                })?;
            let core_bounds = core_floating_bounds_from_grid_members(&window_bounds)
                .ok_or(dock_core::CommandError::InvalidTarget {
                    reason: "the current WindowGrid contains a member without a Core FloatingGroup",
                })?;
            return self.controller.update(cx, |controller, controller_cx| {
                controller.finish_floating_window_chain_move_in(&core_bounds, controller_cx)
            });
        }
        if !next_chains.link_to_side(anchor_member, moving_member, direction) {
            return Err(dock_core::CommandError::InvalidTarget {
                reason: "could not merge the source and target WindowGrids without an ambiguous connection",
            });
        }
        let members = next_chains.members_for(anchor_member);
        let Some(member_bounds) = self.controller.update(cx, |controller, _| {
            members.iter().map(|member| {
                let group = member.floating_group()?;
                controller.layout().floating_group(group).map(|floating| (*member, floating.bounds))
            }).collect::<Option<Vec<_>>>()
        }) else {
            return Err(dock_core::CommandError::InvalidTarget {
                reason: "window grid contains a member without a current Core FloatingGroup",
            });
        };
        let member_bounds = match direction {
            WindowGridDirection::Above | WindowGridDirection::Below => {
                let minimum_width = self.floating_minimum_width(moving, cx)
                    .ok_or(dock_core::CommandError::InvalidTarget {
                        reason: "could not read the minimum width of the moving FloatingGroup",
                    })?;
                match_moving_window_width_to_target(member_bounds, anchor_member, moving_member, minimum_width)
            }
            WindowGridDirection::Left | WindowGridDirection::Right => Some(member_bounds),
        }.ok_or(dock_core::CommandError::InvalidTarget {
            reason: "could not normalize the Floating window bounds",
        })?;
        let links = next_chains.links_for(anchor_member)
            .ok_or(dock_core::CommandError::InvalidTarget {
                reason: "new Floating WindowGrid has no adjacency links",
            })?.to_vec();
        let bounds = self.arrange_window_grid_bounds(member_bounds, &links, anchor_member, cx)
            .ok_or(dock_core::CommandError::InvalidTarget {
                reason: "could not calculate native bounds for the Floating WindowGrid connection",
            })?;
        let core_bounds = core_floating_bounds_from_grid_members(&bounds)
            .ok_or(dock_core::CommandError::InvalidTarget {
                reason: "the WindowGrid contains a member without a Core FloatingGroup",
            })?;
        let result = self.controller.update(cx, |controller, controller_cx| {
            controller.finish_floating_window_chain_move_in(&core_bounds, controller_cx)
        });
        if result.is_ok() {
            self.window_chains = next_chains;
            self.pending_window_chain_alignment.insert(anchor_member);
            for member in members {
                if let Some(group) = member.floating_group()
                    && let Some(panel) = self.host_registry.panel_for_group(group)
                    && let Some(mut entry) = self.registry.windows.remove(&panel)
                {
                    self.sync_native_resize_edge_policy(&mut entry, group);
                    self.registry.windows.insert(panel, entry);
                }
            }
        }
        result
    }

    fn cancel_native_move(&mut self, state: NativeMoveSession, cx: &mut Context<Self>) {
        self.native_move = None;
        let moving_windows = std::mem::take(&mut self.native_move_windows);
        if state.high_resolution_timer {
            let _ = self.native_ops.end_high_resolution_timer();
        }
        let _ = self.controller.update(cx, |controller, controller_cx| {
            let result = controller.cancel_pointer_interaction_on_escape();
            controller.flush_events(controller_cx);
            controller_cx.notify();
            result
        });
        for member in moving_windows {
            if let Err(error) = self.native_ops.move_window(member.native_window, member.cancel_origin) {
                tracing::warn!(?state.group, moving_group = ?member.group, %error, "could not restore floating chain member after Escape");
            }
        }
        cx.notify();
    }

    fn cancel_controller_move(&mut self, cx: &mut Context<Self>) {
        let _ = self.controller.update(cx, |controller, controller_cx| {
            let result = controller.cancel_pointer_interaction();
            controller.flush_events(controller_cx);
            controller_cx.notify();
            result
        });
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
    /// Floating Core drops can change the set of permanent Header/Group hosts.
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
        let Some(panel) = self.host_registry.panel_for_group(group) else {
            return;
        };
        if let Some(entry) = self.registry.windows.get_mut(&panel) {
            entry.native_bounds_dirty = true;
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

    /// Makes the typed Header/Group window registry match the current Core Layout.
    ///
    /// Each Floating Column owns one Header host, and each Group owns one host.
    /// Main remains in its original MainDockView and never enters this registry.
    pub fn reconcile(&mut self, cx: &mut Context<Self>) {
        if self.reconciling {
            return;
        }
        self.reconciling = true;

        let (mut groups, panel_locations, recover_initial_bounds, transferred_panels) =
            self.controller.update(cx, |controller, _| {
                let recover_initial_bounds = controller.take_initial_monitor_recovery_pending();
                let transferred_panels = controller
                    .drag()
                    .host_session()
                    .map(|session| session.transferred_panels().to_vec())
                    .unwrap_or_default();
                let layout = controller.layout();
                let metrics = controller.metrics();
                let mut panel_locations = HashMap::new();

                for side in [DockSide::Left, DockSide::Right] {
                    if let Some(area) = layout.main_area(side) {
                        let mut panels = Vec::new();
                        let mut seen = HashSet::new();
                        collect_panel_order(layout, area.root, &mut panels, &mut seen);
                        for panel in panels {
                            panel_locations.insert(panel, PanelLayoutLocation::Main);
                        }
                    }
                }

                let mut groups = Vec::new();
                for (id, group) in layout.floating_groups() {
                    let Some(area) = layout.dock_area(group.area) else {
                        continue;
                    };

                    let mut panels = Vec::new();
                    let mut seen = HashSet::new();
                    collect_panel_order(layout, area.root, &mut panels, &mut seen);
                    for panel in &panels {
                        panel_locations.insert(*panel, PanelLayoutLocation::Floating(id));
                    }

                    groups.push(FloatingGroupSnapshot {
                        id,
                        bounds: group.bounds,
                        z_order: group.z_order,
                        minimum_size: layout.floating_minimum_size(id, &metrics).ok(),
                        horizontal_resize_minimum_widths: layout
                            .floating_horizontal_resize_minimum_widths(id, &metrics)
                            .ok()
                            .flatten(),
                        panels,
                    });
                }

                (groups, panel_locations, recover_initial_bounds, transferred_panels)
            });

        groups.sort_by_key(|group| group.z_order);
        let prune_result = self.reconcile_part_windows(&transferred_panels, cx);
        let removed_chain_members = prune_result.removed_members;
        if removed_chain_members > 0 {
            tracing::debug!(removed_chain_members, "removed stale WindowGrid members");
        }
        for candidate in prune_result.reconnect_candidates {
            if !self.pending_window_grid_reconnections.contains(&candidate) {
                self.pending_window_grid_reconnections.push(candidate);
            }
        }

        // Permanent Floating hosts are now keyed by Header/Group Core nodes.
        // The PanelId-host table remains only for transitional DragHost ownership.
        let next_hosts = HashMap::<FloatingGroupId, PanelId>::new();
        let desired_panels = HashSet::<PanelId>::new();

        let monitor_work_areas = if recover_initial_bounds {
            match self.native_ops.monitor_work_areas() {
                Ok(areas) => Some(areas),
                Err(error) => {
                    tracing::debug!(
                        %error,
                        "could not enumerate monitor work areas for initial floating-window recovery"
                    );
                    None
                }
            }
        } else {
            None
        };

        // Discard externally closed legacy/DragHost handles. Permanent Floating
        // Header/Group windows have already been reconciled by Core-node identity.
        let unavailable_panels = self
            .registry
            .windows
            .iter()
            .filter_map(|(panel, entry)| {
                (!entry.window.is_available(cx)).then_some(*panel)
            })
            .collect::<Vec<_>>();
        for group in &groups {
            if !self.window_chains.is_chained(window_grid_member(group.id)) {
                continue;
            }
            let Some(panel) = next_hosts.get(&group.id).copied() else {
                continue;
            };
            let host_changed = previous_hosts.get(&group.id).copied() != Some(panel);
            let host_lost = unavailable_panels.contains(&panel);
            if host_changed || host_lost {
                self.pending_window_chain_alignment.insert(window_grid_member(group.id));
            }
        }
        for panel in unavailable_panels {
            self.release_panel_window(panel, cx);
        }

        let missing_panels = desired_panels
            .iter()
            .filter(|panel| !self.registry.windows.contains_key(panel))
            .copied()
            .collect::<Vec<_>>();
        for group in &groups {
            if self.window_chains.is_chained(window_grid_member(group.id))
                && next_hosts
                    .get(&group.id)
                    .is_some_and(|panel| missing_panels.contains(panel))
            {
                self.pending_window_chain_alignment.insert(window_grid_member(group.id));
            }
        }
        for (index, panel) in missing_panels.into_iter().enumerate() {
            let Some(location) = panel_locations.get(&panel).copied() else {
                continue;
            };
            let bounds = self.initial_panel_window_bounds(panel, location, &groups, index, cx);
            if let Some(entry) = self.create_panel_window(panel, bounds, cx) {
                self.registry.windows.insert(panel, entry);
            } else {
                tracing::error!(
                    ?panel,
                    "could not create a native host window for a floating composition"
                );
            }
        }

        // The selected host remains stable while it belongs to the same
        // composition. New compositions get their first panel as a host.
        let mut next_group_by_panel = HashMap::new();
        for (group, panel) in &next_hosts {
            next_group_by_panel.insert(*panel, *group);
        }

        // First detach old hosts and hide them. This happens before any new
        // window mounts the composition, preventing one AnyView from being
        // hosted by two GPUI windows during a host transfer.
        let detached_panels = self
            .registry
            .windows
            .iter()
            .filter_map(|(panel, entry)| {
                (entry.host_group != next_group_by_panel.get(panel).copied())
                    .then_some(*panel)
                    .filter(|_| entry.host_group.is_some())
            })
            .collect::<Vec<_>>();
        // A failed GPUI detach must not be followed by mounting the same
        // composition in another window. Keep that group without an active host
        // for this pass and retry the detach on the next reconciliation.
        let mut blocked_groups = HashSet::new();
        let mut hide_attempted_panels = HashSet::new();
        for panel in detached_panels {
            if let Some(entry) = self.registry.windows.get_mut(&panel) {
                let previous_group = entry.host_group;
                let detached = match entry.window.set_group(None, cx) {
                    Ok(()) => true,
                    Err(error) => {
                        tracing::warn!(
                            ?panel,
                            ?previous_group,
                            %error,
                            "could not detach a panel window; composition host transfer is deferred"
                        );
                        false
                    }
                };

                if !detached {
                    if let Some(group) = previous_group
                        && groups.iter().any(|candidate| candidate.id == group)
                    {
                        blocked_groups.insert(group);
                    }
                }

                let hidden = if let Some(native_window) = entry.native_window {
                    hide_attempted_panels.insert(panel);
                    match self.native_ops.hide(native_window) {
                        Ok(()) => true,
                        Err(error) => {
                            tracing::warn!(
                                ?panel,
                                %error,
                                "could not hide a panel window after it stopped hosting a composition"
                            );
                            false
                        }
                    }
                } else {
                    true
                };
                if hidden {
                    entry.visibility = PanelWindowVisibility::Hidden;
                }

                if let Some(native_window) = entry.native_window {
                    if entry.native_horizontal_resize_min_widths.is_some() {
                        if let Err(error) =
                            self.native_ops.set_horizontal_resize_min_widths(native_window, None)
                        {
                            tracing::warn!(
                                ?panel,
                                %error,
                                "could not clear native edge minimums after a panel stopped hosting"
                            );
                        }
                        entry.native_horizontal_resize_min_widths = None;
                    }
                    if entry.resize_enabled {
                        if let Err(error) = self.native_ops.disable_resize(native_window) {
                            tracing::warn!(
                                ?panel,
                                %error,
                                "could not disable native resize after a panel stopped hosting"
                            );
                        }
                        entry.resize_enabled = false;
                        entry.native_left_resize_enabled = None;
                    }
                    if entry.native_min_size.is_some() {
                        if let Err(error) = self.native_ops.set_min_size(native_window, None) {
                            tracing::warn!(
                                ?panel,
                                %error,
                                "could not clear the native minimum size after a panel stopped hosting"
                            );
                        }
                        entry.native_min_size = None;
                    }
                }

                // On a failed detach, retain the old GPUI host association.
                // A failed native hide also retains the previous visibility so
                // the next reconciliation retries it instead of trusting a
                // state that no longer matches the HWND.
                if detached {
                    entry.host_group = None;
                }
            }
        }

        self.registry.panel_locations = panel_locations;

        // No permanent FloatingComposition host is published. Typed Header/Group
        // members are the visible native representation from this stage onward.
        let mut active_hosts = HashMap::new();
        let mut monitor_corrections = Vec::new();
        for group in &groups {
            if blocked_groups.contains(&group.id) {
                continue;
            }
            let Some(panel) = next_hosts.get(&group.id).copied() else {
                continue;
            };
            let Some(mut entry) = self.registry.windows.remove(&panel) else {
                continue;
            };

            if entry.host_group != Some(group.id) {
                match entry.window.set_group(Some(group.id), cx) {
                    Ok(()) => entry.host_group = Some(group.id),
                    Err(error) => {
                        tracing::warn!(
                            ?panel,
                            ?group.id,
                            %error,
                            "could not assign a floating composition to its native host window"
                        );
                        self.registry.windows.insert(panel, entry);
                        continue;
                    }
                }
            }

            self.sync_window_bounds(&mut entry, group);
            if let Some(areas) = monitor_work_areas.as_deref()
                && let Some(corrected_bounds) =
                    self.restore_native_window_to_monitor(&mut entry, areas)
            {
                monitor_corrections.push((group.id, corrected_bounds));
                entry.bounds = corrected_bounds;
            }
            self.sync_native_resize(&mut entry, group);
            self.registry.windows.insert(panel, entry);
            active_hosts.insert(group.id, panel);
        }
        self.host_registry.floating_groups = active_hosts.clone();

        // First show/configure any replacement native hosts using the current
        // graph. Newly-created PanelWindows start hidden, so their frame metrics
        // are not eligible for reconnection validation until this pass finishes.
        self.synchronize_native_z_order(&groups, &active_hosts, &transferred_panels);
        self.synchronize_native_part_z_order(&groups);

        // Reconnect a straight-through pair only after its surviving native
        // hosts are visible and the proposed complete grid passes geometry
        // validation. A temporarily unavailable host leaves the candidate
        // queued for the next reconciliation pass.
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
                        "reconnected surviving WindowGrid members after removal"
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

        // The first pass made native handles visible; a successful graph change
        // requires applying the updated direction-aware Z-order a second time.
        if reconnected_grid {
            self.synchronize_native_z_order(&groups, &active_hosts, &transferred_panels);
        }
        self.schedule_pending_window_chain_alignment(cx);

        for (panel, entry) in &mut self.registry.windows {
            if entry.host_group.is_none()
                && entry.visibility == PanelWindowVisibility::Visible
                && !hide_attempted_panels.contains(panel)
            {
                let hidden = if let Some(native_window) = entry.native_window {
                    match self.native_ops.hide(native_window) {
                        Ok(()) => true,
                        Err(error) => {
                            tracing::warn!(
                                ?panel,
                                %error,
                                "could not hide a non-host panel window"
                            );
                            false
                        }
                    }
                } else {
                    true
                };
                if hidden {
                    entry.visibility = PanelWindowVisibility::Hidden;
                }
            }
        }

        // Legacy PanelId-host replacement path. The typed Header/Group registry
        // owns permanent Floating windows; this remains for any transitional
        // PanelId-host entry that may still be present.
        let removed_panels = self
            .registry
            .windows
            .keys()
            .filter(|panel| {
                !desired_panels.contains(panel)
            })
            .copied()
            .collect::<Vec<_>>();
        for panel in removed_panels {
            self.release_panel_window(panel, cx);
        }

        self.reconciling = false;
        if !monitor_corrections.is_empty() {
            let applied = self.controller.update(cx, |controller, controller_cx| {
                let applied = match controller.apply_initial_monitor_bounds(&monitor_corrections)
                {
                    Ok(()) => true,
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            "could not apply initial monitor recovery to the Core layout"
                        );
                        false
                    }
                };
                controller.flush_events(controller_cx);
                controller_cx.notify();
                applied
            });
            if applied {
                self.reconcile(cx);
            }
        }
    }

    /// Applies Core composition stacking and WindowGrid edge priority to all
    /// visible Header/Group HWNDs, raising members from back to front.
    fn synchronize_native_part_z_order(&self, groups: &[FloatingGroupSnapshot]) {
        let mut member_z_orders = Vec::new();
        for group in groups {
            if let Some(members) = self.part_members_by_group.get(&group.id) {
                member_z_orders.extend(members.iter().copied().map(|member| (member, group.z_order)));
            }
        }
        let ordered = grid_window_members_in_native_z_order(&member_z_orders, &self.window_chains);

        for member in ordered {
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

    /// Defers initial WindowChain frame alignment until the current reconcile update ends.
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

    /// Calculates seam corrections while the native windows and their frames are available.
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

            // Internal Header/Group links are already described by Core local
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
                if delta == Point::ZERO {
                    continue;
                }
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

    /// Applies Core's floating Z-order to native windows and stores each grid's
    /// direction-aware front-to-back order for activation-time correction.
    ///
    /// Windows are raised from back to front without activation. Left and upper
    /// members stay in front, preserving shared resize edges and physically
    /// covering the CompositionHeader of every lower member.
    fn synchronize_native_z_order(
        &mut self,
        groups: &[FloatingGroupSnapshot],
        active_hosts: &HashMap<FloatingGroupId, PanelId>,
        transferred_panels: &[PanelId],
    ) {
        let mut interaction_chain_members = HashSet::new();
        if let Some(session) = self.native_move {
            interaction_chain_members.extend(self.window_chains.members_for(window_grid_member(session.group)));
        }
        if let Some(session) = self.native_resize {
            interaction_chain_members.extend(self.window_chains.members_for(window_grid_member(session.group)));
        }

        for group_id in floating_groups_in_native_z_order(groups, &self.window_chains) {
            if interaction_chain_members.contains(&window_grid_member(group_id)) {
                continue;
            }
            let Some(group) = groups.iter().find(|group| group.id == group_id) else {
                continue;
            };
            let Some(panel) = active_hosts.get(&group.id).copied() else {
                continue;
            };
            let native_window = {
                let Some(entry) = self.registry.windows.get_mut(&panel) else {
                    continue;
                };
                // Legacy PanelId-host path: a transferred composition stays hidden
                // until DragHost has rendered its empty placeholder and released the
                // cached AnyViews. Permanent typed Group hosts use reconcile_part_windows.
                if entry.visibility == PanelWindowVisibility::Hidden
                    && group.panels.iter().any(|panel| transferred_panels.contains(panel))
                {
                    continue;
                }
                let Some(native_window) = entry.native_window else {
                    continue;
                };

                match self.native_ops.show_no_activate(native_window) {
                    Ok(()) => entry.visibility = PanelWindowVisibility::Visible,
                    Err(error) => {
                        tracing::warn!(
                            ?panel,
                            ?group.id,
                            %error,
                            "could not show the native window hosting a floating composition"
                        );
                        continue;
                    }
                }
                native_window
            };

            // Native activation restores this same front-to-back order by raising
            // its members in reverse, regardless of which HWND receives focus.
            let chain_windows = self
                .window_grid_members_in_native_order(group_id)
                .into_iter()
                .filter_map(|member| self.panel_window_for_grid_member(member)?.native_window)
                .collect::<Vec<_>>();
            if let Err(error) = self
                .native_ops
                .set_window_chain_z_order(native_window, &chain_windows)
            {
                tracing::warn!(
                    ?group_id,
                    %error,
                    "could not register native Floating chain Z-order"
                );
            }

            // ShowWindow(SW_SHOWNOACTIVATE) does not reorder an already-visible
            // HWND. Explicitly apply back-to-front order without changing focus.
            match self.native_ops.window_rect(native_window) {
                Ok(rect) => {
                    if let Err(error) = self.native_ops.move_window_to_top(
                        native_window,
                        PhysicalPoint {
                            x: rect.left,
                            y: rect.top,
                        },
                    ) {
                        tracing::warn!(
                            ?group_id,
                            %error,
                            "could not apply native Floating window Z-order"
                        );
                    }
                }
                Err(error) => tracing::warn!(
                    ?group_id,
                    %error,
                    "could not read native Floating bounds while applying Z-order"
                ),
            }
        }
    }

    /// Shows permanent Group hosts only after DragHost has stopped mounting the
    /// transferred AnyViews. This prevents an empty destination frame while the
    /// temporary source window still owns the content.
    pub(crate) fn show_handoff_target(
        &mut self,
        group: FloatingGroupId,
        cx: &mut Context<Self>,
    ) -> bool {
        // A DockArea tear-off can create several FloatingGroups. All their
        // Group windows remain hidden until the single DragHost releases its
        // source PanelViews, so reveal every hidden Group member together.
        let group_windows = self
            .registry
            .members
            .iter()
            .filter(|(member, entry)| {
                matches!(member, WindowGridMember::Group(_))
                    && entry.host_group.is_some()
            })
            .map(|(member, entry)| (*member, entry.window, entry.native_window, entry.visibility))
            .collect::<Vec<_>>();
        let mut ready_group_windows = 0usize;

        for (member, window, native_window, visibility) in group_windows {
            let Some(native_window) = native_window else {
                continue;
            };
            if visibility == PanelWindowVisibility::Hidden {
                if let Err(error) = self.native_ops.show_no_activate(native_window) {
                    tracing::warn!(
                        ?group,
                        ?member,
                        %error,
                        "could not show a Floating Group window during DragHost handoff"
                    );
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
                    "could not notify a Floating Group window after DragHost handoff"
                );
                continue;
            }
            ready_group_windows += 1;
        }

        if ready_group_windows > 0 {
            cx.notify();
            return true;
        }

        let Some(panel) = self.host_registry.panel_for_group(group) else {
            return false;
        };
        let Some((window, native_window, already_visible)) =
            self.registry.windows.get(&panel).and_then(|entry| {
                (entry.host_group == Some(group)).then_some((
                    entry.window,
                    entry.native_window?,
                    entry.visibility == PanelWindowVisibility::Visible,
                ))
            })
        else {
            return false;
        };
        if !already_visible {
            if let Err(error) = self.native_ops.show_no_activate(native_window) {
                tracing::warn!(
                    ?group,
                    ?panel,
                    %error,
                    "could not show the legacy FloatingHost during DragHost handoff"
                );
                return false;
            }
            if let Some(entry) = self.registry.windows.get_mut(&panel) {
                entry.visibility = PanelWindowVisibility::Visible;
            }
        }

        self.request_native_redraw(panel);
        if let Err(error) = window.notify_host(cx) {
            tracing::warn!(
                ?group,
                ?panel,
                %error,
                "could not notify the legacy FloatingHost after panel handoff"
            );
            return false;
        }

        cx.notify();
        true
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

    /// Requests a native paint for the persistent window hosting this panel.
    pub(crate) fn request_native_redraw(&self, panel: PanelId) {
        if let Some(native_window) = self
            .registry
            .windows
            .get(&panel)
            .and_then(|entry| entry.native_window)
            && let Err(error) = self.native_ops.request_redraw(native_window)
        {
            tracing::debug!(?panel, %error, "could not request a native redraw");
        }
    }

    /// Notifies and invalidates every visible Floating Header/Group host.
    pub(crate) fn redraw_visible_hosts(&self, cx: &mut Context<Self>) {
        let legacy_hosts = self
            .registry
            .windows
            .iter()
            .filter(|(_, entry)| {
                entry.host_group.is_some() && entry.visibility == PanelWindowVisibility::Visible
            })
            .map(|(panel, entry)| (*panel, entry.window))
            .collect::<Vec<_>>();

        for (panel, window) in legacy_hosts {
            if let Err(error) = window.notify_host(cx) {
                tracing::debug!(?panel, %error, "could not notify a legacy FloatingHost");
            }
            self.request_native_redraw(panel);
        }

        let member_hosts = self
            .registry
            .members
            .iter()
            .filter(|(_, entry)| entry.visibility == PanelWindowVisibility::Visible)
            .map(|(member, entry)| (*member, entry.window))
            .collect::<Vec<_>>();

        for (member, window) in member_hosts {
            if let Err(error) = window.notify_host(cx) {
                tracing::debug!(?member, %error, "could not notify a visible Floating part host");
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
            .part_members_by_group
            .get(&group)
            .into_iter()
            .flatten()
            .filter(|member| matches!(member, WindowGridMember::Group(_)))
            .find_map(|member| {
                self.registry.members.get(member).and_then(|entry| {
                    (entry.host_group == Some(group)
                        && entry.visibility == PanelWindowVisibility::Visible)
                        .then_some(entry.native_window)
                        .flatten()
                })
            })
            .or_else(|| {
                let panel = self.host_registry.panel_for_group(group)?;
                let entry = self.registry.windows.get(&panel)?;
                (entry.host_group == Some(group)
                    && entry.visibility == PanelWindowVisibility::Visible)
                    .then_some(entry.native_window)
                    .flatten()
            });
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
                "could not activate a Floating Group window after DragHost close"
            );
            return false;
        }

        matches!(
            self.native_ops.foreground_window(),
            Ok(Some(foreground)) if foreground == native_window
        )
    }

    /// Returns the transferred active panels belonging to one ready Group host.
    /// The host calls this during render before scheduling a next-frame handoff
    /// confirmation.
    pub(crate) fn pending_drag_host_handoff_panels(
        &self,
        group: FloatingGroupId,
        window_id: WindowId,
        cx: &mut Context<Self>,
    ) -> Option<Vec<PanelId>> {
        let group_member = self.registry.members.iter().find_map(|(member, entry)| {
            match member {
                WindowGridMember::Group(group_node)
                    if entry.window_id == window_id
                        && entry.host_group == Some(group)
                        && entry.visibility == PanelWindowVisibility::Visible =>
                {
                    Some(*group_node)
                }
                _ => None,
            }
        });

        if let Some(group_node) = group_member {
            let group_panels = self.controller.update(cx, |controller, _| {
                let mut panels = Vec::new();
                let mut seen = HashSet::new();
                collect_panel_order(controller.layout(), group_node, &mut panels, &mut seen);
                panels
            });
            let drag_host = self.drag_host_manager.as_ref()?.upgrade()?;
            let required = drag_host.update(cx, |manager, _| {
                manager.pending_handoff_panels(group)
            })?;
            let local_required = required
                .into_iter()
                .filter(|panel| group_panels.contains(panel))
                .collect::<Vec<_>>();
            return Some(local_required);
        }

        let host_panel = self.host_registry.panel_for_group(group)?;
        let entry = self.registry.windows.get(&host_panel)?;
        if entry.window_id != window_id
            || entry.host_group != Some(group)
            || entry.visibility != PanelWindowVisibility::Visible
        {
            return None;
        }
        let drag_host = self.drag_host_manager.as_ref()?.upgrade()?;
        drag_host.update(cx, |manager, _| manager.pending_handoff_panels(group))
    }

    /// Called after one Group host has rendered the transferred active PanelViews
    /// that belong to its Core Group. DragHostManager closes only after every
    /// required Group host across all detached FloatingGroups has rendered.
    pub(crate) fn notify_drag_host_handoff_rendered(
        &mut self,
        group: FloatingGroupId,
        window_id: WindowId,
        rendered_panels: &[PanelId],
        cx: &mut Context<Self>,
    ) {
        let part_member = self.registry.members.iter().find_map(|(member, entry)| {
            match member {
                WindowGridMember::Group(_)
                    if entry.window_id == window_id
                        && entry.host_group == Some(group)
                        && entry.visibility == PanelWindowVisibility::Visible =>
                {
                    Some(*member)
                }
                _ => None,
            }
        });

        if part_member.is_none() {
            let Some(host_panel) = self.host_registry.panel_for_group(group) else {
                return;
            };
            let Some(entry) = self.registry.windows.get(&host_panel) else {
                return;
            };
            if entry.window_id != window_id
                || entry.host_group != Some(group)
                || entry.visibility != PanelWindowVisibility::Visible
            {
                return;
            }
        }

        if let Some(drag_host) = self.drag_host_manager.as_ref().and_then(|manager| manager.upgrade()) {
            let _ = drag_host.update(cx, |manager, manager_cx| {
                manager.complete_floating_handoff(group, rendered_panels, manager_cx);
            });
        }

        if let Some(member) = part_member {
            self.request_member_redraw(member);
        } else if let Some(panel) = self.host_registry.panel_for_group(group) {
            self.request_native_redraw(panel);
        }
    }

    fn create_part_window(
        &mut self,
        spec: FloatingPartWindowSnapshot,
        cx: &mut Context<Self>,
    ) -> Option<PanelWindowEntry> {
        let controller = self.controller.clone();
        let theme = self.theme;
        let manager = cx.entity().downgrade();
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
            // Header/Group windows are positioned by FloatingWindowManager.
            // Their custom drag and resize behavior is connected in later stages.
            is_movable: false,
            app_owns_titlebar_drag: false,
            is_resizable: false,
            is_minimizable: false,
            ..Default::default()
        };

        let window = match cx.open_window(options, move |_, app| {
            let host = match spec.kind {
                FloatingPartKind::Header { column } => FloatingPartHost::new_header(
                    controller,
                    spec.floating_group,
                    column,
                    theme,
                    manager,
                ),
                FloatingPartKind::Group { column, group } => FloatingPartHost::new_group(
                    controller,
                    spec.floating_group,
                    column,
                    group,
                    theme,
                    manager,
                ),
            };
            app.new(|_| host)
        }) {
            Ok(window) => window,
            Err(error) => {
                tracing::error!(
                    ?spec.member,
                    %error,
                    "GPUI could not create a Floating Header/Group window"
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
                    "could not inspect the new Floating part window identifier"
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
                    "Floating part window has no native handle"
                );
                let _ = window.update(cx, |_, window, _| window.remove_window());
                return None;
            }
            Err(error) => {
                tracing::error!(
                    ?spec.member,
                    %error,
                    "could not inspect a newly created Floating part window"
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
                "could not create a Floating part window without the main native owner"
            );
            let _ = window.update(cx, |_, window, _| window.remove_window());
            return None;
        };
        if let Err(error) = self.native_ops.set_owner(native_window, owner) {
            tracing::error!(
                ?spec.member,
                %error,
                "could not assign the main window as Floating part owner"
            );
            let _ = window.update(cx, |_, window, _| window.remove_window());
            return None;
        }
        if !hide_from_taskbar(native_window) {
            tracing::warn!(
                ?spec.member,
                "could not hide a Floating part window from the taskbar"
            );
        }

        let scale = self
            .native_ops
            .scale_factor(native_window)
            .ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0);
        let outer_bounds = self
            .part_outer_bounds_for_target(spec, native_window, scale)
            .unwrap_or(spec.bounds);
        self.sync_part_window_bounds(native_window, outer_bounds);

        Some(PanelWindowEntry {
            window: PanelWindowHandle::Part(window),
            window_id,
            native_window: Some(native_window),
            bounds: outer_bounds,
            native_min_size: None,
            native_horizontal_resize_min_widths: None,
            native_left_resize_enabled: None,
            native_top_resize_enabled: None,
            native_scale_factor: scale,
            native_bounds_dirty: false,
            resize_enabled: false,
            host_group: Some(spec.floating_group),
            visibility: PanelWindowVisibility::Hidden,
        })
    }

    fn sync_part_window_bounds(&self, native_window: NativeWindowId, bounds: Rect) {
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
                "could not synchronize Floating part native bounds"
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

    /// Replaces legacy composition-only graph nodes with the real Header/Group
    /// members while preserving links between different Floating compositions.
    fn rebuild_floating_part_window_grid(
        &mut self,
        desired: &[FloatingPartWindowSnapshot],
    ) {
        let previous_links = self.window_chains.all_links();
        let mut owners = HashMap::<WindowGridMember, FloatingGroupId>::new();
        for spec in desired {
            owners.insert(spec.member, spec.floating_group);
        }

        let mut links = floating_part_internal_links(desired);
        for previous in previous_links {
            let from_group = match previous.from {
                WindowGridMember::FloatingComposition(group) => Some(group),
                member => owners.get(&member).copied(),
            };
            let to_group = match previous.to {
                WindowGridMember::FloatingComposition(group) => Some(group),
                member => owners.get(&member).copied(),
            };
            let (Some(from_group), Some(to_group)) = (from_group, to_group) else {
                continue;
            };
            if from_group == to_group {
                // Internal Header/Group edges are regenerated from the current
                // Core layout; keep only relationships between compositions.
                continue;
            }

            let from = match previous.from {
                WindowGridMember::FloatingComposition(group) => {
                    floating_part_boundary_member(desired, group, previous.direction)
                }
                member => owners.contains_key(&member).then_some(member),
            };
            let to = match previous.to {
                WindowGridMember::FloatingComposition(group) => {
                    floating_part_boundary_member(desired, group, previous.direction.opposite())
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
            // Any previously queued edge that is already present in the rebuilt
            // Core topology is no longer a reconnection candidate.
            let rebuilt_links = self.window_chains.all_links().into_iter().collect::<HashSet<_>>();
            self.pending_window_grid_reconnections
                .retain(|candidate| !rebuilt_links.contains(candidate));
        } else {
            tracing::warn!(
                "could not rebuild WindowGrid from the current Floating Header/Group topology"
            );
        }
    }

    fn reconcile_part_windows(
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
            collect_floating_part_windows(
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
            .filter_map(|(member, entry)| (!entry.window.is_available(cx)).then_some(*member))
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
                .entry(spec.floating_group)
                .or_default()
                .push(spec.member);
        }
        self.part_members_by_group = members_by_group;

        for spec in desired {
            if !self.registry.members.contains_key(&spec.member) {
                if let Some(entry) = self.create_part_window(spec, cx) {
                    self.registry.members.insert(spec.member, entry);
                } else {
                    continue;
                }
            }

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
            let target_bounds = native_window
                .and_then(|native_window| {
                    self.part_outer_bounds_for_target(spec, native_window, current_scale)
                })
                .unwrap_or_else(|| {
                    if native_window.is_some() { current_bounds } else { spec.bounds }
                });
            let scale_changed = current_scale.is_some_and(|scale| {
                previous_scale.is_none_or(|previous| (previous - scale).abs() > f32::EPSILON)
            });
            if current_bounds != target_bounds || native_bounds_dirty || scale_changed {
                if let Some(native_window) = native_window {
                    self.sync_part_window_bounds(native_window, target_bounds);
                }
                if let Some(entry) = self.registry.members.get_mut(&spec.member) {
                    entry.bounds = target_bounds;
                    entry.native_scale_factor = current_scale;
                    entry.native_bounds_dirty = false;
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
                            "could not show a Floating part window"
                        ),
                    }
                }
                (true, Some(PanelWindowVisibility::Visible), Some(native_window)) => {
                    if let Err(error) = self.native_ops.hide(native_window) {
                        tracing::warn!(
                            ?spec.member,
                            %error,
                            "could not hide a Floating part while its PanelViews are transported"
                        );
                    } else if let Some(entry) = self.registry.members.get_mut(&spec.member) {
                        entry.visibility = PanelWindowVisibility::Hidden;
                    }
                }
                _ => {}
            }
        }

        // Retain legacy FloatingComposition graph endpoints for this pass so
        // their external links can be migrated to real Header/Group boundaries.
        // Stale typed members are still pruned first, allowing WindowGrid to
        // produce straight-through candidates before the Core topology is rebuilt.
        let mut live_members = self.registry.members.keys().copied().collect::<HashSet<_>>();
        for link in self.window_chains.all_links() {
            for member in [link.from, link.to] {
                if matches!(member, WindowGridMember::FloatingComposition(_)) {
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

        self.rebuild_floating_part_window_grid(&desired);
        let rebuilt_links = self.window_chains.all_links().into_iter().collect::<HashSet<_>>();
        self.pending_window_grid_reconnections
            .retain(|candidate| !rebuilt_links.contains(candidate));

        prune_result
    }

    fn initial_panel_window_bounds(
        &self,
        panel: PanelId,
        location: PanelLayoutLocation,
        groups: &[FloatingGroupSnapshot],
        offset: usize,
        cx: &mut Context<Self>,
    ) -> Rect {
        if let PanelLayoutLocation::Floating(group) = location
            && let Some(group) = groups.iter().find(|candidate| candidate.id == group)
        {
            return group.bounds;
        }

        let (viewport, preferred_size, floating_count) =
            self.controller.update(cx, |controller, _| {
                let preferred_size = controller
                    .layout()
                    .panel_instance(panel)
                    .and_then(|instance| controller.registry().get(&instance.key))
                    .map(|descriptor| descriptor.constraints().preferred_size)
                    .unwrap_or(Size::new(280.0, 220.0));
                (
                    controller.viewport(),
                    preferred_size,
                    controller.layout().floating_groups().count(),
                )
            });

        let cascade = (floating_count.saturating_add(offset).min(7) as f32) * 24.0;
        let max_x =
            (viewport.origin.x + viewport.size.width - preferred_size.width).max(viewport.origin.x);
        let max_y =
            (viewport.origin.y + viewport.size.height - preferred_size.height)
                .max(viewport.origin.y);
        let x = (viewport.origin.x
            + (viewport.size.width - preferred_size.width).max(0.0) * 0.5
            + cascade)
            .min(max_x);
        let y = (viewport.origin.y
            + (viewport.size.height - preferred_size.height).max(0.0) * 0.5
            + cascade)
            .min(max_y);

        Rect::new(Point::new(x, y), preferred_size)
    }

    fn release_panel_window(&mut self, panel: PanelId, cx: &mut Context<Self>) {
        let Some(mut entry) = self.registry.windows.remove(&panel) else {
            self.registry.panel_locations.remove(&panel);
            return;
        };

        if let Some(native_window) = entry.native_window {
            if entry.native_horizontal_resize_min_widths.is_some() {
                if let Err(error) =
                    self.native_ops.set_horizontal_resize_min_widths(native_window, None)
                {
                    tracing::warn!(
                        ?panel,
                        %error,
                        "could not clear native edge minimums before destroying a panel window"
                    );
                }
                entry.native_horizontal_resize_min_widths = None;
            }
            if entry.resize_enabled {
                if let Err(error) = self.native_ops.disable_resize(native_window) {
                    tracing::warn!(
                        ?panel,
                        %error,
                        "could not disable native resize before destroying a panel window"
                    );
                }
                entry.resize_enabled = false;
                entry.native_left_resize_enabled = None;
            }
            if entry.native_min_size.is_some() {
                if let Err(error) = self.native_ops.set_min_size(native_window, None) {
                    tracing::warn!(
                        ?panel,
                        %error,
                        "could not clear the native minimum size before destroying a panel window"
                    );
                }
                entry.native_min_size = None;
            }
            if let Err(error) = self.native_ops.hide(native_window) {
                tracing::debug!(
                    ?panel,
                    %error,
                    "panel window was already hidden or unavailable during release"
                );
            }
        }

        entry.window.remove_window(cx);
    }

    fn create_panel_window(
        &mut self,
        panel: PanelId,
        bounds: Rect,
        cx: &mut Context<Self>,
    ) -> Option<PanelWindowEntry> {
        let controller = self.controller.clone();
        let panel_view_controller = self.controller.clone();
        let theme = self.theme;
        let manager = cx.entity().downgrade();
        let logical_origin = self.gpui_origin_for_bounds(bounds);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(logical_origin.x), px(logical_origin.y)),
                size(px(bounds.size.width), px(bounds.size.height)),
            ))),
            titlebar: None,
            focus: false,
            show: false,
            kind: WindowKind::Floating,
            is_movable: true,
            app_owns_titlebar_drag: true,
            is_resizable: true,
            is_minimizable: false,
            ..Default::default()
        };

        // The GPUI root is created hidden. reconcile assigns its composition,
        // configures the native surface, and only then makes this window visible.
        let window = match cx.open_window(options, move |window, app| {
            // Materialize every open PanelView before this hidden native
            // surface can be shown. This also creates inactive-tab views once;
            // all later hosts reuse the same PanelId -> AnyView cache.
            panel_view_controller.update(app, |controller, panel_cx| {
                controller.ensure_panel_views(window, panel_cx);
            });
            app.new(|_| FloatingHost::new_panel_window(controller, theme, manager))
        }) {
            Ok(window) => window,
            Err(error) => {
                tracing::error!(
                    ?panel,
                    %error,
                    "GPUI could not create a floating PanelWindow"
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
                    ?panel,
                    %error,
                    "could not inspect the new PanelWindow identifier"
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
                tracing::debug!(
                    ?panel,
                    %error,
                    "PanelWindow has no native handle"
                );
                None
            }
            Err(error) => {
                tracing::error!(
                    ?panel,
                    %error,
                    "could not inspect a newly created PanelWindow"
                );
                let _ = window.update(cx, |_, window, _| window.remove_window());
                return None;
            }
        };

        if self.main_native_window.is_some() && native_window.is_none() {
            // A hidden Windows floating window without an HWND cannot be shown or bound
            // to the application's native owner; do not register it as usable.
            let _ = window.update(cx, |_, window, _| window.remove_window());
            return None;
        }

        if let Some(native_window) = native_window {
            // Each floating host window is owned by MainWindow for its lifetime.
            // Dock placement and composition-host transfers never change its owner.
            let Some(owner) = self.main_native_window else {
                tracing::error!(
                    ?panel,
                    "could not create PanelWindow without the main native owner"
                );
                let _ = window.update(cx, |_, window, _| window.remove_window());
                return None;
            };
            if let Err(error) = self.native_ops.set_owner(native_window, owner) {
                tracing::error!(
                    ?panel,
                    %error,
                    "could not assign the main window as PanelWindow owner"
                );
                let _ = window.update(cx, |_, window, _| window.remove_window());
                return None;
            }
            if !hide_from_taskbar(native_window) {
                tracing::warn!(
                    ?panel,
                    "could not hide the newly created PanelWindow from the taskbar"
                );
            }
            if let Some(position) = self.physical_origin_for_bounds(bounds)
                && let Err(error) = self.native_ops.move_window(native_window, position)
            {
                tracing::warn!(
                    ?panel,
                    %error,
                    "could not place a newly created PanelWindow"
                );
            }
        }

        Some(PanelWindowEntry {
            window: PanelWindowHandle::Floating(window),
            window_id,
            native_window,
            bounds,
            native_min_size: None,
            native_horizontal_resize_min_widths: None,
            native_left_resize_enabled: None,
            native_top_resize_enabled: None,
            native_scale_factor: None,
            native_bounds_dirty: false,
            resize_enabled: false,
            host_group: None,
            visibility: PanelWindowVisibility::Hidden,
        })
    }

    /// Repositions a restored window only when its title area is no longer
    /// reachable on any connected monitor. Returns corrected Core bounds.
    fn restore_native_window_to_monitor(
        &self,
        entry: &mut PanelWindowEntry,
        work_areas: &[platform::PhysicalRect],
    ) -> Option<Rect> {
        let native_window = entry.native_window?;
        if work_areas.is_empty() {
            return None;
        }
        let current = self.native_ops.window_rect(native_window).ok()?;
        if native_rect_has_reachable_title_area(current, work_areas) {
            return None;
        }
        let work_area = self.preferred_monitor_work_area(work_areas)?;
        let position = restored_window_origin(current, work_area)?;
        if position.x == current.left && position.y == current.top {
            return None;
        }
        if let Err(error) = self.native_ops.move_window(native_window, position) {
            tracing::warn!(
                %error,
                "could not move an off-screen restored floating window onto a connected monitor"
            );
            return None;
        }

        let destination_scale = self
            .native_ops
            .scale_factor(native_window)
            .ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        if let Some(desired_size) =
            DockGeometry::physical_size_from_logical(entry.bounds.size, destination_scale)
            && let Err(error) = self.native_ops.resize_window(native_window, desired_size)
        {
            tracing::warn!(
                %error,
                "could not rescale a restored floating window for its destination monitor"
            );
        }

        let corrected = self.core_bounds_from_native_window(native_window)?;
        entry.native_scale_factor = Some(destination_scale);
        Some(corrected)
    }

    fn preferred_monitor_work_area(
        &self,
        work_areas: &[platform::PhysicalRect],
    ) -> Option<platform::PhysicalRect> {
        if let Some(main) = self.main_native_window
            && let Ok(rect) = self.native_ops.window_rect(main)
        {
            let center = PhysicalPoint {
                x: (rect.left as i64 + rect.right as i64).div_euclid(2) as i32,
                y: (rect.top as i64 + rect.bottom as i64).div_euclid(2) as i32,
            };
            if let Some(area) = work_areas.iter().copied().find(|area| area.contains(center)) {
                return Some(area);
            }
        }
        work_areas.first().copied()
    }

    fn sync_native_resize_edge_policy(
        &self,
        entry: &mut PanelWindowEntry,
        group: FloatingGroupId,
    ) {
        let Some(native_window) = entry.native_window else {
            return;
        };
        if !entry.resize_enabled {
            return;
        }

        let member = window_grid_member(group);
        let desired_left_enabled = !self
            .window_chains
            .has_neighbor_on_side(member, WindowGridDirection::Left);
        let desired_top_enabled = !self
            .window_chains
            .has_neighbor_on_side(member, WindowGridDirection::Above);

        if entry.native_left_resize_enabled != Some(desired_left_enabled) {
            match self
                .native_ops
                .set_left_resize_enabled(native_window, desired_left_enabled)
            {
                Ok(()) => entry.native_left_resize_enabled = Some(desired_left_enabled),
                Err(error) => tracing::warn!(
                    ?group,
                    %error,
                    "could not synchronize the native left-edge resize policy"
                ),
            }
        }

        if entry.native_top_resize_enabled != Some(desired_top_enabled) {
            match self
                .native_ops
                .set_top_resize_enabled(native_window, desired_top_enabled)
            {
                Ok(()) => entry.native_top_resize_enabled = Some(desired_top_enabled),
                Err(error) => tracing::warn!(
                    ?group,
                    %error,
                    "could not synchronize the native top-edge resize policy"
                ),
            }
        }
    }

    fn sync_native_resize(
        &self,
        entry: &mut PanelWindowEntry,
        group: &FloatingGroupSnapshot,
    ) {
        let Some(native_window) = entry.native_window else {
            return;
        };

        if !entry.resize_enabled {
            match self.native_ops.enable_resize(native_window) {
                Ok(()) => entry.resize_enabled = true,
                Err(error) => tracing::warn!(
                    ?group.id,
                    %error,
                    "could not enable native floating-window resize"
                ),
            }
        }

        self.sync_native_resize_edge_policy(entry, group.id);

        let scale = self
            .native_ops
            .scale_factor(native_window)
            .ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        let desired_minimum = group
            .minimum_size
            .and_then(|minimum| DockGeometry::physical_size_from_logical(minimum, scale));

        if entry.native_min_size != desired_minimum {
            match self.native_ops.set_min_size(native_window, desired_minimum) {
                Ok(()) => entry.native_min_size = desired_minimum,
                Err(error) => tracing::warn!(
                    ?group.id,
                    %error,
                    "could not synchronize native floating-window minimum size"
                ),
            }
        }

        let desired_edge_minimums = group.horizontal_resize_minimum_widths.and_then(
            |(left_width, right_width)| {
                Some((
                    DockGeometry::physical_coordinate(left_width, scale)?,
                    DockGeometry::physical_coordinate(right_width, scale)?,
                ))
            },
        );
        if entry.native_horizontal_resize_min_widths != desired_edge_minimums {
            match self
                .native_ops
                .set_horizontal_resize_min_widths(native_window, desired_edge_minimums)
            {
                Ok(()) => entry.native_horizontal_resize_min_widths = desired_edge_minimums,
                Err(error) => tracing::warn!(
                    ?group.id,
                    %error,
                    "could not synchronize side-specific native minimum widths"
                ),
            }
        }
    }

    /// Recomputes native minimum sizes from the live Core layout during an OS resize.
    ///
    /// LayoutChanged notifications can be coalesced with an active reconciliation.
    /// Polling the constraints alongside HWND bounds prevents Win32 from retaining
    /// an obsolete minimum width after one edge has already changed the composition.
    fn sync_native_resize_constraints(
        &mut self,
        group: FloatingGroupId,
        window_id: WindowId,
        cx: &mut Context<Self>,
    ) {
        let snapshot = self.controller.update(cx, |controller, _| {
            let layout = controller.layout();
            let metrics = controller.metrics();
            let floating = layout.floating_group(group)?;

            Some(FloatingGroupSnapshot {
                id: group,
                bounds: floating.bounds,
                z_order: floating.z_order,
                minimum_size: layout.floating_minimum_size(group, &metrics).ok(),
                horizontal_resize_minimum_widths: layout
                    .floating_horizontal_resize_minimum_widths(group, &metrics)
                    .ok()
                    .flatten(),
                panels: Vec::new(),
            })
        });
        let Some(snapshot) = snapshot else {
            return;
        };
        let Some(panel) = self.host_registry.panel_for_group(group) else {
            return;
        };
        let Some(mut entry) = self.registry.windows.remove(&panel) else {
            return;
        };

        if entry.window_id == window_id {
            self.sync_native_resize(&mut entry, &snapshot);
        }
        self.registry.windows.insert(panel, entry);
    }

    fn sync_window_bounds(
        &self,
        entry: &mut PanelWindowEntry,
        group: &FloatingGroupSnapshot,
    ) {
        let Some(native_window) = entry.native_window else {
            entry.native_scale_factor = None;
            entry.native_bounds_dirty = false;
            entry.bounds = group.bounds;
            return;
        };

        let Some(scale) = self
            .native_ops
            .scale_factor(native_window)
            .ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0)
        else {
            return;
        };
        let scale_changed = entry.native_scale_factor.is_none_or(|previous| {
            (previous - scale).abs() > 0.001
        });

        // While Windows owns the sizing loop, do not send Core bounds back to
        // the HWND. The final reconciliation applies the canonical Core bounds.
        if self
            .native_resize
            .is_some_and(|session| session.group == group.id)
        {
            entry.bounds = group.bounds;
            entry.native_scale_factor = Some(scale);
            entry.native_bounds_dirty = true;
            return;
        }

        if entry.bounds == group.bounds && !scale_changed && !entry.native_bounds_dirty {
            return;
        }

        // Apply position and client size in one native operation. For a
        // DockTab tear-off, Core moves the top edge upward and increases the
        // height by the same amount; one SetWindowPos keeps the bottom edge
        // anchored without visibly moving the whole window first.
        let Some(position) = self.physical_origin_for_bounds(group.bounds) else {
            tracing::warn!(
                ?group.id,
                "could not convert floating-window origin to physical coordinates"
            );
            return;
        };
        let Some(initial_size) = DockGeometry::physical_size_from_logical(
            group.bounds.size,
            scale,
        ) else {
            tracing::warn!(
                ?group.id,
                "could not convert floating-window size to physical coordinates"
            );
            return;
        };
        if let Err(error) =
            self.native_ops
                .set_window_bounds(native_window, position, initial_size)
        {
            tracing::warn!(
                ?group.id,
                %error,
                "could not synchronize floating native-window bounds"
            );
            return;
        }

        // SetWindowPos emits WM_SIZE/WM_MOVE on Windows. GPUI's platform
        // callbacks already refresh the cached viewport and notify bounds
        // observers; calling bounds_changed() here would re-enter FloatingHost.

        // Moving to another monitor can synchronously update the HWND DPI.
        // Correct the client size after the atomic bounds update if necessary.
        let effective_scale = self
            .native_ops
            .scale_factor(native_window)
            .ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0);
        let Some(effective_scale) = effective_scale else {
            // Keep the old cache so a later reconciliation retries the sync.
            return;
        };
        let effective_scale_changed = (scale - effective_scale).abs() > 0.001;

        if effective_scale_changed {
            let Some(desired_size) = DockGeometry::physical_size_from_logical(
                group.bounds.size,
                effective_scale,
            ) else {
                tracing::warn!(
                    ?group.id,
                    "could not convert floating-window size for the destination monitor"
                );
                return;
            };
            if let Err(error) = self.native_ops.resize_window(native_window, desired_size) {
                tracing::warn!(
                    ?group.id,
                    %error,
                    "could not correct floating-window size after a DPI change"
                );
                return;
            }
        }

        entry.native_scale_factor = Some(effective_scale);
        entry.bounds = group.bounds;
        entry.native_bounds_dirty = false;
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
    use super::{native_rect_has_reachable_title_area, restored_window_origin};
    use crate::geometry::DockGeometry;
    use dock_core::Point;
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
        floating_chain_drop_target_is_supported, floating_window_chain_direction,
        floating_window_drop_destination, NativeMoveTarget,
    };
    use crate::window_chain::WindowGridDirection;
    use dock_core::{DropTarget, DropZone, FloatingGroupId, NodeId, Point};
    use slotmap::Key;

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
            assert!(matches!(
                floating_window_drop_destination(FloatingGroupId::null(), target, Point::ZERO),
                NativeMoveTarget::FloatingChain {
                    direction: actual,
                    ..
                } if actual == direction
            ));
        }

        assert_eq!(floating_window_chain_direction(center), None);
        assert!(!floating_chain_drop_target_is_supported(center, false));
        assert!(matches!(
            floating_window_drop_destination(FloatingGroupId::null(), center, Point::ZERO),
            NativeMoveTarget::FloatingDock { target, .. } if target == center
        ));
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
mod floating_part_topology_tests {
    use super::{
        FloatingPartKind, FloatingPartWindowSnapshot, floating_part_boundary_member,
        floating_part_internal_links,
    };
    use crate::window_chain::{WindowGridDirection, WindowGridMember, WindowGridRegistry};
    use dock_core::{FloatingGroupId, NodeId, Point, Rect, Size};
    use slotmap::SlotMap;

    fn part(
        member: WindowGridMember,
        floating_group: FloatingGroupId,
        kind: FloatingPartKind,
        x: f32,
        y: f32,
    ) -> FloatingPartWindowSnapshot {
        FloatingPartWindowSnapshot {
            member,
            floating_group,
            kind,
            bounds: Rect::new(Point::new(x, y), Size::new(240.0, 180.0)),
            floating_origin: Point::new(100.0, 200.0),
            local_origin: Point::new(x, y),
            contains_transferred_panels: false,
        }
    }

    #[test]
    fn two_columns_build_one_header_row_and_two_independent_group_stacks() {
        let mut node_ids = SlotMap::<NodeId, ()>::with_key();
        let mut floating_ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let floating = floating_ids.insert(());
        let column_a = node_ids.insert(());
        let column_b = node_ids.insert(());
        let group_a1 = node_ids.insert(());
        let group_a2 = node_ids.insert(());
        let group_b1 = node_ids.insert(());
        let group_b2 = node_ids.insert(());

        let header_a = WindowGridMember::Header(column_a);
        let header_b = WindowGridMember::Header(column_b);
        let a1 = WindowGridMember::Group(group_a1);
        let a2 = WindowGridMember::Group(group_a2);
        let b1 = WindowGridMember::Group(group_b1);
        let b2 = WindowGridMember::Group(group_b2);

        let specs = [
            part(
                header_a,
                floating,
                FloatingPartKind::Header { column: column_a },
                0.0,
                0.0,
            ),
            part(
                a1,
                floating,
                FloatingPartKind::Group { column: column_a, group: group_a1 },
                0.0,
                40.0,
            ),
            part(
                a2,
                floating,
                FloatingPartKind::Group { column: column_a, group: group_a2 },
                0.0,
                220.0,
            ),
            part(
                header_b,
                floating,
                FloatingPartKind::Header { column: column_b },
                244.0,
                0.0,
            ),
            part(
                b1,
                floating,
                FloatingPartKind::Group { column: column_b, group: group_b1 },
                244.0,
                40.0,
            ),
            part(
                b2,
                floating,
                FloatingPartKind::Group { column: column_b, group: group_b2 },
                244.0,
                220.0,
            ),
        ];

        let links = floating_part_internal_links(&specs);
        assert_eq!(links.len(), 5);
        assert!(links.contains(&crate::window_chain::WindowGridLink {
            from: header_a,
            to: a1,
            direction: WindowGridDirection::Below,
        }));
        assert!(links.contains(&crate::window_chain::WindowGridLink {
            from: a1,
            to: a2,
            direction: WindowGridDirection::Below,
        }));
        assert!(links.contains(&crate::window_chain::WindowGridLink {
            from: header_a,
            to: header_b,
            direction: WindowGridDirection::Right,
        }));
        assert!(links.contains(&crate::window_chain::WindowGridLink {
            from: header_b,
            to: b1,
            direction: WindowGridDirection::Below,
        }));
        assert!(links.contains(&crate::window_chain::WindowGridLink {
            from: b1,
            to: b2,
            direction: WindowGridDirection::Below,
        }));

        let mut registry = WindowGridRegistry::default();
        assert!(registry.replace_all_links(&links));
        assert_eq!(registry.members_for(header_a).len(), 6);
        assert!(registry.is_valid());
    }

    #[test]
    fn composition_boundaries_resolve_to_outermost_header_and_group() {
        let mut node_ids = SlotMap::<NodeId, ()>::with_key();
        let mut floating_ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let floating = floating_ids.insert(());
        let column_a = node_ids.insert(());
        let column_b = node_ids.insert(());
        let group_a = node_ids.insert(());
        let group_b = node_ids.insert(());
        let header_a = WindowGridMember::Header(column_a);
        let header_b = WindowGridMember::Header(column_b);
        let top_group = WindowGridMember::Group(group_a);
        let bottom_group = WindowGridMember::Group(group_b);
        let specs = [
            part(
                header_a,
                floating,
                FloatingPartKind::Header { column: column_a },
                0.0,
                0.0,
            ),
            part(
                top_group,
                floating,
                FloatingPartKind::Group { column: column_a, group: group_a },
                0.0,
                40.0,
            ),
            part(
                header_b,
                floating,
                FloatingPartKind::Header { column: column_b },
                244.0,
                0.0,
            ),
            part(
                bottom_group,
                floating,
                FloatingPartKind::Group { column: column_b, group: group_b },
                244.0,
                40.0,
            ),
        ];

        assert_eq!(
            floating_part_boundary_member(&specs, floating, WindowGridDirection::Left),
            Some(header_a),
        );
        assert_eq!(
            floating_part_boundary_member(&specs, floating, WindowGridDirection::Right),
            Some(header_b),
        );
        assert_eq!(
            floating_part_boundary_member(&specs, floating, WindowGridDirection::Above),
            Some(header_a),
        );
        assert_eq!(
            floating_part_boundary_member(&specs, floating, WindowGridDirection::Below),
            Some(bottom_group),
        );
    }
}

//! Runtime owner of native windows for floating dock compositions.
//!
//! Main-dock panels do not allocate native surfaces. A composition gets one
//! host window only after it becomes a floating group. The shared DockController
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
    components::FloatingHost, drag_host_manager::DragHostSurface, geometry::DockGeometry,
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
    DragHost(WindowHandle<DragHostSurface>),
}

impl PanelWindowHandle {
    fn is_available(&self, cx: &mut Context<FloatingWindowManager>) -> bool {
        match self {
            Self::Floating(window) => window.update(cx, |_, _, _| ()).is_ok(),
            Self::DragHost(window) => window.update(cx, |_, _, _| ()).is_ok(),
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
            Self::DragHost(window) => {
                match window.update(cx, |surface, window, host_cx| {
                    let updated = surface.set_floating_group(group, window, host_cx);
                    host_cx.notify();
                    updated
                }) {
                    Ok(true) => Ok(()),
                    Ok(false) => Err("DragHost could not switch its Floating composition".into()),
                    Err(error) => Err(error.to_string()),
                }
            }
        }
    }

    /// Requests a redraw from the exact GPUI window that owns this host.
    fn notify_host(&self, cx: &mut Context<FloatingWindowManager>) -> Result<(), String> {
        match self {
            Self::Floating(window) => window
                .update(cx, |_, _, host_cx| host_cx.notify())
                .map_err(|error| error.to_string()),
            Self::DragHost(window) => window
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
            Self::DragHost(window) => {
                let _ = window.update(cx, |surface, window, surface_cx| {
                    let _ = surface.set_floating_group(None, window, surface_cx);
                    window.remove_window();
                });
            }
        }
    }

    /// Refreshes GPUI's internal viewport after the native HWND was resized directly.
    fn refresh_gpui_bounds(
        &self,
        cx: &mut Context<FloatingWindowManager>,
    ) -> Result<(), String> {
        match self {
            Self::Floating(window) => window
                .update(cx, |_, window, app| window.bounds_changed(app))
                .map_err(|error| error.to_string()),
            Self::DragHost(window) => window
                .update(cx, |_, window, app| window.bounds_changed(app))
                .map_err(|error| error.to_string()),
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

/// Native-window records keyed by the PanelId currently hosting a floating composition.
///
/// Main-dock panels have no native-window record. When a composition is floated,
/// one of its panels is selected as the host until that composition is docked or
/// the selected host panel leaves it.
#[derive(Default)]
pub struct PanelWindowRegistry {
    windows: HashMap<PanelId, PanelWindowEntry>,
    panel_locations: HashMap<PanelId, PanelLayoutLocation>,
}

impl PanelWindowRegistry {
    pub fn panel_count(&self) -> usize {
        self.windows.len()
    }

    pub fn visible_count(&self) -> usize {
        self.windows
            .values()
            .filter(|entry| entry.visibility == PanelWindowVisibility::Visible)
            .count()
    }

    pub fn window_for_panel(&self, panel: PanelId) -> Option<WindowHandle<FloatingHost>> {
        match self.windows.get(&panel)?.window {
            PanelWindowHandle::Floating(window) => Some(window),
            PanelWindowHandle::DragHost(_) => None,
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

/// Maps each Core floating composition to the panel window created on demand
/// to host it. The Main Dock is hosted by the one shared MainWindow.
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
        insert_before: bool,
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
struct NativeResizeSession {
    group: FloatingGroupId,
    window_id: WindowId,
    start_bounds: Rect,
    transaction_started: bool,
}


fn native_resize_right_edge_delta(
    previous: Rect,
    actual: Rect,
    scale_factor: f32,
) -> Option<f32> {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return None;
    }
    let previous_right = previous.origin.x + previous.size.width * scale_factor;
    let actual_right = actual.origin.x + actual.size.width * scale_factor;
    let delta = actual_right - previous_right;
    delta.is_finite().then_some(delta)
}

/// Shifts only the members after the resized window. Origins are stored in
/// physical desktop pixels; every member's own logical dimensions stay intact.
fn shift_window_chain_after_resize(
    mut bounds: Vec<(FloatingGroupId, Rect)>,
    resized_group: FloatingGroupId,
    right_edge_delta: f32,
) -> Option<Vec<(FloatingGroupId, Rect)>> {
    if !right_edge_delta.is_finite() {
        return None;
    }
    let resized_index = bounds
        .iter()
        .position(|(group, _)| *group == resized_group)?;
    for (_, bounds) in bounds.iter_mut().skip(resized_index + 1) {
        bounds.origin.x += right_edge_delta;
        if !bounds.origin.x.is_finite() {
            return None;
        }
    }
    Some(bounds)
}

fn floating_groups_in_native_z_order(
    groups: &[FloatingGroupSnapshot],
    chains: &WindowChainRegistry,
) -> Vec<FloatingGroupId> {
    let mut emitted = HashSet::new();
    let mut blocks = Vec::new();

    for group in groups {
        if emitted.contains(&group.id) {
            continue;
        }

        let members = chains.members_for(group.id);
        let block = members
            .into_iter()
            .filter(|member| {
                !emitted.contains(member)
                    && groups.iter().any(|candidate| candidate.id == *member)
            })
            .collect::<Vec<_>>();
        if block.is_empty() {
            continue;
        }

        let block_z_order = block
            .iter()
            .filter_map(|member| {
                groups
                    .iter()
                    .find(|candidate| candidate.id == *member)
                    .map(|candidate| candidate.z_order)
            })
            .max()
            .unwrap_or(group.z_order);

        emitted.extend(block.iter().copied());
        blocks.push((block_z_order, block));
    }

    blocks.sort_by_key(|(z_order, _)| *z_order);
    blocks
        .into_iter()
        .flat_map(|(_, mut members)| {
            // The sequence is applied back-to-front with HWND_TOP. Reverse each
            // left-to-right chain so the left member ends above its right neighbour.
            members.reverse();
            members
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct NativeFrameGeometry {
    outer_width: f32,
    visible_left_inset: f32,
    visible_right_inset: f32,
}

impl NativeFrameGeometry {
    fn from_rects(outer: platform::PhysicalRect, visible: platform::PhysicalRect) -> Option<Self> {
        let outer_width = i64::from(outer.right) - i64::from(outer.left);
        let visible_left_inset = i64::from(visible.left) - i64::from(outer.left);
        let visible_right_inset = i64::from(outer.right) - i64::from(visible.right);

        if outer_width <= 0
            || visible_left_inset < 0
            || visible_right_inset < 0
            || visible_left_inset + visible_right_inset >= outer_width
        {
            return None;
        }

        Some(Self {
            outer_width: outer_width as f32,
            visible_left_inset: visible_left_inset as f32,
            visible_right_inset: visible_right_inset as f32,
        })
    }

    fn without_frame(outer_width: f32) -> Option<Self> {
        (outer_width.is_finite() && outer_width > 0.0).then_some(Self {
            outer_width,
            visible_left_inset: 0.0,
            visible_right_inset: 0.0,
        })
    }
}

fn arrange_contiguous_window_chain_bounds(
    mut bounds: Vec<(FloatingGroupId, Rect)>,
    frames: &[NativeFrameGeometry],
    anchor: FloatingGroupId,
) -> Option<Vec<(FloatingGroupId, Rect)>> {
    if bounds.len() != frames.len()
        || frames.iter().any(|frame| {
            !frame.outer_width.is_finite()
                || frame.outer_width <= 0.0
                || !frame.visible_left_inset.is_finite()
                || !frame.visible_right_inset.is_finite()
                || frame.visible_left_inset < 0.0
                || frame.visible_right_inset < 0.0
                || frame.visible_left_inset + frame.visible_right_inset >= frame.outer_width
        })
    {
        return None;
    }

    let anchor_index = bounds.iter().position(|(group, _)| *group == anchor)?;
    let anchor_origin = bounds[anchor_index].1.origin;

    // Keep the anchor at its drop position. Position adjacent windows so that
    // their DWM-visible edges touch, accounting for the invisible resize border
    // around each HWND.
    for index in (0..anchor_index).rev() {
        let next = frames[index + 1];
        let current = frames[index];
        bounds[index].1.origin.x = bounds[index + 1].1.origin.x
            + next.visible_left_inset
            - current.outer_width
            + current.visible_right_inset;
        bounds[index].1.origin.y = anchor_origin.y;
    }
    for index in anchor_index + 1..bounds.len() {
        let previous = frames[index - 1];
        let current = frames[index];
        bounds[index].1.origin.x = bounds[index - 1].1.origin.x
            + previous.outer_width
            - previous.visible_right_inset
            - current.visible_left_inset;
        bounds[index].1.origin.y = anchor_origin.y;
    }

    Some(bounds)
}

fn floating_window_chain_insert_before(target: DropTarget) -> Option<bool> {
    match target {
        DropTarget::GroupEdge {
            zone: dock_core::DropZone::Left,
            ..
        } => Some(true),
        DropTarget::GroupEdge {
            zone: dock_core::DropZone::Right,
            ..
        } => Some(false),
        _ => None,
    }
}

fn floating_window_drop_destination(
    anchor: FloatingGroupId,
    target: DropTarget,
    point: Point,
) -> NativeMoveTarget {
    match floating_window_chain_insert_before(target) {
        Some(insert_before) => NativeMoveTarget::FloatingChain {
            anchor,
            insert_before,
        },
        None => NativeMoveTarget::FloatingDock { target, point },
    }
}

/// Ordered groups of independent native Floating windows.
///
/// The first member is used as the registry key. Removing that member simply
/// re-keys the remaining chain; no additional runtime identifier is required.
/// Single windows are intentionally not stored as chains.
#[derive(Clone, Default)]
struct WindowChainRegistry {
    chains: HashMap<FloatingGroupId, Vec<FloatingGroupId>>,
    chain_by_group: HashMap<FloatingGroupId, FloatingGroupId>,
}

impl WindowChainRegistry {
    fn chain_for(&self, group: FloatingGroupId) -> Option<&[FloatingGroupId]> {
        let chain_id = *self.chain_by_group.get(&group)?;
        self.chains.get(&chain_id).map(Vec::as_slice)
    }

    /// Returns a chain's ordered members, treating an unlinked window as a
    /// one-member chain for move operations.
    fn members_for(&self, group: FloatingGroupId) -> Vec<FloatingGroupId> {
        self.chain_for(group)
            .map_or_else(|| vec![group], |members| members.to_vec())
    }

    fn link_before(
        &mut self,
        anchor: FloatingGroupId,
        moving: FloatingGroupId,
    ) -> bool {
        self.link_at(anchor, moving, false)
    }

    fn link_after(
        &mut self,
        anchor: FloatingGroupId,
        moving: FloatingGroupId,
    ) -> bool {
        self.link_at(anchor, moving, true)
    }

    /// Inserts the complete moving chain immediately before/after the anchor
    /// member, preserving the internal order of both chains.
    fn link_at(
        &mut self,
        anchor: FloatingGroupId,
        moving: FloatingGroupId,
        after: bool,
    ) -> bool {
        if anchor == moving {
            return false;
        }

        let anchor_chain = self.chain_by_group.get(&anchor).copied();
        let moving_chain = self.chain_by_group.get(&moving).copied();
        if anchor_chain.is_some() && anchor_chain == moving_chain {
            return false;
        }

        let mut anchor_members = self.members_for(anchor);
        let moving_members = self.members_for(moving);
        let Some(anchor_index) = anchor_members.iter().position(|member| *member == anchor)
        else {
            return false;
        };

        self.take_chain_for(anchor);
        self.take_chain_for(moving);

        let insertion_index = if after {
            anchor_index + 1
        } else {
            anchor_index
        };
        anchor_members.splice(insertion_index..insertion_index, moving_members);
        self.store_chain(anchor_members);
        true
    }

    /// Inserts an ordered group of independent windows before/after an anchor.
    ///
    /// The entire operation is applied to a clone so a failed link cannot leave
    /// either the source sequence or destination chain partially rewritten.
    fn link_sequence_at(
        &mut self,
        anchor: FloatingGroupId,
        moving: &[FloatingGroupId],
        insert_before: bool,
    ) -> bool {
        if moving.is_empty() {
            return false;
        }
        let mut seen = HashSet::new();
        if moving.iter().any(|group| *group == anchor || !seen.insert(*group)) {
            return false;
        }

        let mut next = self.clone();
        for pair in moving.windows(2) {
            if !next.link_after(pair[0], pair[1]) {
                return false;
            }
        }
        let linked = if insert_before {
            next.link_before(anchor, moving[0])
        } else {
            next.link_after(anchor, moving[0])
        };
        if !linked {
            return false;
        }

        *self = next;
        true
    }

    /// Removes one window from its chain. The remaining members keep their
    /// order; zero- and one-member results are not retained as chains.
    fn remove_member(&mut self, group: FloatingGroupId) -> bool {
        let Some(chain_id) = self.chain_by_group.get(&group).copied() else {
            return false;
        };
        let Some(mut members) = self.chains.remove(&chain_id) else {
            self.chain_by_group.remove(&group);
            return false;
        };

        for member in &members {
            self.chain_by_group.remove(member);
        }
        members.retain(|member| *member != group);
        self.store_chain(members);
        true
    }

    /// Drops links to FloatingGroups that no longer exist in Core Layout.
    ///
    /// Returns the number of removed members and the surviving chains whose
    /// adjacency may need to be compacted after an extracted source disappears.
    fn retain_groups(
        &mut self,
        live_groups: &HashSet<FloatingGroupId>,
    ) -> (usize, Vec<Vec<FloatingGroupId>>) {
        let previous_chains = self.chains.values().cloned().collect::<Vec<_>>();
        self.chains.clear();
        self.chain_by_group.clear();

        let mut removed = 0;
        let mut chains_to_compact = Vec::new();
        for mut members in previous_chains {
            let previous_len = members.len();
            members.retain(|member| live_groups.contains(member));
            removed += previous_len - members.len();
            if members.len() > 1 && members.len() != previous_len {
                chains_to_compact.push(members.clone());
            }
            self.store_chain(members);
        }
        (removed, chains_to_compact)
    }

    fn take_chain_for(&mut self, group: FloatingGroupId) {
        let Some(chain_id) = self.chain_by_group.remove(&group) else {
            return;
        };
        if let Some(members) = self.chains.remove(&chain_id) {
            for member in members {
                self.chain_by_group.remove(&member);
            }
        }
    }

    fn store_chain(&mut self, mut members: Vec<FloatingGroupId>) {
        let mut seen = HashSet::new();
        members.retain(|member| seen.insert(*member));
        if members.len() < 2 {
            return;
        }

        let chain_id = members[0];
        for member in &members {
            self.chain_by_group.insert(*member, chain_id);
        }
        self.chains.insert(chain_id, members);
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
    registry: PanelWindowRegistry,
    host_registry: CompositionHostRegistry,
    window_chains: WindowChainRegistry,
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
            registry: PanelWindowRegistry::default(),
            host_registry: CompositionHostRegistry::default(),
            window_chains: WindowChainRegistry::default(),
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

    /// Returns ordered members of a chain. An unlinked Floating window is
    /// represented as a one-member chain for callers that move windows.
    #[allow(dead_code)] // Consumed by the chain-wide move stage.
    pub(crate) fn window_chain_members(
        &self,
        group: FloatingGroupId,
    ) -> Vec<FloatingGroupId> {
        self.window_chains.members_for(group)
    }

    /// Inserts the moving window's complete chain immediately before the anchor.
    #[allow(dead_code)] // Consumed by the whole-window docking stage.
    pub(crate) fn link_window_chain_before(
        &mut self,
        anchor: FloatingGroupId,
        moving: FloatingGroupId,
    ) -> bool {
        self.window_chains.link_before(anchor, moving)
    }

    /// Inserts the moving window's complete chain immediately after the anchor.
    #[allow(dead_code)] // Consumed by the whole-window docking stage.
    pub(crate) fn link_window_chain_after(
        &mut self,
        anchor: FloatingGroupId,
        moving: FloatingGroupId,
    ) -> bool {
        self.window_chains.link_after(anchor, moving)
    }

    /// Removes a Floating window from its chain without disturbing other members.
    #[allow(dead_code)] // Consumed by the content-extraction lifecycle stage.
    pub(crate) fn remove_window_from_chain(
        &mut self,
        group: FloatingGroupId,
    ) -> bool {
        self.window_chains.remove_member(group)
    }

    /// Registers the ordered Core result of one Main DockArea tear-off.
    ///
    /// This method deliberately does not reconcile: DragHost must establish
    /// the chain before its native HWND is adopted and reconciliation creates
    /// the remaining persistent Floating windows.
    pub(crate) fn register_window_chain(
        &mut self,
        groups: &[FloatingGroupId],
    ) -> bool {
        if groups.is_empty() {
            return false;
        }
        if groups.len() == 1 {
            return true;
        }

        let mut seen = HashSet::new();
        if !groups.iter().all(|group| seen.insert(*group)) {
            return false;
        }

        let mut next = self.window_chains.clone();
        for pair in groups.windows(2) {
            if !next.link_after(pair[0], pair[1]) {
                return false;
            }
        }
        self.window_chains = next;
        true
    }

    /// Joins newly-created Floating windows to an existing native window chain.
    ///
    /// The source groups remain separate Core FloatingGroups and separate HWNDs.
    /// Their final positions are calculated from each native frame so the visible
    /// DWM edges touch the selected side of the anchor window.
    pub(crate) fn join_window_chain_at(
        &mut self,
        anchor: FloatingGroupId,
        groups: &[FloatingGroupId],
        insert_before: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), dock_core::CommandError> {
        let mut next_chains = self.window_chains.clone();
        if !next_chains.link_sequence_at(anchor, groups, insert_before) {
            return Err(dock_core::CommandError::InvalidTarget {
                reason: "could not insert the new Floating windows into the target WindowChain",
            });
        }

        let members = next_chains.members_for(anchor);
        let Some(member_bounds) = self.controller.update(cx, |controller, _| {
            members
                .iter()
                .map(|member| {
                    controller
                        .layout()
                        .floating_group(*member)
                        .map(|floating| (*member, floating.bounds))
                })
                .collect::<Option<Vec<_>>>()
        }) else {
            return Err(dock_core::CommandError::InvalidTarget {
                reason: "WindowChain contains a FloatingGroup that no longer exists",
            });
        };

        let frame_geometries = self.frame_geometries_for_bounds(&member_bounds);
        let Some(bounds) =
            arrange_contiguous_window_chain_bounds(member_bounds, &frame_geometries, anchor)
        else {
            return Err(dock_core::CommandError::InvalidTarget {
                reason: "could not calculate native frame geometry for the new WindowChain",
            });
        };

        let current_bounds = self.controller.update(cx, |controller, _| {
            bounds
                .iter()
                .map(|(group, _)| {
                    controller
                        .layout()
                        .floating_group(*group)
                        .map(|floating| (*group, floating.bounds))
                })
                .collect::<Option<Vec<_>>>()
        }).ok_or(dock_core::CommandError::InvalidTarget {
            reason: "WindowChain changed while preparing its native bounds",
        })?;
        let commands = bounds
            .iter()
            .filter_map(|(group, bounds)| {
                current_bounds
                    .iter()
                    .find(|(current_group, _)| current_group == group)
                    .filter(|(_, current)| current != bounds)
                    .map(|_| DockCommand::ResizeFloatingBounds {
                        group: *group,
                        bounds: *bounds,
                    })
            })
            .collect::<Vec<_>>();

        self.window_chains = next_chains;
        for member in members {
            if let Some(panel) = self.host_registry.panel_for_group(member)
                && let Some(mut entry) = self.registry.windows.remove(&panel)
            {
                self.sync_native_resize_edge_policy(&mut entry, member);
                self.registry.windows.insert(panel, entry);
            }
        }
        if commands.is_empty() {
            self.reconcile(cx);
        } else {
            self.schedule_window_chain_bounds_update(commands, cx);
        }
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

    /// Registers an already-open DragHost as the native host for a newly floated DockArea.
    /// The HWND is transferred to the persistent registry instead of opening another window.
    pub(crate) fn adopt_drag_host_window(
        &mut self,
        panel: PanelId,
        group: FloatingGroupId,
        window: WindowHandle<DragHostSurface>,
        window_id: WindowId,
        native_window: NativeWindowId,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.registry.windows.contains_key(&panel) {
            return false;
        }
        let Some(bounds) = self.controller.update(cx, |controller, _| {
            controller.layout().floating_group(group).map(|floating| floating.bounds)
        }) else {
            return false;
        };
        // The DragHost was created before the Floating composition header
        // existed. Start reconciliation from its actual native bounds so the
        // newly reserved header height moves the top edge upward and increases
        // the client height instead of shifting the detached tab downward.
        let current_native_bounds = self
            .core_bounds_from_native_window(native_window)
            .unwrap_or(bounds);

        let configured = window.update(cx, |surface, native, surface_cx| {
            surface.set_floating_group(Some(group), native, surface_cx)
        });
        if !matches!(configured, Ok(true)) {
            return false;
        }

        let scale = self.native_ops.scale_factor(native_window).ok();
        self.registry.windows.insert(
            panel,
            PanelWindowEntry {
                window: PanelWindowHandle::DragHost(window),
                window_id,
                native_window: Some(native_window),
                bounds: current_native_bounds,
                native_min_size: None,
                native_horizontal_resize_min_widths: None,
                native_left_resize_enabled: None,
                native_top_resize_enabled: None,
                native_scale_factor: scale,
            native_bounds_dirty: false,
                resize_enabled: false,
                host_group: Some(group),
                visibility: PanelWindowVisibility::Visible,
            },
        );
        // Preserve the adopted panel as the selected host during the first
        // reconciliation; otherwise the normal first-panel selection may open
        // a second native window for the same composition.
        self.host_registry.floating_groups.insert(group, panel);
        self.reconcile(cx);

        self.host_registry.floating_groups.get(&group) == Some(&panel)
            && self.registry.windows.get(&panel).is_some_and(|entry| {
                entry.host_group == Some(group)
                    && entry.native_window == Some(native_window)
                    && entry.window_id == window_id
            })
    }

    fn panel_window_for_group(&self, group: FloatingGroupId) -> Option<&PanelWindowEntry> {
        let panel = self.host_registry.panel_for_group(group)?;
        self.registry.windows.get(&panel)
    }

    /// Returns the outer HWND width and the inset to each visible frame edge.
    /// GetWindowRect includes invisible resize borders; DWM's extended frame
    /// bounds provide the visible edges used for a true visual edge-to-edge join.
    fn frame_geometries_for_bounds(
        &self,
        bounds: &[(FloatingGroupId, Rect)],
    ) -> Vec<NativeFrameGeometry> {
        bounds
            .iter()
            .map(|(group, bounds)| {
                let Some(entry) = self.panel_window_for_group(*group) else {
                    return NativeFrameGeometry::without_frame(bounds.size.width)
                        .unwrap_or(NativeFrameGeometry {
                            outer_width: bounds.size.width.max(1.0),
                            visible_left_inset: 0.0,
                            visible_right_inset: 0.0,
                        });
                };
                let Some(native_window) = entry.native_window else {
                    let scale = entry.native_scale_factor.unwrap_or(1.0);
                    return NativeFrameGeometry::without_frame(bounds.size.width * scale)
                        .unwrap_or(NativeFrameGeometry {
                            outer_width: bounds.size.width,
                            visible_left_inset: 0.0,
                            visible_right_inset: 0.0,
                        });
                };

                let outer = self.native_ops.window_rect(native_window).ok();
                let visible = self.native_ops.visible_frame_rect(native_window).ok();
                if let (Some(outer), Some(visible)) = (outer, visible)
                    && let Some(geometry) = NativeFrameGeometry::from_rects(outer, visible)
                {
                    return geometry;
                }

                let scale = self
                    .native_ops
                    .scale_factor(native_window)
                    .ok()
                    .or(entry.native_scale_factor)
                    .filter(|scale| scale.is_finite() && *scale > 0.0)
                    .unwrap_or(1.0);
                NativeFrameGeometry::without_frame(bounds.size.width * scale)
                    .unwrap_or(NativeFrameGeometry {
                        outer_width: bounds.size.width,
                        visible_left_inset: 0.0,
                        visible_right_inset: 0.0,
                    })
            })
            .collect()
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
                    if matches!(event, DockEvent::LayoutChanged { .. }) {
                        manager.reconcile(manager_cx);
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
            self.registry
                .windows
                .values()
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
            self.registry.windows.values().map(|entry| entry.native_window),
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
        let chain_members = self.window_chains.members_for(group);
        self.controller.update(cx, |controller, controller_cx| {
            let primary_result = controller.dispatch(command);
            match primary_result {
                Ok(_) => {
                    let actual_bounds = controller
                        .layout()
                        .floating_group(group)
                        .map(|floating| floating.bounds);
                    if let Some(actual_bounds) = actual_bounds
                        && let Some(right_edge_delta) = native_resize_right_edge_delta(
                            current_bounds,
                            actual_bounds,
                            scale_factor,
                        )
                        && right_edge_delta.abs() > 0.01
                    {
                        let chain_bounds = chain_members
                            .iter()
                            .map(|member| {
                                controller
                                    .layout()
                                    .floating_group(*member)
                                    .map(|floating| (*member, floating.bounds))
                            })
                            .collect::<Option<Vec<_>>>();
                        if let Some(chain_bounds) = chain_bounds
                            && let Some(shifted_bounds) = shift_window_chain_after_resize(
                                chain_bounds,
                                group,
                                right_edge_delta,
                            )
                        {
                            let neighbor_commands = shifted_bounds
                                .into_iter()
                                .skip_while(|(member, _)| *member != group)
                                .skip(1)
                                .map(|(member, bounds)| DockCommand::ResizeFloatingBounds {
                                    group: member,
                                    bounds,
                                })
                                .collect::<Vec<_>>();
                            if let Err(error) = controller.dispatch_sequence(neighbor_commands) {
                                tracing::warn!(
                                    ?group,
                                    %error,
                                    "could not shift later Window Chain members after native Resize"
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

            // The current resize and every dependent neighbour shift stay inside
            // the same open native-resize history transaction and one event flush.
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
        if state.transaction_started {
            self.controller.update(cx, |controller, controller_cx| {
                controller.finish_native_resize_transaction();
                controller_cx.notify();
            });
        }

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
        for member in self.window_chains.members_for(group) {
            let Some(member_entry) = self.panel_window_for_group(member) else {
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
                group: member,
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
        known.extend(self.registry.windows.values().filter_map(|entry| {
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

            // Resolve the target Floating's own side zones so the destination
            // window paints the same affordance used to choose the WindowChain side.
            // The Core composition is not docked into the target; only HWND bounds
            // and WindowChain membership are changed after release.
            let resolved_target = self.controller.update(cx, |controller, controller_cx| {
                let resolved = controller
                    .prepare_floating_window_drop_resolver(target_window_id, target_group, source)
                    .and_then(|resolver| resolver.resolve(point));
                let target = match resolved {
                    // Keep every legal target from the Floating resolver. Left/Right
                    // are interpreted as native WindowChain insertion on release;
                    // all other zones remain ordinary Core docking destinations.
                    Ok(target) => target,
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
        &self,
        screen: PhysicalPoint,
        drag_host: NativeWindowId,
        excluded_source_window: Option<WindowId>,
    ) -> Option<DockingWindowAtPoint> {
        let Some(main_window_id) = self.host_registry.main_window_id else {
            return Some(DockingWindowAtPoint::ForeignWindow);
        };
        let Some(main_native_window) = self.main_native_window else {
            return Some(DockingWindowAtPoint::ForeignWindow);
        };
        let mut known_windows = Vec::with_capacity(self.registry.windows.len() + 1);

        if excluded_source_window != Some(main_window_id) {
            known_windows.push(main_native_window);
        }
        known_windows.extend(self.registry.windows.values().filter_map(|entry| {
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

    /// Returns true when the cursor is over a registered FloatingHost above MainHost.
    pub(crate) fn cursor_over_floating_window_from_main(&self) -> bool {
        let Some(main) = self.main_native_window else {
            return false;
        };
        let Ok(cursor) = self.native_ops.cursor_position() else {
            return false;
        };
        let mut known = vec![main];
        known.extend(self.registry.windows.values().filter_map(|entry| {
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
    ) -> Option<Point> {
        match source_host {
            DragSourceHost::Main => {
                if source_window_id.is_some_and(|id| Some(id) != self.host_registry.main_window_id) {
                    return None;
                }
                self.core_point_for_main(screen)
            }
            DragSourceHost::Floating { group } => {
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

    fn finish_native_move(&mut self, state: NativeMoveSession, cx: &mut Context<Self>) {
        if state.high_resolution_timer {
            let _ = self.native_ops.end_high_resolution_timer();
        }

        let moving_windows = std::mem::take(&mut self.native_move_windows);
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
                insert_before,
            }) => self.finish_floating_window_chain_move(
                state.group,
                anchor,
                insert_before,
                cx,
            ),
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
            self.reconcile(cx);
        }

        self.redraw_visible_hosts(cx);
        cx.notify();
    }

    fn native_bounds_for_move_windows(
        &self,
        moving_windows: &[NativeMoveWindow],
        cx: &mut Context<Self>,
    ) -> Option<Vec<(FloatingGroupId, Rect)>> {
        let mut bounds = self.controller.update(cx, |controller, _| {
            moving_windows
                .iter()
                .map(|moving| {
                    controller
                        .layout()
                        .floating_group(moving.group)
                        .map(|floating| (moving.group, floating.bounds))
                })
                .collect::<Option<Vec<_>>>()
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

    /// Joins whole native Floating windows without changing their Core ownership.
    /// The candidate chain becomes live only after all contiguous Core bounds have
    /// committed as one history transaction.
    fn finish_floating_window_chain_move(
        &mut self,
        moving: FloatingGroupId,
        anchor: FloatingGroupId,
        insert_before: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), dock_core::CommandError> {
        let mut next_chains = self.window_chains.clone();
        let linked = if insert_before {
            next_chains.link_before(anchor, moving)
        } else {
            next_chains.link_after(anchor, moving)
        };
        if !linked {
            // Dropping inside the same chain does not redefine membership. Until
            // chain-wide movement is enabled, restore the dragged member rather
            // than leaving its neighbours detached from their stored order.
            self.mark_group_native_bounds_dirty(moving);
            self.cancel_controller_move(cx);
            self.reconcile(cx);
            return Ok(());
        }

        let members = next_chains.members_for(anchor);
        let Some(member_bounds) = self.controller.update(cx, |controller, _| {
            members
                .iter()
                .map(|member| {
                    controller
                        .layout()
                        .floating_group(*member)
                        .map(|floating| (*member, floating.bounds))
                })
                .collect::<Option<Vec<_>>>()
        }) else {
            return Err(dock_core::CommandError::InvalidTarget {
                reason: "window chain contains a FloatingGroup that no longer exists",
            });
        };
        let frame_geometries = self.frame_geometries_for_bounds(&member_bounds);
        let Some(bounds) =
            arrange_contiguous_window_chain_bounds(member_bounds, &frame_geometries, anchor)
        else {
            return Err(dock_core::CommandError::InvalidTarget {
                reason: "could not calculate visible native frame geometry for the Floating window chain",
            });
        };

        let result = self.controller.update(cx, |controller, controller_cx| {
            controller.finish_floating_window_chain_move_in(&bounds, controller_cx)
        });
        if result.is_ok() {
            self.window_chains = next_chains;
            for member in members {
                if let Some(panel) = self.host_registry.panel_for_group(member)
                    && let Some(mut entry) = self.registry.windows.remove(&panel)
                {
                    self.sync_native_resize_edge_policy(&mut entry, member);
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

    /// Makes the PanelId registry and composition-host map match the current Layout.
    ///
    /// Every open panel keeps its own native window. A floating composition is
    /// rendered by one of those windows; the other panel windows stay hidden.
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

        let live_groups = groups.iter().map(|group| group.id).collect::<HashSet<_>>();
        let (removed_chain_members, chains_to_compact) =
            self.window_chains.retain_groups(&live_groups);
        if removed_chain_members > 0 {
            tracing::debug!(
                removed_chain_members,
                "removed stale FloatingGroups from window chains"
            );
        }

        if !chains_to_compact.is_empty() {
            let mut compacted_bounds = Vec::new();
            for members in &chains_to_compact {
                let member_bounds = members
                    .iter()
                    .map(|member| {
                        groups
                            .iter()
                            .find(|snapshot| snapshot.id == *member)
                            .map(|snapshot| (snapshot.id, snapshot.bounds))
                    })
                    .collect::<Option<Vec<_>>>();
                let Some(member_bounds) = member_bounds else {
                    continue;
                };
                let frame_geometries = self.frame_geometries_for_bounds(&member_bounds);
                if let Some(anchor) = members.first().copied()
                    && let Some(bounds) = arrange_contiguous_window_chain_bounds(
                        member_bounds,
                        &frame_geometries,
                        anchor,
                    )
                {
                    compacted_bounds.extend(bounds);
                }
            }

            let commands = compacted_bounds
                .iter()
                .filter_map(|(group, bounds)| {
                    groups
                        .iter()
                        .find(|snapshot| snapshot.id == *group)
                        .filter(|snapshot| snapshot.bounds != *bounds)
                        .map(|_| DockCommand::ResizeFloatingBounds {
                            group: *group,
                            bounds: *bounds,
                        })
                })
                .collect::<Vec<_>>();

            if !commands.is_empty() {
                self.schedule_window_chain_bounds_update(commands, cx);
            }
        }

        let previous_hosts = self.host_registry.floating_groups.clone();
        let next_hosts = groups
            .iter()
            .filter_map(|group| {
                let previous = previous_hosts
                    .get(&group.id)
                    .copied()
                    .filter(|panel| group.panels.contains(panel));
                let host = previous.or_else(|| group.panels.first().copied())?;
                Some((group.id, host))
            })
            .collect::<HashMap<_, _>>();
        let desired_panels = next_hosts.values().copied().collect::<HashSet<_>>();

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

        // Discard externally closed native handles, then create only the
        // composition hosts selected above. Main-dock panels allocate no windows.
        let unavailable_panels = self
            .registry
            .windows
            .iter()
            .filter_map(|(panel, entry)| {
                (!entry.window.is_available(cx)).then_some(*panel)
            })
            .collect::<Vec<_>>();
        for panel in unavailable_panels {
            self.release_panel_window(panel, cx);
        }

        let missing_panels = desired_panels
            .iter()
            .filter(|panel| !self.registry.windows.contains_key(panel))
            .copied()
            .collect::<Vec<_>>();
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

        // Publish only hosts whose GPUI view has actually been assigned. A
        // failed assignment remains absent and is retried by the next pass.
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

            self.sync_window_bounds(&mut entry, group, cx);
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

        self.synchronize_native_z_order(&groups, &active_hosts, &transferred_panels);

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

        // A composition host is handed to a surviving PanelWindow before the
        // closed panel's own HWND is destroyed. The old host was detached above;
        // the replacement has already been assigned and shown at this point.
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

    /// Applies Core's floating z-order to native windows and stores each chain's
    /// left-to-right order for activation-time correction.
    ///
    /// Windows are raised from back to front without activation. Members inside
    /// one chain are raised right-to-left so the left member owns the overlapping
    /// seam and its right Resize edge remains reachable.
    fn synchronize_native_z_order(
        &mut self,
        groups: &[FloatingGroupSnapshot],
        active_hosts: &HashMap<FloatingGroupId, PanelId>,
        transferred_panels: &[PanelId],
    ) {
        let mut interaction_chain_members = HashSet::new();
        if let Some(session) = self.native_move {
            interaction_chain_members.extend(self.window_chains.members_for(session.group));
        }
        if let Some(session) = self.native_resize {
            interaction_chain_members.extend(self.window_chains.members_for(session.group));
        }

        for group_id in floating_groups_in_native_z_order(groups, &self.window_chains) {
            if interaction_chain_members.contains(&group_id) {
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
                // A newly-created FloatingHost for a transferred composition stays
                // hidden until DragHost has rendered its empty placeholder and
                // released the cached AnyViews. Already-visible hosts remain visible.
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

            let chain_windows = self
                .window_chains
                .members_for(group_id)
                .into_iter()
                .filter_map(|member| self.panel_window_for_group(member)?.native_window)
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

    /// Shows the persistent FloatingHost only after DragHost has stopped
    /// mounting the transferred AnyViews. This prevents a newly-created host
    /// from presenting an empty frame while DragHost still owns the content.
    pub(crate) fn show_handoff_target(
        &mut self,
        group: FloatingGroupId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(panel) = self.host_registry.panel_for_group(group) else {
            return false;
        };
        let Some((window, native_window, already_visible)) = self.registry.windows.get(&panel).and_then(|entry| {
            (entry.host_group == Some(group)).then_some((
                entry.window,
                entry.native_window?,
                entry.visibility == PanelWindowVisibility::Visible,
            ))
        }) else {
            return false;
        };
        if !already_visible {
            if let Err(error) = self.native_ops.show_no_activate(native_window) {
                tracing::warn!(
                    ?group,
                    ?panel,
                    %error,
                    "could not show the persistent FloatingHost during DragHost handoff"
                );
                return false;
            }
            if let Some(entry) = self.registry.windows.get_mut(&panel) {
                entry.visibility = PanelWindowVisibility::Visible;
            }
        }

        // Request a redraw now; foreground activation is deferred until
        // DragHost has actually been closed, otherwise Windows can activate
        // its owner (Main) again while destroying the owned DragHost.
        self.request_native_redraw(panel);
        if let Err(error) = window.notify_host(cx) {
            tracing::warn!(
                ?group,
                ?panel,
                %error,
                "could not notify the FloatingHost after panel handoff"
            );
            return false;
        }

        cx.notify();
        true
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

    /// Notifies and invalidates all currently visible floating composition hosts.
    pub(crate) fn redraw_visible_hosts(&self, cx: &mut Context<Self>) {
        let hosts = self
            .registry
            .windows
            .iter()
            .filter(|(_, entry)| {
                entry.host_group.is_some() && entry.visibility == PanelWindowVisibility::Visible
            })
            .map(|(panel, entry)| (*panel, entry.window))
            .collect::<Vec<_>>();

        for (panel, window) in hosts {
            if let Err(error) = window.notify_host(cx) {
                tracing::debug!(?panel, %error, "could not notify a visible FloatingHost");
            }
            self.request_native_redraw(panel);
        }
    }

    /// Activates the persistent destination after its temporary DragHost has closed.
    ///
    /// Closing the owned DragHost can restore Main as the foreground window.
    /// Only reclaim focus from Main; never steal it back after the user has
    /// already moved to another application or another floating window.
    pub(crate) fn activate_handoff_target(
        &self,
        group: FloatingGroupId,
        closing_drag_host: NativeWindowId,
    ) -> bool {
        let Some(panel) = self.host_registry.panel_for_group(group) else {
            return false;
        };
        let Some(entry) = self.registry.windows.get(&panel) else {
            return false;
        };
        if entry.host_group != Some(group) || entry.visibility != PanelWindowVisibility::Visible {
            return false;
        }
        let Some(native_window) = entry.native_window else {
            return false;
        };

        match self.native_ops.foreground_window() {
            Ok(Some(foreground)) if foreground == native_window => return true,
            Ok(Some(foreground)) if foreground == closing_drag_host => return false,
            Ok(Some(foreground)) if Some(foreground) != self.main_native_window => {
                // Respect a different app window or another FloatingHost that
                // the user activated after releasing the drag.
                return true;
            }
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(
                    ?group,
                    %error,
                    "could not inspect foreground window after DragHost close"
                );
                return false;
            }
        }

        if let Err(error) = self.native_ops.activate(native_window) {
            tracing::warn!(
                ?group,
                ?panel,
                %error,
                "could not activate FloatingHost after DragHost close"
            );
            return false;
        }

        matches!(
            self.native_ops.foreground_window(),
            Ok(Some(foreground)) if foreground == native_window
        )
    }

    /// Returns the transferred panels whose destination FloatingHost is ready
    /// to render. FloatingHost calls this from its render pass before scheduling
    /// a next-frame handoff confirmation.
    pub(crate) fn pending_drag_host_handoff_panels(
        &self,
        group: FloatingGroupId,
        window_id: WindowId,
        cx: &mut Context<Self>,
    ) -> Option<Vec<PanelId>> {
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

    /// Called after the destination FloatingHost has completed a frame with
    /// the transferred PanelViews mounted. Only then is the temporary DragHost
    /// removed from the desktop.
    pub(crate) fn notify_drag_host_handoff_rendered(
        &mut self,
        group: FloatingGroupId,
        window_id: WindowId,
        rendered_panels: &[PanelId],
        cx: &mut Context<Self>,
    ) {
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

        if let Some(drag_host) = self.drag_host_manager.as_ref().and_then(|manager| manager.upgrade()) {
            let _ = drag_host.update(cx, |manager, manager_cx| {
                manager.complete_floating_handoff(group, rendered_panels, manager_cx);
            });
        }

        // The destination frame was rendered before DragHost closed. Queue one
        // more native paint after the source surface stops covering the target.
        self.request_native_redraw(host_panel);
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

        let members = self.window_chains.members_for(group);
        let desired_left_enabled = members.first() == Some(&group);
        let desired_top_enabled = members.len() <= 1;

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
        cx: &mut Context<Self>,
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

        // The HWND has been resized through the native API rather than
        // Window::resize. Refresh GPUI's cached viewport immediately so an
        // adopted DragHost does not keep rendering at its pre-promotion height.
        if let Err(error) = entry.window.refresh_gpui_bounds(cx) {
            tracing::warn!(
                ?group.id,
                %error,
                "could not refresh GPUI bounds after native floating-window resize"
            );
        }

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
    use super::is_managed_foreground_window;
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
mod window_chain_registry_tests {
    use super::WindowChainRegistry;
    use dock_core::FloatingGroupId;
    use slotmap::SlotMap;
    use std::collections::HashSet;

    fn groups(count: usize) -> Vec<FloatingGroupId> {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        (0..count).map(|_| ids.insert(())).collect()
    }

    #[test]
    fn links_before_and_after_keep_requested_order() {
        let [a, b, c, d] = groups(4).try_into().expect("four group ids");
        let mut chains = WindowChainRegistry::default();

        assert!(chains.link_after(a, b));
        assert!(chains.link_after(b, c));
        assert!(chains.link_before(b, d));
        assert_eq!(chains.members_for(a), vec![a, d, b, c]);
        assert_eq!(chains.members_for(c), vec![a, d, b, c]);
    }

    #[test]
    fn joining_existing_chains_inserts_the_whole_chain_at_the_anchor() {
        let [a, b, c, d, e] = groups(5).try_into().expect("five group ids");
        let mut chains = WindowChainRegistry::default();

        assert!(chains.link_after(a, b));
        assert!(chains.link_after(c, d));
        assert!(chains.link_after(a, c));
        assert_eq!(chains.members_for(a), vec![a, c, d, b]);
        assert_eq!(chains.members_for(e), vec![e]);
    }

    #[test]
    fn linking_members_already_in_the_same_chain_is_a_no_op() {
        let [a, b, c] = groups(3).try_into().expect("three group ids");
        let mut chains = WindowChainRegistry::default();

        assert!(chains.link_after(a, b));
        assert!(!chains.link_after(a, b));
        assert!(!chains.link_before(a, b));
        assert_eq!(chains.members_for(c), vec![c]);
        assert_eq!(chains.members_for(a), vec![a, b]);
    }

    #[test]
    fn inserts_a_new_window_sequence_before_or_after_an_existing_chain() {
        let [left, anchor, right, source_a, source_b, new_right_a, new_right_b] =
            groups(7).try_into().expect("seven group ids");
        let mut chains = WindowChainRegistry::default();
        assert!(chains.link_after(anchor, right));
        assert!(chains.link_before(anchor, left));

        assert!(chains.link_sequence_at(anchor, &[source_a, source_b], true));
        assert_eq!(
            chains.members_for(anchor),
            vec![left, source_a, source_b, anchor, right]
        );

        assert!(chains.link_sequence_at(anchor, &[new_right_a, new_right_b], false));
        assert_eq!(
            chains.members_for(anchor),
            vec![left, source_a, source_b, anchor, new_right_a, new_right_b, right]
        );
    }

    #[test]
    fn removing_a_middle_member_preserves_the_remaining_order() {
        let [a, b, c] = groups(3).try_into().expect("three group ids");
        let mut chains = WindowChainRegistry::default();

        chains.link_after(a, b);
        chains.link_after(b, c);
        assert!(chains.remove_member(b));
        assert_eq!(chains.members_for(a), vec![a, c]);
        assert_eq!(chains.members_for(c), vec![a, c]);
        assert_eq!(chains.members_for(b), vec![b]);
    }


    #[test]
    fn removing_the_first_member_rekeys_the_remaining_chain() {
        let [a, b, c] = groups(3).try_into().expect("three group ids");
        let mut chains = WindowChainRegistry::default();

        chains.link_after(a, b);
        chains.link_after(b, c);
        assert!(chains.remove_member(a));
        assert_eq!(chains.members_for(b), vec![b, c]);
        assert_eq!(chains.members_for(c), vec![b, c]);
        assert_eq!(chains.members_for(a), vec![a]);
    }

    #[test]
    fn removing_members_dissolves_a_chain_when_only_one_remains() {
        let [a, b] = groups(2).try_into().expect("two group ids");
        let mut chains = WindowChainRegistry::default();

        chains.link_after(a, b);
        assert!(chains.remove_member(a));
        assert_eq!(chains.members_for(b), vec![b]);
        assert!(!chains.remove_member(a));
    }


    #[test]
    fn pruning_a_middle_member_returns_ordered_survivors_for_compaction() {
        let [a, b, c, d] = groups(4).try_into().expect("four group ids");
        let mut chains = WindowChainRegistry::default();

        chains.link_after(a, b);
        chains.link_after(b, c);
        chains.link_after(c, d);

        let live = HashSet::from([a, c, d]);
        let (removed, survivors) = chains.retain_groups(&live);
        assert_eq!(removed, 1);
        assert_eq!(survivors, vec![vec![a, c, d]]);
        assert_eq!(chains.members_for(c), vec![a, c, d]);
    }

    #[test]
    fn reconciliation_prunes_missing_groups_and_dissolves_short_chains() {
        let [a, b, c, d] = groups(4).try_into().expect("four group ids");
        let mut chains = WindowChainRegistry::default();

        chains.link_after(a, b);
        chains.link_after(b, c);
        chains.link_after(c, d);

        let live = HashSet::from([a, c, d]);
        assert_eq!(
            chains.retain_groups(&live),
            (1, vec![vec![a, c, d]])
        );
        assert_eq!(chains.members_for(a), vec![a, c, d]);
        assert_eq!(chains.members_for(b), vec![b]);

        let live = HashSet::from([c]);
        assert_eq!(chains.retain_groups(&live), (2, Vec::new()));
        assert_eq!(chains.retain_groups(&live), (0, Vec::new()));
        assert_eq!(chains.members_for(c), vec![c]);
    }
}


#[cfg(test)]
mod contiguous_window_chain_bounds_tests {
    use super::{
        NativeFrameGeometry, arrange_contiguous_window_chain_bounds,
        floating_window_chain_insert_before, floating_window_drop_destination, NativeMoveTarget,
    };
    use dock_core::{FloatingGroupId, Point, Rect, Size};
    use slotmap::{Key, SlotMap};

    fn groups() -> (SlotMap<FloatingGroupId, ()>, [FloatingGroupId; 3]) {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let groups = [ids.insert(()), ids.insert(()), ids.insert(())];
        (ids, groups)
    }

    #[test]
    fn keeps_anchor_and_places_neighbours_edge_to_edge() {
        let (_ids, [a, b, c]) = groups();
        let input = vec![
            (a, Rect::new(Point::new(50.0, 10.0), Size::new(120.0, 300.0))),
            (b, Rect::new(Point::new(500.0, 40.0), Size::new(240.0, 360.0))),
            (c, Rect::new(Point::new(900.0, 90.0), Size::new(180.0, 280.0))),
        ];

        let frames = [
            NativeFrameGeometry::without_frame(120.0).unwrap(),
            NativeFrameGeometry::without_frame(240.0).unwrap(),
            NativeFrameGeometry::without_frame(180.0).unwrap(),
        ];
        let arranged = arrange_contiguous_window_chain_bounds(input, &frames, b)
            .expect("anchor must belong to the chain");

        assert_eq!(arranged[0].1.origin, Point::new(380.0, 40.0));
        assert_eq!(arranged[1].1.origin, Point::new(500.0, 40.0));
        assert_eq!(arranged[2].1.origin, Point::new(740.0, 40.0));
        assert_eq!(arranged[0].1.size, Size::new(120.0, 300.0));
        assert_eq!(arranged[1].1.size, Size::new(240.0, 360.0));
        assert_eq!(arranged[2].1.size, Size::new(180.0, 280.0));
    }

    #[test]
    fn supports_anchor_at_chain_edge_and_rejects_missing_anchor() {
        let (_ids, [a, b, c]) = groups();
        let input = vec![
            (a, Rect::new(Point::new(30.0, 8.0), Size::new(100.0, 200.0))),
            (b, Rect::new(Point::new(300.0, 28.0), Size::new(150.0, 240.0))),
            (c, Rect::new(Point::new(700.0, 48.0), Size::new(200.0, 260.0))),
        ];

        let frames = [
            NativeFrameGeometry::without_frame(100.0).unwrap(),
            NativeFrameGeometry::without_frame(150.0).unwrap(),
            NativeFrameGeometry::without_frame(200.0).unwrap(),
        ];
        let arranged = arrange_contiguous_window_chain_bounds(input.clone(), &frames, a)
            .expect("first member is a valid anchor");
        assert_eq!(arranged[0].1.origin, Point::new(30.0, 8.0));
        assert_eq!(arranged[1].1.origin, Point::new(130.0, 8.0));
        assert_eq!(arranged[2].1.origin, Point::new(280.0, 8.0));

        assert!(
            arrange_contiguous_window_chain_bounds(
                input,
                &frames,
                FloatingGroupId::null(),
            )
            .is_none()
        );
    }

    #[test]
    fn visible_frame_edges_touch_despite_invisible_resize_borders() {
        let (_ids, [a, b, c]) = groups();
        let input = vec![
            (a, Rect::new(Point::new(0.0, 20.0), Size::new(300.0, 240.0))),
            (b, Rect::new(Point::new(1000.0, 40.0), Size::new(400.0, 320.0))),
            (c, Rect::new(Point::new(1800.0, 60.0), Size::new(200.0, 200.0))),
        ];

        // Outer widths are physical pixels. DWM-visible bounds are inset by
        // each window's invisible resize border.
        let frames = [
            NativeFrameGeometry {
                outer_width: 450.0,
                visible_left_inset: 6.0,
                visible_right_inset: 6.0,
            },
            NativeFrameGeometry {
                outer_width: 500.0,
                visible_left_inset: 8.0,
                visible_right_inset: 8.0,
            },
            NativeFrameGeometry {
                outer_width: 400.0,
                visible_left_inset: 4.0,
                visible_right_inset: 4.0,
            },
        ];
        let arranged = arrange_contiguous_window_chain_bounds(input, &frames, b)
            .expect("visible frame geometry must place the chain");
        assert_eq!(arranged[0].1.origin, Point::new(564.0, 40.0));
        assert_eq!(arranged[1].1.origin, Point::new(1000.0, 40.0));
        assert_eq!(arranged[2].1.origin, Point::new(1488.0, 40.0));

        let a_visible_right =
            arranged[0].1.origin.x + frames[0].outer_width - frames[0].visible_right_inset;
        let b_visible_left = arranged[1].1.origin.x + frames[1].visible_left_inset;
        let b_visible_right =
            arranged[1].1.origin.x + frames[1].outer_width - frames[1].visible_right_inset;
        let c_visible_left = arranged[2].1.origin.x + frames[2].visible_left_inset;
        assert_eq!(a_visible_right, b_visible_left);
        assert_eq!(b_visible_right, c_visible_left);
    }

    #[test]
    fn rejects_missing_or_invalid_frame_geometry() {
        let (_ids, [a, b, _c]) = groups();
        let input = vec![
            (a, Rect::new(Point::ZERO, Size::new(100.0, 200.0))),
            (b, Rect::new(Point::new(300.0, 0.0), Size::new(100.0, 200.0))),
        ];
        let valid = NativeFrameGeometry::without_frame(100.0).unwrap();
        assert!(arrange_contiguous_window_chain_bounds(input.clone(), &[valid], a).is_none());
        assert!(
            arrange_contiguous_window_chain_bounds(
                input,
                &[
                    valid,
                    NativeFrameGeometry {
                        outer_width: 100.0,
                        visible_left_inset: 50.0,
                        visible_right_inset: 50.0,
                    },
                ],
                a,
            )
            .is_none()
        );
    }

    #[test]
    fn floating_window_chain_side_matches_the_resolved_drop_zone() {
        use dock_core::{DropTarget, DropZone, NodeId};

        let group = NodeId::null();
        let left = DropTarget::GroupEdge {
            group,
            zone: DropZone::Left,
        };
        let right = DropTarget::GroupEdge {
            group,
            zone: DropZone::Right,
        };
        let top = DropTarget::GroupEdge {
            group,
            zone: DropZone::Top,
        };
        let center = DropTarget::GroupTabs { group, index: 0 };

        assert_eq!(floating_window_chain_insert_before(left), Some(true));
        assert_eq!(floating_window_chain_insert_before(right), Some(false));
        assert_eq!(floating_window_chain_insert_before(top), None);

        assert!(matches!(
            floating_window_drop_destination(FloatingGroupId::null(), left, Point::ZERO),
            NativeMoveTarget::FloatingChain { insert_before: true, .. }
        ));
        assert!(matches!(
            floating_window_drop_destination(FloatingGroupId::null(), right, Point::ZERO),
            NativeMoveTarget::FloatingChain { insert_before: false, .. }
        ));
        assert!(matches!(
            floating_window_drop_destination(FloatingGroupId::null(), top, Point::ZERO),
            NativeMoveTarget::FloatingDock { target, .. } if target == top
        ));
        assert!(matches!(
            floating_window_drop_destination(FloatingGroupId::null(), center, Point::ZERO),
            NativeMoveTarget::FloatingDock { target, .. } if target == center
        ));
    }
}


#[cfg(test)]
mod native_resize_chain_geometry_tests {
    use super::{native_resize_right_edge_delta, shift_window_chain_after_resize};
    use dock_core::{FloatingGroupId, Point, Rect, Size};
    use slotmap::SlotMap;

    fn groups() -> (SlotMap<FloatingGroupId, ()>, [FloatingGroupId; 3]) {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let groups = [ids.insert(()), ids.insert(()), ids.insert(())];
        (ids, groups)
    }

    #[test]
    fn right_edge_resize_moves_later_windows_by_physical_width_delta_only() {
        let (_ids, [a, b, c]) = groups();
        let before = Rect::new(Point::new(100.0, 20.0), Size::new(300.0, 400.0));
        let actual = Rect::new(Point::new(100.0, 20.0), Size::new(340.0, 400.0));
        let delta = native_resize_right_edge_delta(before, actual, 1.5)
            .expect("valid monitor scale");
        assert_eq!(delta, 60.0);

        let bounds = vec![
            (a, Rect::new(Point::new(100.0, 20.0), Size::new(300.0, 400.0))),
            (b, actual),
            (c, Rect::new(Point::new(760.0, 20.0), Size::new(220.0, 320.0))),
        ];
        let shifted = shift_window_chain_after_resize(bounds, b, delta)
            .expect("resized member must belong to chain");
        assert_eq!(shifted[0].1.origin, Point::new(100.0, 20.0));
        assert_eq!(shifted[1].1.origin, Point::new(100.0, 20.0));
        assert_eq!(shifted[2].1.origin, Point::new(820.0, 20.0));
        assert_eq!(shifted[2].1.size, Size::new(220.0, 320.0));
    }

    #[test]
    fn left_edge_resize_with_anchored_right_edge_does_not_move_neighbours() {
        let before = Rect::new(Point::new(100.0, 20.0), Size::new(300.0, 400.0));
        let actual = Rect::new(Point::new(160.0, 20.0), Size::new(260.0, 400.0));
        assert_eq!(native_resize_right_edge_delta(before, actual, 1.5), Some(0.0));
    }

    #[test]
    fn resizing_last_member_does_not_change_earlier_windows() {
        let (_ids, [a, b, c]) = groups();
        let bounds = vec![
            (a, Rect::new(Point::new(100.0, 20.0), Size::new(300.0, 400.0))),
            (b, Rect::new(Point::new(400.0, 20.0), Size::new(240.0, 360.0))),
            (c, Rect::new(Point::new(640.0, 20.0), Size::new(220.0, 320.0))),
        ];
        let shifted = shift_window_chain_after_resize(bounds.clone(), c, 80.0)
            .expect("last member must belong to chain");
        assert_eq!(shifted, bounds);
    }
}

#[cfg(test)]
mod native_z_order_tests {
    use super::{FloatingGroupSnapshot, WindowChainRegistry, floating_groups_in_native_z_order};
    use dock_core::{FloatingGroupId, Point, Rect, Size};
    use slotmap::SlotMap;

    fn groups() -> (SlotMap<FloatingGroupId, ()>, [FloatingGroupId; 5]) {
        let mut ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let groups = [
            ids.insert(()),
            ids.insert(()),
            ids.insert(()),
            ids.insert(()),
            ids.insert(()),
        ];
        (ids, groups)
    }

    fn snapshot(id: FloatingGroupId, z_order: i32) -> FloatingGroupSnapshot {
        FloatingGroupSnapshot {
            id,
            bounds: Rect::new(Point::ZERO, Size::new(300.0, 240.0)),
            z_order,
            minimum_size: None,
            horizontal_resize_minimum_widths: None,
            panels: Vec::new(),
        }
    }

    #[test]
    fn chain_members_are_kept_adjacent_in_native_z_order() {
        let (_ids, [a, b, c, d, e]) = groups();
        let groups = [
            snapshot(a, 1),
            snapshot(c, 4),
            snapshot(d, 5),
            snapshot(e, 7),
            snapshot(b, 8),
        ];
        let mut chains = WindowChainRegistry::default();
        assert!(chains.link_after(a, b));
        assert!(chains.link_after(c, d));

        assert_eq!(
            floating_groups_in_native_z_order(&groups, &chains),
            vec![d, c, e, b, a],
            "chains are raised back-to-front internally and ranked by their frontmost member"
        );
    }

    #[test]
    fn an_unlinked_floating_group_remains_a_single_z_order_block() {
        let (_ids, [a, b, c, d, _]) = groups();
        let groups = [snapshot(a, 3), snapshot(b, 1), snapshot(c, 2), snapshot(d, 4)];

        assert_eq!(
            floating_groups_in_native_z_order(&groups, &WindowChainRegistry::default()),
            vec![b, c, a, d]
        );
    }
}


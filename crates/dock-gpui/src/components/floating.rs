use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use dock_core::{
    ColumnDisplayItem, ColumnMode, ColumnRect, ComputeRectsError, DockCommand, DockMetrics,
    DragPreview, DropZoneRect, FloatingGroupId, FloatingGroupRect, FlyoutState,
    GroupRect, LayoutRects, Node, NodeId, PanelId, PanelRect, Point, Rect, SplitterRect,
    TabHeaderRect, TabOverflowRect, TabStripRect, VerticalOverflow,
};
use gpui::{
    AnyView, Context, DispatchPhase, Entity, FocusHandle, HitboxBehavior, HitboxId, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Render, ScrollHandle,
    Styled, WeakEntity, Window, WindowId, canvas, div, prelude::InteractiveElement as _, rgb,
};
use ui::Theme;

use crate::{
    dock_components::{
        collect_group_presentations, CompositionDragArea, DockCompositionView,
        DockGroupView, DockTab, GroupDragArea, GroupPresentation,
    },
    dock_host::{
        build_context_menu_entries, collect_subtree_nodes, context_menu_target_exists,
        render_resize_preview,
    },
    input::{
        drag::{
            begin_drag_at, begin_group_drag_host, begin_panel_drag_host,
            finish_floating_tab_drag,
            update_drag_at, update_floating_tab_drag, FloatingTabDrag,
        },
        interaction::DockInteractionCoordinator,
        resize::{
            begin_resize_hit_with_rects, finish_resize_with_rects, update_resize,
        },
        resize_preview_for_interaction, ResizePreview,
    },
    DockController, DragSourceHost, FloatingWindowManager,
};

use super::{
    drop_overlay::DropOverlay,
    context_menu::{
        ContextMenu, ContextMenuCallbacks, ContextMenuDismiss, ContextMenuSubmenuToggle,
        ContextMenuTarget, GroupMenuToggle,
    },
    panel_host::PanelHost,
    splitter::SplitterView,
};

/// Content view for one native floating window.
///
/// The root already owns the native window bounds and local coordinate origin.
/// The view renders only the Core subtree supplied for its FloatingGroupId.
struct FloatingWindowContent;

impl FloatingWindowContent {
    #[allow(clippy::too_many_arguments)]
    fn render(
        floating: FloatingGroupRect,
        columns: &[ColumnRect],
        groups: &[GroupRect],
        panels: Vec<(PanelRect, Option<AnyView>)>,
        dragged_group: Option<NodeId>,
        tab_strips: &[TabStripRect],
        tab_headers: &[TabHeaderRect],
        tab_overflows: &[TabOverflowRect],
        group_presentations: &HashMap<NodeId, GroupPresentation>,
        column_groups: &HashMap<NodeId, Vec<NodeId>>,
        column_modes: &HashMap<NodeId, ColumnMode>,
        rail_display_orders: &HashMap<NodeId, Vec<ColumnDisplayItem>>,
        splitters: &[SplitterRect],
        vertical_overflow: &[VerticalOverflow],
        scroll_handles: &HashMap<NodeId, ScrollHandle>,
        active_flyout: Option<FlyoutState>,
        drag_preview: Option<DragPreview>,
        drag_zones: &[DropZoneRect],
        metrics: DockMetrics,
        hide_header: bool,
        menu_group: Option<NodeId>,
        context_menu: GroupMenuToggle,
        middle_click_closes_tabs: bool,
        controller: Entity<DockController>,
        theme: Theme,
    ) -> impl IntoElement {
        let origin = Point::ZERO;
        let mut column_group_ids = HashSet::new();
        for ids in column_groups.values() {
            column_group_ids.extend(ids.iter().copied());
        }

        let column_active_panels = column_group_ids
            .iter()
            .filter_map(|group| {
                group_presentations
                    .get(group)
                    .and_then(|presentation| presentation.active_panel)
            })
            .collect::<HashSet<PanelId>>();

        let mut root = div()
            .id(format!("floating-window-{:?}", floating.group))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(theme.colors.panel_surface_background));

        // MainDockView and FloatingWindowContent use the same composition
        // renderer. Only the host-local coordinates, floating identity and
        // shared CompositionHeader visibility differ between the two windows.
        for column in columns {
            let group_ids = column_groups
                .get(&column.column)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let mode = column_modes
                .get(&column.column)
                .copied()
                .unwrap_or(ColumnMode::Expanded);

            root = root.child(DockCompositionView::render_column(
                *column,
                groups,
                &panels,
                tab_strips,
                tab_headers,
                tab_overflows,
                group_presentations,
                group_ids,
                mode,
                rail_display_orders
                    .get(&column.column)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                active_flyout,
                drag_preview,
                drag_zones,
                metrics,
                menu_group,
                context_menu.clone(),
                middle_click_closes_tabs,
                splitters,
                vertical_overflow,
                scroll_handles.get(&column.column).cloned(),
                controller.clone(),
                theme,
                Some(floating.group),
                !hide_header,
                dragged_group,
            ));
        }

        // Tabs nodes that are not owned by a Column still use DockGroupView.
        for group in groups
            .iter()
            .filter(|group| !column_group_ids.contains(&group.group))
        {
            let Some(strip) = tab_strips
                .iter()
                .find(|strip| strip.group == group.group)
                .copied()
            else {
                continue;
            };
            let Some(presentation) = group_presentations.get(&group.group) else {
                continue;
            };
            let headers = tab_headers
                .iter()
                .filter(|header| header.group == group.group)
                .copied()
                .collect::<Vec<_>>();
            let overflow = tab_overflows
                .iter()
                .find(|overflow| overflow.group == group.group)
                .cloned();

            if dragged_group == Some(group.group) {
                root = root.child(DockGroupView::render_placeholder(
                    *group,
                    origin,
                    metrics,
                    theme,
                ));
            } else {
                root = root.child(DockGroupView::render_core(
                    None,
                    *group,
                    strip,
                    &headers,
                    overflow,
                    presentation,
                    origin,
                    metrics,
                    0.0,
                    controller.clone(),
                    theme,
                    context_menu.clone(),
                ));
            }
        }

        // X splits and splitters outside Columns are painted once above the
        // child surfaces. Column-owned Y splitters were already rendered by
        // DockColumnView, where their scroll offset is available.
        for splitter in splitters.iter().filter(|splitter| {
            !(splitter.axis == dock_core::Axis::Y && column_groups.contains_key(&splitter.owner))
        }) {
            root = root.child(SplitterView::render(
                *splitter,
                origin,
                Point::ZERO,
                controller.clone(),
                metrics,
                theme,
                Some(floating.group),
            ));
        }

        // Active panel views outside Columns (standalone Tabs/Panel roots and
        // split children) are hosted directly in the floating composition.
        let loose_panels = panels
            .into_iter()
            .filter_map(|(panel, view)| {
                if column_active_panels.contains(&panel.panel) {
                    return None;
                }

                Some((panel, view))
            })
            .collect();
        root = root.child(PanelHost::render(loose_panels, origin, theme));

        if !hide_header {
            root = root.child(DockCompositionView::render_header(
                format!("floating-composition-{:?}", floating.group),
                floating.rect,
                columns,
                metrics,
                theme,
            ));
        }

        root
    }
}


#[derive(Clone, Copy)]
struct FloatingTabPress {
    tab_group: NodeId,
    panel: PanelId,
    origin: Point,
    layout_version: u64,
}

#[derive(Clone, Copy)]
struct FloatingGroupPress {
    group: NodeId,
    layout_version: u64,
}

#[derive(Clone)]
struct FloatingHostOpenMenu {
    target: ContextMenuTarget,
    anchor: Point,
    open_submenu: Option<usize>,
}

struct FloatingHostSnapshot {
    rects: LayoutRects,
    floating: FloatingGroupRect,
    column_ids: HashSet<NodeId>,
    column_groups: HashMap<NodeId, Vec<NodeId>>,
    column_modes: HashMap<NodeId, ColumnMode>,
    group_presentations: HashMap<NodeId, GroupPresentation>,
    rail_display_orders: HashMap<NodeId, Vec<ColumnDisplayItem>>,
    metrics: DockMetrics,
    drag_preview: Option<DragPreview>,
    drag_zones: Vec<DropZoneRect>,
    active_drop_zone: Option<dock_core::DropZoneRect>,
    resize_preview: Option<ResizePreview>,
}

/// Floating tab drags transfer one Panel to DragHost after leaving the source strip
/// or entering another docking window. The shared CompositionHeader moves the whole window.
fn floating_tab_drag_host_required(leaves_source: bool, over_other_window: bool) -> bool {
    leaves_source || over_other_window
}

/// A Floating containing exactly one Group and one Panel can move as a whole.
/// Detaching its only tab/group would only create a replacement window.
fn floating_has_single_tab_dock(
    layout: &dock_core::Layout,
    floating_group: FloatingGroupId,
    expected_group: NodeId,
) -> bool {
    let Some(floating) = layout.floating_group(floating_group) else {
        return false;
    };
    let Some(area) = layout.dock_area(floating.area) else {
        return false;
    };
    fn collect_groups(layout: &dock_core::Layout, node: NodeId, groups: &mut Vec<NodeId>) {
        match layout.node(node) {
            Some(Node::Split(split)) => {
                for child in &split.children {
                    collect_groups(layout, *child, groups);
                }
            }
            Some(Node::Column(column)) => groups.extend(column.groups.iter().copied()),
            Some(Node::Tabs(_)) => groups.push(node),
            Some(Node::Panel(_)) | None => {}
        }
    }
    let mut groups = Vec::new();
    collect_groups(layout, area.root, &mut groups);
    let [group] = groups.as_slice() else {
        return false;
    };
    if *group != expected_group {
        return false;
    }
    let Some(Node::Tabs(tabs)) = layout.node(*group) else {
        return false;
    };
    let [panel] = tabs.children.as_slice() else {
        return false;
    };

    matches!(layout.node(*panel), Some(Node::Panel(_)))
}

/// Gives the shared CompositionHeader input priority over its content.
/// Returns Some(false) for controls.
fn floating_dock_area_title_pointer_region(
    area_rect: Rect,
    columns: &[ColumnRect],
    point: Point,
    metrics: DockMetrics,
) -> Option<bool> {
    let header_height = metrics.column_header_height.min(area_rect.size.height);
    let right = area_rect.origin.x + area_rect.size.width;
    let bottom = area_rect.origin.y + header_height;
    if point.x < area_rect.origin.x
        || point.x >= right
        || point.y < area_rect.origin.y
        || point.y >= bottom
    {
        return None;
    }

    Some(CompositionDragArea::contains(
        area_rect,
        columns,
        point,
        header_height,
        metrics.column_control_button_width,
    ))
}

fn floating_dock_component_tab_zones(snapshot: &FloatingHostSnapshot) -> HashMap<PanelId, Rect> {
    let mut zones = HashMap::new();
    for column in &snapshot.rects.columns {
        let group_ids = snapshot
            .column_groups
            .get(&column.column)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let group_rects = snapshot
            .rects
            .groups
            .iter()
            .filter(|group| group_ids.contains(&group.group))
            .collect::<Vec<_>>();
        if group_rects.len() != 1 {
            continue;
        }
        let group = group_rects[0];
        let Some(presentation) = snapshot.group_presentations.get(&group.group) else {
            continue;
        };
        if presentation.is_single_tab()
            && presentation.active_panel == presentation.tabs.first().map(|tab| tab.panel)
            && let Some(tab) = presentation.tabs.first()
        {
            zones.insert(
                tab.panel,
                DockTab::drag_zone(group.rect, snapshot.metrics),
            );
        }
    }
    zones
}

fn floating_host_snapshot(
    controller: &mut DockController,
    window_id: WindowId,
    group: FloatingGroupId,
) -> Result<Option<FloatingHostSnapshot>, ComputeRectsError> {
    let metrics = controller.metrics();
    let rects = controller.floating_geometry(window_id, group)?.clone();
    let layout = controller.layout();
    let Some(floating) = rects.floating_groups.first().copied() else {
        // Hidden dock chrome deliberately produces an empty floating snapshot.
        return Ok(None);
    };
    let Some(floating_model) = layout.floating_group(group) else {
        return Ok(None);
    };
    let Some(area) = layout.dock_area(floating_model.area) else {
        return Ok(None);
    };

    let mut node_ids = HashSet::new();
    let mut column_ids = HashSet::new();
    let mut column_groups = HashMap::new();
    collect_subtree_nodes(
        layout,
        area.root,
        &mut node_ids,
        &mut column_ids,
        &mut column_groups,
    );

    let column_modes = column_ids
        .iter()
        .filter_map(|id| match layout.node(*id) {
            Some(Node::Column(column)) => Some((*id, column.mode)),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let group_presentations =
        collect_group_presentations(layout, controller.registry(), &node_ids, Some(&rects));
    let rail_display_orders = column_modes
        .iter()
        .filter(|(_, mode)| mode.is_rail())
        .filter_map(|(column, _)| {
            layout
                .column_display_order(*column)
                .ok()
                .map(|order| (*column, order))
        })
        .collect::<HashMap<NodeId, Vec<ColumnDisplayItem>>>();
    let drag_preview = controller.floating_window_drag_preview(window_id, group);
    let drag_resolver = controller.floating_window_drop_resolver(window_id, group);
    let drag_zones = drag_resolver
        .map(|resolver| resolver.zones().to_vec())
        .unwrap_or_default();
    let active_drop_zone =
        DropOverlay::resolve_active_zone(drag_resolver, drag_preview);
    let resize_preview = controller
        .resize_interaction()
        .and_then(resize_preview_for_interaction);

    Ok(Some(FloatingHostSnapshot {
        rects,
        floating,
        column_ids,
        column_groups,
        column_modes,
        group_presentations,
        rail_display_orders,
        metrics,
        drag_preview,
        drag_zones,
        active_drop_zone,
        resize_preview,
    }))
}

/// Root GPUI view for exactly one Core floating composition.
///
/// The host owns presentation-only state (scroll handles and a transient
/// context menu). The shared DockController remains the sole owner of Layout,
/// history and panel views. Geometry is requested in this window's local
/// coordinate space on every render.
pub struct FloatingHost {
    controller: Entity<DockController>,
    manager: WeakEntity<FloatingWindowManager>,
    group: Option<FloatingGroupId>,
    theme: Theme,
    column_scroll_handles: HashMap<NodeId, ScrollHandle>,
    open_context_menu: Option<FloatingHostOpenMenu>,
    tab_press: Rc<Cell<Option<FloatingTabPress>>>,
    group_press: Rc<Cell<Option<FloatingGroupPress>>>,
    pointer_capture: Rc<Cell<Option<HitboxId>>>,
    controller_observed: bool,
    manager_observed: bool,
    native_resize_observation_pending: bool,
    middle_click_closes_tabs: bool,
    /// Root focus boundary for cancelling gestures when this PanelWindow loses app focus.
    root_focus_handle: Option<FocusHandle>,
    focus_observed: bool,
}

impl FloatingHost {
    /// Creates a host for an already assigned floating composition.
    ///
    /// This constructor remains compatible with callers that explicitly create
    /// a group host. The permanent PanelWindow manager uses new_panel_window().
    pub fn new(
        controller: Entity<DockController>,
        group: FloatingGroupId,
        theme: Theme,
        manager: WeakEntity<FloatingWindowManager>,
    ) -> Self {
        Self::with_host_group(controller, Some(group), theme, manager)
    }

    /// Creates the permanent view hosted by one PanelWindow.
    ///
    /// A window with no assigned group stays blank and hidden. The manager
    /// assigns a group only while this panel is the chosen composition host.
    pub(crate) fn new_panel_window(
        controller: Entity<DockController>,
        theme: Theme,
        manager: WeakEntity<FloatingWindowManager>,
    ) -> Self {
        Self::with_host_group(controller, None, theme, manager)
    }

    fn with_host_group(
        controller: Entity<DockController>,
        group: Option<FloatingGroupId>,
        theme: Theme,
        manager: WeakEntity<FloatingWindowManager>,
    ) -> Self {
        Self {
            controller,
            manager,
            group,
            theme,
            column_scroll_handles: HashMap::new(),
            open_context_menu: None,
            tab_press: Rc::new(Cell::new(None)),
            group_press: Rc::new(Cell::new(None)),
            pointer_capture: Rc::new(Cell::new(None)),
            controller_observed: false,
            manager_observed: false,
            native_resize_observation_pending: false,
            middle_click_closes_tabs: true,
            root_focus_handle: None,
            focus_observed: false,
        }
    }

    /// Returns the Core group currently rendered by this PanelWindow, if any.
    pub fn group(&self) -> Option<FloatingGroupId> {
        self.group
    }

    /// Changes which composition this panel's permanent window hosts.
    pub(crate) fn set_host_group(
        &mut self,
        group: Option<FloatingGroupId>,
        window: &mut Window,
    ) {
        if self.group == group {
            return;
        }
        self.group = group;
        self.column_scroll_handles.clear();
        self.open_context_menu = None;
        self.tab_press.set(None);
        self.group_press.set(None);
        DockInteractionCoordinator::release(window, &self.pointer_capture);
    }

    /// Enables or disables closing a tab with the middle mouse button.
    pub fn middle_click_closes_tabs(mut self, enabled: bool) -> Self {
        self.middle_click_closes_tabs = enabled;
        self
    }
}

impl Render for FloatingHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(group) = self.group else {
            return div().id("empty-panel-window").size_full();
        };

        if !self.controller_observed {
            cx.observe(&self.controller, |_, _, host_cx| host_cx.notify())
                .detach();
            self.controller_observed = true;
        }
        if !self.manager_observed
            && let Some(manager) = self.manager.upgrade()
        {
            cx.observe(&manager, |_, _, host_cx| host_cx.notify())
                .detach();
            self.manager_observed = true;
        }

        // Reuse the controller's persistent PanelId -> AnyView cache. This call
        // is idempotent; factories run only for cache misses.
        self.controller.update(cx, |controller, panel_cx| {
            controller.ensure_panel_views(window, panel_cx);
        });

        let window_id = window.window_handle().window_id();
        let focus_handle = self
            .root_focus_handle
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        if !self.focus_observed {
            let controller = self.controller.clone();
            let manager = self.manager.clone();
            let tab_press = self.tab_press.clone();
            let group_press = self.group_press.clone();
            let pointer_capture = self.pointer_capture.clone();
            cx.on_focus_out(&focus_handle, window, move |_host, _event, window, app| {
                let preserve_interaction = manager.upgrade().is_some_and(|manager| {
                    manager.update(app, |manager, manager_cx| {
                        manager.should_preserve_pointer_interaction_on_focus_out(manager_cx)
                    })
                });
                if preserve_interaction {
                    return;
                }
                DockInteractionCoordinator::cancel_on_focus_loss(
                    &controller,
                    window,
                    &pointer_capture,
                    app,
                );
                tab_press.set(None);
                group_press.set(None);
            })
            .detach();
            self.focus_observed = true;
        }

        if !self.native_resize_observation_pending {
            self.native_resize_observation_pending = true;
            cx.defer_in(window, move |host, _, host_cx| {
                host.native_resize_observation_pending = false;
                // The permanent PanelWindow may stop hosting a group before
                // this deferred callback runs.
                if let Some(group) = host.group
                    && let Some(manager) = host.manager.upgrade()
                {
                    manager.update(host_cx, |manager, manager_cx| {
                        manager.observe_native_resize(group, window_id, manager_cx);
                    });
                }
            });
        }
        let snapshot = self.controller.update(cx, |controller, _| {
            floating_host_snapshot(controller, window_id, group)
        });
        let snapshot = match snapshot {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(
                    ?self.group,
                    %error,
                    "floating host could not obtain local Core geometry"
                );
                None
            }
        };
        let hide_header = self.manager.upgrade().is_some_and(|manager| {
            manager.update(cx, |manager, _| manager.hide_floating_header(group))
        });

        let mut root = div()
            .id(format!("floating-host-root-{:?}", group))
            .track_focus(&focus_handle)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(self.theme.colors.panel_surface_background));

        // The shared CompositionHeader moves the complete floating composition.
        // Every visible DockTab remains an individual panel drag source, including
        // the only tab in a floating window; it can detach and join another group.
        let manager = self.manager.clone();
        let controller = self.controller.clone();
        let tab_press = self.tab_press.clone();
        let group_press = self.group_press.clone();
        let pointer_capture = self.pointer_capture.clone();
        let pointer_capture_for_escape = pointer_capture.clone();
        let input_window_id = window_id;
        let floating_rect = snapshot.as_ref().map(|snapshot| snapshot.floating.rect);
        let dock_component_tab_zones = snapshot
            .as_ref()
            .map(floating_dock_component_tab_zones)
            .unwrap_or_default();
        // Each mouse-event handler owns its map because all three closures are `move`.
        let dock_component_tab_zones_for_down = dock_component_tab_zones.clone();
        let dock_component_tab_zones_for_move = dock_component_tab_zones.clone();
        let dock_component_tab_zones_for_up = dock_component_tab_zones;
        let dock_area_drag_area = snapshot.as_ref().map(|snapshot| {
            (
                snapshot.floating.rect,
                snapshot.rects.columns.clone(),
                snapshot.metrics,
            )
        });
        let title_input = canvas(
            move |bounds, window, _| Some(window.insert_hitbox(bounds, HitboxBehavior::Normal).id),
            move |_, hitbox, window, _| {
                if let Some(hitbox) = hitbox {
                    let manager_for_down = manager.clone();
                    let controller_for_down = controller.clone();
                    let tab_press_for_down = tab_press.clone();
                    let group_press_for_down = group_press.clone();
                    let pointer_capture_for_down = pointer_capture.clone();
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, app| {
                        if phase != DispatchPhase::Capture
                            || event.button != MouseButton::Left
                            || !hitbox.is_hovered(window)
                            || DockInteractionCoordinator::has_capture(&pointer_capture_for_down)
                            || DockInteractionCoordinator::window_has_capture(window)
                            || tab_press_for_down.get().is_some()
                            || group_press_for_down.get().is_some()
                            || controller_for_down.update(app, |controller, _| {
                                controller.has_active_pointer_interaction()
                            })
                        {
                            return;
                        }
                        let native_interaction_active =
                            manager_for_down.upgrade().is_some_and(|manager| {
                                manager.update(app, |manager, manager_cx| {
                                    manager.native_move_active()
                                        || manager.drag_host_manager().is_some_and(|drag_host| {
                                            drag_host.update(manager_cx, |drag_host, _| {
                                                drag_host.active_drag_host_id().is_some()
                                            })
                                        })
                                })
                            });
                        if native_interaction_active {
                            return;
                        }
                        let Ok(point) = crate::geometry::DockGeometry::point_from_gpui(event.position) else {
                            return;
                        };
                        let core_hit = controller_for_down.update(app, |controller, _| {
                            controller
                                .hit_floating_window(input_window_id, group, point)
                                .ok()
                        });
                        // FloatingWindowContent can render DockTab independently of the
                        // DockGroupView. Normalize the tab's own geometry before the
                        // pointer handler decides between moving this window and tearing
                        // the individual panel off into another Floating window.
                        let dock_component_tab_hit =
                            controller_for_down.update(app, |controller, _| {
                                let rects = controller
                                    .floating_geometry(input_window_id, group)
                                    .ok()?;
                                rects.tab_headers.iter().find_map(|header| {
                                    (dock_component_tab_zones_for_down.contains_key(&header.panel)
                                        && matches!(header.rect.contains(point), Ok(true)))
                                    .then_some(dock_core::HitResult::TabHeader {
                                        group: header.group,
                                        panel: header.panel,
                                        index: header.index,
                                    })
                                })
                            });
                        let hit = dock_component_tab_hit.or(core_hit);

                        if let Some(resize_hit @ dock_core::HitResult::Splitter { .. }) =
                            hit.clone()
                        {
                            let floating_rects = controller_for_down.update(app, |controller, _| {
                                controller
                                    .floating_geometry(input_window_id, group)
                                    .ok()
                                    .cloned()
                            });
                            if let Some(rects) = floating_rects {
                                begin_resize_hit_with_rects(
                                    &controller_for_down,
                                    resize_hit,
                                    point,
                                    &rects,
                                    app,
                                );
                                let resize_started = controller_for_down
                                    .update(app, |controller, _| {
                                        controller.resize_interaction().is_some()
                                    });
                                if resize_started {
                                    if !DockInteractionCoordinator::capture(
                                        window,
                                        hitbox,
                                        &pointer_capture_for_down,
                                    ) {
                                        DockInteractionCoordinator::cancel(
                                            &controller_for_down,
                                            window,
                                            &pointer_capture_for_down,
                                            app,
                                        );
                                        return;
                                    }
                                    app.stop_propagation();
                                    return;
                                }
                            }
                        }

                        // The remaining CompositionHeader region controls moving the whole Floating.
                        let floating_title_region = if !hide_header {
                            dock_area_drag_area.as_ref().and_then(
                                |(area_rect, columns, metrics)| {
                                    floating_dock_area_title_pointer_region(
                                        *area_rect,
                                        columns,
                                        point,
                                        *metrics,
                                    )
                                },
                            )
                        } else {
                            None
                        };
                        if let Some(is_title_grip) = floating_title_region {
                            if is_title_grip
                                && let Some(manager) = manager_for_down.upgrade()
                            {
                                manager.update(app, |manager, manager_cx| {
                                    manager.begin_existing_window_move(group, point, manager_cx)
                                });
                            }
                            // The header owns this point, including the fixed controls.
                            // Do not let a button press fall through to the Group or Tab below.
                            app.stop_propagation();
                            return;
                        }

                        let group_candidate = controller_for_down.update(app, |controller, _| {
                            let dock_group = match hit.clone()? {
                                dock_core::HitResult::Group { group }
                                | dock_core::HitResult::TabStrip { group } => group,
                                _ => return None,
                            };
                            let rects = controller.floating_geometry(input_window_id, group).ok()?;
                            let group_rect = rects
                                .groups
                                .iter()
                                .find(|rect| rect.group == dock_group)?
                                .rect;
                            let in_column = controller.layout().nodes().any(|(_, node)| {
                                matches!(
                                    node,
                                    Node::Column(column) if column.groups.contains(&dock_group)
                                )
                            });
                            let zone = GroupDragArea::drag_zone(
                                group_rect,
                                controller.metrics(),
                                in_column,
                            );
                            GroupDragArea::contains(zone, point)
                                .then_some((dock_group, controller.layout().version()))
                        });
                        if let Some((dock_group, layout_version)) = group_candidate {
                            let is_single_tab_floating = controller_for_down.update(
                                app,
                                |controller, _| {
                                    floating_has_single_tab_dock(
                                        controller.layout(),
                                        group,
                                        dock_group,
                                    )
                                },
                            );
                            if is_single_tab_floating
                                && let Some(manager) = manager_for_down.upgrade()
                                && manager.update(app, |manager, manager_cx| {
                                    manager.begin_existing_window_move(
                                        group,
                                        point,
                                        manager_cx,
                                    )
                                })
                            {
                                app.stop_propagation();
                                return;
                            }

                            begin_drag_at(
                                &controller_for_down,
                                dock_core::DragPayload::Group { group: dock_group },
                                dock_core::DragHandle::GroupGrip,
                                point,
                                DragSourceHost::Floating { group },
                                input_window_id,
                                app,
                            );
                            group_press_for_down.set(Some(FloatingGroupPress {
                                group: dock_group,
                                layout_version,
                            }));
                            if !DockInteractionCoordinator::capture(
                                window,
                                hitbox,
                                &pointer_capture_for_down,
                            ) {
                                group_press_for_down.set(None);
                                DockInteractionCoordinator::cancel(
                                    &controller_for_down,
                                    window,
                                    &pointer_capture_for_down,
                                    app,
                                );
                            }
                            app.stop_propagation();
                            return;
                        }

                        let press = controller_for_down.update(app, |controller, _| {
                            let dock_core::HitResult::TabHeader {
                                group: tab_group,
                                panel,
                                ..
                            } = hit.clone()?
                            else {
                                return None;
                            };
                            // FloatingWindowContent uses DockTab for every Group.
                            // The complete header is available for activation and drag.
                            Some((tab_group, panel, controller.layout().version()))
                        });
                        if let Some((tab_group, panel, layout_version)) = press {
                            let is_single_tab_floating = controller_for_down.update(
                                app,
                                |controller, _| {
                                    floating_has_single_tab_dock(
                                        controller.layout(),
                                        group,
                                        tab_group,
                                    )
                                },
                            );
                            if is_single_tab_floating
                                && let Some(manager) = manager_for_down.upgrade()
                                && manager.update(app, |manager, manager_cx| {
                                    manager.begin_existing_window_move(
                                        group,
                                        point,
                                        manager_cx,
                                    )
                                })
                            {
                                app.stop_propagation();
                                return;
                            }

                            tab_press_for_down.set(Some(FloatingTabPress {
                                tab_group,
                                panel,
                                origin: point,
                                layout_version,
                            }));
                            // Capture at press so leaving this native window before
                            // the next GPUI move event still reaches the drag pipeline.
                            if !DockInteractionCoordinator::capture(
                                window,
                                hitbox,
                                &pointer_capture_for_down,
                            ) {
                                tab_press_for_down.set(None);
                            }
                        }
                    });

                    let manager_for_move = manager.clone();
                    let controller_for_move = controller.clone();
                    let tab_press_for_move = tab_press.clone();
                    let group_press_for_move = group_press.clone();
                    let pointer_capture_for_move = pointer_capture.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, app| {
                        if phase != DispatchPhase::Capture || !event.dragging() {
                            return;
                        }
                        let resize_active = controller_for_move
                            .update(app, |controller, _| controller.resize_interaction().is_some());
                        if resize_active {
                            if !DockInteractionCoordinator::ensure_current(
                                &controller_for_move,
                                window,
                                &pointer_capture_for_move,
                                app,
                            ) {
                                app.stop_propagation();
                                return;
                            }
                            if DockInteractionCoordinator::capture_was_lost(
                                window,
                                &pointer_capture_for_move,
                            ) {
                                DockInteractionCoordinator::cancel(
                                    &controller_for_move,
                                    window,
                                    &pointer_capture_for_move,
                                    app,
                                );
                                app.stop_propagation();
                                return;
                            }
                            let Ok(point) =
                                crate::geometry::DockGeometry::point_from_gpui(event.position)
                            else {
                                return;
                            };
                            let should_schedule = controller_for_move.update(app, |controller, _| {
                                controller.queue_resize_frame_point(point)
                            });
                            if should_schedule {
                                let controller = controller_for_move.clone();
                                window.on_next_frame(move |_, app| {
                                    let point = controller
                                        .update(app, |controller, _| {
                                            controller.take_resize_frame_point()
                                        });
                                    if let Some(point) = point {
                                        update_resize(&controller, point, app);
                                    }
                                });
                            }
                            app.stop_propagation();
                            return;
                        }
                        if let Some(group_press) = group_press_for_move.get() {
                            let layout_is_current =
                                controller_for_move.update(app, |controller, _| {
                                    controller.layout().version() == group_press.layout_version
                                });
                            if !layout_is_current {
                                group_press_for_move.set(None);
                                DockInteractionCoordinator::cancel(
                                    &controller_for_move,
                                    window,
                                    &pointer_capture_for_move,
                                    app,
                                );
                                app.stop_propagation();
                                return;
                            }
                            let manager_entity = manager_for_move.upgrade();
                            let drag_host_active = manager_entity
                                .as_ref()
                                .and_then(|manager| {
                                    manager.update(app, |manager, _| manager.drag_host_manager())
                                })
                                .is_some_and(|drag_host| {
                                    drag_host.update(app, |manager, _| {
                                        manager.active_drag_host_id().is_some()
                                    })
                                });
                            if drag_host_active {
                                group_press_for_move.set(None);
                                if DockInteractionCoordinator::has_capture(
                                    &pointer_capture_for_move,
                                ) {
                                    DockInteractionCoordinator::release(
                                        window,
                                        &pointer_capture_for_move,
                                    );
                                }
                                app.stop_propagation();
                                return;
                            }
                            if manager_entity.as_ref().is_some_and(|manager| {
                                manager.update(app, |manager, _| manager.native_move_active())
                            }) {
                                app.stop_propagation();
                                return;
                            }
                            if DockInteractionCoordinator::capture_was_lost(
                                window,
                                &pointer_capture_for_move,
                            ) {
                                group_press_for_move.set(None);
                                DockInteractionCoordinator::cancel(
                                    &controller_for_move,
                                    window,
                                    &pointer_capture_for_move,
                                    app,
                                );
                                app.stop_propagation();
                                return;
                            }
                            let Ok(point) =
                                crate::geometry::DockGeometry::point_from_gpui(event.position)
                            else {
                                app.stop_propagation();
                                return;
                            };
                            let dragging = update_drag_at(&controller_for_move, point, app)
                                || controller_for_move.update(app, |controller, _| {
                                    matches!(
                                        controller.drag().session().state(),
                                        dock_core::DragState::Dragging {
                                            payload: dock_core::DragPayload::Group { group },
                                            ..
                                        } if group == group_press.group
                                    )
                                });
                            if dragging {
                                let leaves_source =
                                    controller_for_move.update(app, |controller, _| {
                                        let Some(rects) = controller
                                            .floating_geometry(input_window_id, group)
                                            .ok()
                                        else {
                                            return true;
                                        };
                                        let Some(group_rect) = rects
                                            .groups
                                            .iter()
                                            .find(|rect| rect.group == group_press.group)
                                            .map(|rect| rect.rect)
                                        else {
                                            return true;
                                        };
                                        let in_column =
                                            controller.layout().nodes().any(|(_, node)| {
                                                matches!(
                                                    node,
                                                    Node::Column(column)
                                                        if column.groups.contains(&group_press.group)
                                                )
                                            });
                                        let zone = GroupDragArea::drag_zone(
                                            group_rect,
                                            controller.metrics(),
                                            in_column,
                                        );
                                        !GroupDragArea::contains(zone, point)
                                    });
                                let over_other_window =
                                    manager_entity.as_ref().is_some_and(|manager| {
                                        manager.update(app, |manager, _| {
                                            manager.cursor_over_other_docking_window(group)
                                        })
                                    });
                                if leaves_source || over_other_window {
                                    let drag_host_manager =
                                        manager_entity.as_ref().and_then(|manager| {
                                            manager.update(app, |manager, _| {
                                                manager.drag_host_manager()
                                            })
                                        });
                                    if let Some(drag_host_manager) = drag_host_manager {
                                        if !begin_group_drag_host(
                                            &controller_for_move,
                                            &drag_host_manager,
                                            app,
                                        ) {
                                            tracing::debug!(
                                                "Floating Group drag could not start DragHost"
                                            );
                                        }
                                    } else {
                                        controller_for_move.update(app, |controller, cx| {
                                            let _ = controller.cancel_pointer_interaction();
                                            cx.notify();
                                        });
                                    }
                                    group_press_for_move.set(None);
                                    DockInteractionCoordinator::release(
                                        window,
                                        &pointer_capture_for_move,
                                    );
                                    app.stop_propagation();
                                    return;
                                }
                            }
                            app.stop_propagation();
                            return;
                        }

                        let Some(press) = tab_press_for_move.get() else {
                            return;
                        };
                        let manager_entity = manager_for_move.upgrade();
                        let drag_host_active = manager_entity
                            .as_ref()
                            .and_then(|manager| {
                                manager.update(app, |manager, _| manager.drag_host_manager())
                            })
                            .is_some_and(|drag_host| {
                                drag_host.update(app, |manager, _| {
                                    manager.active_drag_host_id().is_some()
                                })
                            });
                        if drag_host_active {
                            tab_press_for_move.set(None);
                            if DockInteractionCoordinator::has_capture(&pointer_capture_for_move) {
                                DockInteractionCoordinator::release(window, &pointer_capture_for_move);
                            }
                            return;
                        }
                        if manager_entity.as_ref().is_some_and(|manager| {
                            manager
                                .update(app, |manager, _| manager.native_move_active())
                        }) {
                            return;
                        }
                        if DockInteractionCoordinator::capture_was_lost(
                            window,
                            &pointer_capture_for_move,
                        ) {
                            tab_press_for_move.set(None);
                            DockInteractionCoordinator::cancel(
                                &controller_for_move,
                                window,
                                &pointer_capture_for_move,
                                app,
                            );
                            return;
                        }
                        let Ok(point) = crate::geometry::DockGeometry::point_from_gpui(event.position) else {
                            return;
                        };
                        let drag = FloatingTabDrag {
                            source_window_id: input_window_id,
                            source_group: group,
                            panel: press.panel,
                            press_origin: press.origin,
                            layout_version: press.layout_version,
                        };
                        let dragging = update_floating_tab_drag(
                            &controller_for_move,
                            drag,
                            point,
                            app,
                        );
                        if !dragging {
                            if !DockInteractionCoordinator::release_if_inactive(
                                &controller_for_move,
                                window,
                                &pointer_capture_for_move,
                                app,
                            ) {
                                tab_press_for_move.set(None);
                            }
                            return;
                        }
                        // Capture only after the Core threshold is crossed. A
                        // normal click therefore continues to reach tab controls.
                        if !DockInteractionCoordinator::capture(
                            window,
                            hitbox,
                            &pointer_capture_for_move,
                        ) {
                            tab_press_for_move.set(None);
                            DockInteractionCoordinator::cancel(
                                &controller_for_move,
                                window,
                                &pointer_capture_for_move,
                                app,
                            );
                            return;
                        }

                        let leaves_source = if let Some(zone) =
                            dock_component_tab_zones_for_move.get(&press.panel).copied()
                        {
                            DockTab::leaves_strip_zone(zone, point)
                        } else {
                            controller_for_move.update(app, |controller, _| {
                                let strip = controller
                                    .floating_geometry(input_window_id, group)
                                    .ok()
                                    .and_then(|rects| {
                                        rects
                                            .tab_strips
                                            .iter()
                                            .find(|strip| strip.group == press.tab_group)
                                            .map(|strip| strip.rect)
                                    });
                                if let Some(strip) = strip {
                                    DockTab::leaves_strip_zone(strip, point)
                                } else {
                                    floating_rect.is_some_and(|rect| {
                                        rect.contains(point).map(|contains| !contains).unwrap_or(true)
                                    })
                                }
                            })
                        };
                        let over_other_window = manager_entity.as_ref().is_some_and(|manager| {
                            manager
                                .update(app, |manager, _| {
                                    manager.cursor_over_other_docking_window(group)
                                })
                                
                        });
                        if floating_tab_drag_host_required(leaves_source, over_other_window)
                            && let Some(drag_host_manager) = manager_entity.as_ref().and_then(|manager| {
                                manager.update(app, |manager, _| manager.drag_host_manager())
                            })
                        {
                            if !begin_panel_drag_host(
                                &controller_for_move,
                                &drag_host_manager,
                                app,
                            ) {
                                tracing::debug!("Floating Panel drag could not start DragHost");
                            }
                            tab_press_for_move.set(None);
                            DockInteractionCoordinator::release(
                                window,
                                &pointer_capture_for_move,
                            );
                            app.stop_propagation();
                            return;
                        }
                        app.stop_propagation();
                    });

                    let manager_for_up = manager.clone();
                    let controller_for_up = controller.clone();
                    let tab_press_for_up = tab_press.clone();
                    let group_press_for_up = group_press.clone();
                    let pointer_capture_for_up = pointer_capture.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, app| {
                        if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                            return;
                        }
                        let resize_active = controller_for_up
                            .update(app, |controller, _| controller.resize_interaction().is_some());
                        if resize_active {
                            if !DockInteractionCoordinator::ensure_current(
                                &controller_for_up,
                                window,
                                &pointer_capture_for_up,
                                app,
                            ) {
                                app.stop_propagation();
                                return;
                            }
                            if DockInteractionCoordinator::capture_was_lost(
                                window,
                                &pointer_capture_for_up,
                            ) {
                                DockInteractionCoordinator::cancel(
                                    &controller_for_up,
                                    window,
                                    &pointer_capture_for_up,
                                    app,
                                );
                                app.stop_propagation();
                                return;
                            }
                            let Ok(point) =
                                crate::geometry::DockGeometry::point_from_gpui(event.position)
                            else {
                                DockInteractionCoordinator::cancel(
                                    &controller_for_up,
                                    window,
                                    &pointer_capture_for_up,
                                    app,
                                );
                                app.stop_propagation();
                                return;
                            };
                            let floating_rects = controller_for_up.update(app, |controller, _| {
                                controller
                                    .floating_geometry(input_window_id, group)
                                    .ok()
                                    .cloned()
                            });
                            controller_for_up.update(app, |controller, _| {
                                controller.clear_resize_frame_point();
                            });
                            if let Some(rects) = floating_rects {
                                finish_resize_with_rects(
                                    &controller_for_up,
                                    point,
                                    event.click_count,
                                    &rects,
                                    app,
                                );
                                DockInteractionCoordinator::release(
                                    window,
                                    &pointer_capture_for_up,
                                );
                            } else {
                                DockInteractionCoordinator::cancel(
                                    &controller_for_up,
                                    window,
                                    &pointer_capture_for_up,
                                    app,
                                );
                            }
                            app.stop_propagation();
                            return;
                        }
                        if let Some(manager) = manager_for_up.upgrade() {
                            let drag_host_active = manager
                                .update(app, |manager, _| manager.drag_host_manager())
                                .is_some_and(|drag_host| {
                                    drag_host.update(app, |manager, _| {
                                        manager.active_drag_host_id().is_some()
                                    })
                                });
                            if drag_host_active {
                                tab_press_for_up.set(None);
                                group_press_for_up.set(None);
                                if DockInteractionCoordinator::has_capture(
                                    &pointer_capture_for_up,
                                ) {
                                    DockInteractionCoordinator::release(
                                        window,
                                        &pointer_capture_for_up,
                                    );
                                }
                                return;
                            }
                            if manager.update(app, |manager, _| manager.native_move_active()) {
                                return;
                            }
                        }
                        if DockInteractionCoordinator::capture_was_lost(
                            window,
                            &pointer_capture_for_up,
                        ) {
                            tab_press_for_up.set(None);
                            group_press_for_up.set(None);
                            DockInteractionCoordinator::cancel(
                                &controller_for_up,
                                window,
                                &pointer_capture_for_up,
                                app,
                            );
                            return;
                        }
                        if let Some(group_press) = group_press_for_up.get() {
                            group_press_for_up.set(None);
                            let layout_is_current =
                                controller_for_up.update(app, |controller, _| {
                                    controller.layout().version() == group_press.layout_version
                                });
                            if !layout_is_current {
                                DockInteractionCoordinator::cancel(
                                    &controller_for_up,
                                    window,
                                    &pointer_capture_for_up,
                                    app,
                                );
                                app.stop_propagation();
                                return;
                            }
                            let Ok(point) =
                                crate::geometry::DockGeometry::point_from_gpui(event.position)
                            else {
                                DockInteractionCoordinator::release(
                                    window,
                                    &pointer_capture_for_up,
                                );
                                app.stop_propagation();
                                return;
                            };
                            let dragging = update_drag_at(&controller_for_up, point, app)
                                || controller_for_up.update(app, |controller, _| {
                                    matches!(
                                        controller.drag().session().state(),
                                        dock_core::DragState::Dragging {
                                            payload: dock_core::DragPayload::Group { group },
                                            ..
                                        } if group == group_press.group
                                    )
                                });
                            if dragging {
                                let leaves_source =
                                controller_for_up.update(app, |controller, _| {
                                    let Some(rects) = controller
                                        .floating_geometry(input_window_id, group)
                                        .ok()
                                    else {
                                        return true;
                                    };
                                    let Some(group_rect) = rects
                                        .groups
                                        .iter()
                                        .find(|rect| rect.group == group_press.group)
                                        .map(|rect| rect.rect)
                                    else {
                                        return true;
                                    };
                                    let in_column =
                                        controller.layout().nodes().any(|(_, node)| {
                                            matches!(
                                                node,
                                                Node::Column(column)
                                                    if column.groups.contains(&group_press.group)
                                            )
                                        });
                                    let zone = GroupDragArea::drag_zone(
                                        group_rect,
                                        controller.metrics(),
                                        in_column,
                                    );
                                    !GroupDragArea::contains(zone, point)
                                });
                                let manager_entity = manager_for_up.upgrade();
                                let over_other_window =
                                    manager_entity.as_ref().is_some_and(|manager| {
                                        manager.update(app, |manager, _| {
                                            manager.cursor_over_other_docking_window(group)
                                        })
                                    });
                                if leaves_source || over_other_window {
                                    if let Some(drag_host_manager) =
                                        manager_entity.as_ref().and_then(|manager| {
                                            manager.update(app, |manager, _| {
                                                manager.drag_host_manager()
                                            })
                                        })
                                    {
                                        if !begin_group_drag_host(
                                            &controller_for_up,
                                            &drag_host_manager,
                                            app,
                                        ) {
                                            tracing::debug!(
                                                "Floating Group drag could not start DragHost on release"
                                            );
                                        }
                                    } else {
                                        controller_for_up.update(app, |controller, cx| {
                                            let _ = controller.cancel_pointer_interaction();
                                            cx.notify();
                                        });
                                    }
                                } else {
                                    controller_for_up.update(app, |controller, cx| {
                                        let _ = controller.cancel_pointer_interaction();
                                        cx.notify();
                                    });
                                }
                            } else {
                                controller_for_up.update(app, |controller, cx| {
                                    let _ = controller.cancel_pointer_interaction();
                                    cx.notify();
                                });
                            }
                            DockInteractionCoordinator::release(window, &pointer_capture_for_up);
                            app.stop_propagation();
                            return;
                        }
                        let Some(press) = tab_press_for_up.get() else {
                            if DockInteractionCoordinator::has_capture(&pointer_capture_for_up) {
                                DockInteractionCoordinator::release(
                                    window,
                                    &pointer_capture_for_up,
                                );
                            }
                            return;
                        };
                        tab_press_for_up.set(None);
                        let Ok(point) = crate::geometry::DockGeometry::point_from_gpui(event.position) else {
                            DockInteractionCoordinator::release(
                                window,
                                &pointer_capture_for_up,
                            );
                            return;
                        };

                        // MouseUp can be the first event delivered beyond the drag threshold.
                        // Give the shared floating-tab pipeline the release position before
                        // deciding whether this tab crossed its source strip boundary.
                        let drag = FloatingTabDrag {
                            source_window_id: input_window_id,
                            source_group: group,
                            panel: press.panel,
                            press_origin: press.origin,
                            layout_version: press.layout_version,
                        };
                        let dragging = update_floating_tab_drag(
                            &controller_for_up,
                            drag,
                            point,
                            app,
                        );
                        if dragging {
                            let leaves_source = if let Some(zone) =
                                dock_component_tab_zones_for_up.get(&press.panel).copied()
                            {
                                DockTab::leaves_strip_zone(zone, point)
                            } else {
                                controller_for_up.update(app, |controller, _| {
                                    let strip = controller
                                        .floating_geometry(input_window_id, group)
                                        .ok()
                                        .and_then(|rects| {
                                            rects
                                                .tab_strips
                                                .iter()
                                                .find(|strip| strip.group == press.tab_group)
                                                .map(|strip| strip.rect)
                                        });
                                    if let Some(strip) = strip {
                                        DockTab::leaves_strip_zone(strip, point)
                                    } else {
                                        floating_rect.is_some_and(|rect| {
                                            rect.contains(point).map(|contains| !contains).unwrap_or(true)
                                        })
                                    }
                                })
                            };
                            let manager_entity = manager_for_up.upgrade();
                            let over_other_window = manager_entity.as_ref().is_some_and(|manager| {
                                manager.update(app, |manager, _| {
                                    manager.cursor_over_other_docking_window(group)
                                })
                            });
                            if floating_tab_drag_host_required(leaves_source, over_other_window)
                                && let Some(drag_host_manager) = manager_entity.as_ref().and_then(|manager| {
                                    manager.update(app, |manager, _| manager.drag_host_manager())
                                })
                            {
                                if !begin_panel_drag_host(
                                    &controller_for_up,
                                    &drag_host_manager,
                                    app,
                                ) {
                                    tracing::debug!("Floating Panel drag could not start DragHost on release");
                                }
                                DockInteractionCoordinator::release(
                                    window,
                                    &pointer_capture_for_up,
                                );
                                app.stop_propagation();
                                return;
                            }
                        }

                        let (was_dragging, was_pressed) =
                            controller_for_up.update(app, |controller, _| {
                                match controller.drag().session().state() {
                                    dock_core::DragState::Dragging {
                                        payload: dock_core::DragPayload::Panel { panel },
                                        ..
                                    } if panel == press.panel => (true, false),
                                    dock_core::DragState::Pressed {
                                        payload: dock_core::DragPayload::Panel { panel },
                                        ..
                                    } if panel == press.panel => (false, true),
                                    _ => (false, false),
                                }
                            });
                        if was_dragging {
                            finish_floating_tab_drag(
                                &controller_for_up,
                                input_window_id,
                                group,
                                press.panel,
                                point,
                                app,
                            );
                            DockInteractionCoordinator::release(
                                window,
                                &pointer_capture_for_up,
                            );
                            app.stop_propagation();
                            return;
                        }

                        if was_pressed {
                            DockInteractionCoordinator::cancel(
                                &controller_for_up,
                                window,
                                &pointer_capture_for_up,
                                app,
                            );
                        } else {
                            DockInteractionCoordinator::release(
                                window,
                                &pointer_capture_for_up,
                            );
                        }
                        let should_activate = controller_for_up.update(app, |controller, _| {
                            if controller.layout().version() != press.layout_version {
                                return false;
                            }
                            let core_hit = controller
                                .hit_floating_window(input_window_id, group, point)
                                .unwrap_or(dock_core::HitResult::Miss);
                            let released_hit = controller
                                .floating_geometry(input_window_id, group)
                                .ok()
                                .and_then(|rects| {
                                    rects.tab_headers.iter().find_map(|header| {
                                        (dock_component_tab_zones_for_up.contains_key(&header.panel)
                                            && matches!(header.rect.contains(point), Ok(true)))
                                        .then_some(dock_core::HitResult::TabHeader {
                                            group: header.group,
                                            panel: header.panel,
                                            index: header.index,
                                        })
                                    })
                                })
                                .unwrap_or(core_hit);
                            let dock_core::HitResult::TabHeader {
                                group: released_group,
                                panel: released_panel,
                                ..
                            } = released_hit
                            else {
                                return false;
                            };
                            if released_group != press.tab_group || released_panel != press.panel {
                                return false;
                            }
                            controller
                                .floating_geometry(input_window_id, group)
                                .ok()
                                .is_some_and(|rects| {
                                    rects.tab_headers.iter().any(|header| {
                                        header.group == released_group
                                            && header.panel == released_panel
                                            && matches!(header.rect.contains(point), Ok(true))
                                    })
                                })
                        });
                        if should_activate {
                            controller_for_up.update(app, |controller, cx| {
                                if let Err(error) = controller.dispatch_in(
                                    DockCommand::ActivateTab {
                                        group: press.tab_group,
                                        panel: press.panel,
                                    },
                                    cx,
                                ) {
                                    tracing::debug!(%error, "could not activate floating tab");
                                }
                            });
                            app.stop_propagation();
                        }
                    });
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        root = root.child(title_input);

        let Some(snapshot) = snapshot else {
            return root;
        };

        // GPUI scroll state is presentation-only and follows the lifetime of
        // the Column node within this one floating composition.
        let overflow_columns = snapshot
            .rects
            .vertical_overflow
            .iter()
            .map(|overflow| overflow.owner)
            .collect::<HashSet<_>>();
        self.column_scroll_handles.retain(|column, _| {
            snapshot.column_ids.contains(column) && overflow_columns.contains(column)
        });
        for column in overflow_columns {
            self.column_scroll_handles.entry(column).or_default();
        }

        // Every PanelRect in compute_floating_rects belongs to this group and
        // is already expressed in local (0, 0) coordinates.
        let panel_views = snapshot
            .rects
            .panels
            .iter()
            .map(|panel_rect| {
                let panel_rect = *panel_rect;
                self.controller.update(cx, |controller, panel_cx| {
                    (
                        panel_rect,
                        controller.panel_view(panel_rect.panel, window, panel_cx),
                    )
                })
            })
            .collect::<Vec<_>>();

        let dragged_group = self.controller.update(cx, |controller, _| {
            controller.dragged_group_from(window_id)
        });
        let pending_transfers = self.controller.update(cx, |controller, _| {
            controller.pending_panel_view_transfers_from(window_id)
        });
        if !pending_transfers.is_empty()
            && pending_transfers.iter().all(|panel| {
                panel_views
                    .iter()
                    .all(|(rect, view)| rect.panel != *panel || view.is_none())
            })
        {
            let controller = self.controller.clone();
            window.on_next_frame(move |_, app| {
                controller.update(app, |controller, controller_cx| {
                    let mut confirmed = false;
                    for panel in pending_transfers {
                        confirmed |= controller
                            .confirm_panel_view_source_detached(panel, window_id);
                    }
                    if confirmed {
                        controller_cx.notify();
                    }
                });
            });
        }

        // A DragHost handoff completes only after this persistent host has
        // built a frame that can mount every transferred active PanelView.
        let handoff_panels = self.manager.upgrade().and_then(|manager| {
            manager.update(cx, |manager, manager_cx| {
                manager.pending_drag_host_handoff_panels(group, window_id, manager_cx)
            })
        });
        if let Some(required_panels) = handoff_panels.filter(|required| {
            required.iter().all(|panel| {
                panel_views
                    .iter()
                    .all(|(rect, view)| rect.panel != *panel || view.is_some())
            })
        }) {
            let manager = self.manager.clone();
            window.on_next_frame(move |_, app| {
                if let Some(manager) = manager.upgrade() {
                    let _ = manager.update(app, |manager, manager_cx| {
                        manager.notify_drag_host_handoff_rendered(
                            group,
                            window_id,
                            &required_panels,
                            manager_cx,
                        );
                    });
                }
            });
        }

        if self.open_context_menu.as_ref().is_some_and(|open| {
            !context_menu_target_exists(
                open.target,
                &snapshot.group_presentations,
                &snapshot.column_ids,
                Some(&snapshot.rects),
            )
        }) {
            self.open_context_menu = None;
        }

        let toggle_host = cx.entity().downgrade();
        let toggle_context_menu: GroupMenuToggle = Rc::new(move |target, anchor, app| {
            if let Some(host) = toggle_host.upgrade() {
                host.update(app, |host, host_cx| {
                    host.open_context_menu = if host
                        .open_context_menu
                        .as_ref()
                        .is_some_and(|open| open.target == target)
                    {
                        None
                    } else {
                        Some(FloatingHostOpenMenu {
                            target,
                            anchor,
                            open_submenu: None,
                        })
                    };
                    host_cx.notify();
                });
            }
        });

        let dismiss_host = cx.entity().downgrade();
        let dismiss_context_menu: ContextMenuDismiss = Rc::new(move |app| {
            if let Some(host) = dismiss_host.upgrade() {
                host.update(app, |host, host_cx| {
                    host.open_context_menu = None;
                    host_cx.notify();
                });
            }
        });

        let submenu_host = cx.entity().downgrade();
        let toggle_context_submenu: ContextMenuSubmenuToggle = Rc::new(move |index, app| {
            if let Some(host) = submenu_host.upgrade() {
                host.update(app, |host, host_cx| {
                    if let Some(open) = host.open_context_menu.as_mut() {
                        open.open_submenu = index;
                        host_cx.notify();
                    }
                });
            }
        });

        root = root.child(FloatingWindowContent::render(
            snapshot.floating,
            &snapshot.rects.columns,
            &snapshot.rects.groups,
            panel_views,
            dragged_group,
            &snapshot.rects.tab_strips,
            &snapshot.rects.tab_headers,
            &snapshot.rects.tab_overflows,
            &snapshot.group_presentations,
            &snapshot.column_groups,
            &snapshot.column_modes,
            &snapshot.rail_display_orders,
            &snapshot.rects.splitters,
            &snapshot.rects.vertical_overflow,
            &self.column_scroll_handles,
            None,
            snapshot.drag_preview,
            &snapshot.drag_zones,
            snapshot.metrics,
            hide_header,
            None,
            toggle_context_menu,
            self.middle_click_closes_tabs,
            self.controller.clone(),
            self.theme,
        ));

        if let Some(mut preview) = snapshot.resize_preview {
            if let Some(owner) = preview.scroll_owner {
                let scroll_offset = self
                    .column_scroll_handles
                    .get(&owner)
                    .map(|handle| handle.offset().y.as_f32().max(0.0))
                    .unwrap_or(0.0);
                preview.rect.origin.y -= scroll_offset;
            }
            root = root.child(render_resize_preview(preview, self.theme));
        }

        if let Some(preview) = snapshot.drag_preview {
            root = root.child(DropOverlay::render(
                snapshot.active_drop_zone,
                Some(preview),
                &snapshot.rects.groups,
                &snapshot.rects.columns,
                &snapshot.column_groups,
                &snapshot.rects.tab_headers,
                &snapshot.rects.tab_strips,
                None,
                snapshot.rects.floating_groups.first().map(|floating| floating.rect),
                self.theme,
            ));
            // Match MainHost: Floating hosts paint only resolved drop feedback.
            // DragHost/native move already represents the source; a second
            // cursor-following Ghost obscures the destination tab row.
        }

        if let Some(open) = self.open_context_menu.as_ref()
            && let Some(viewport) = snapshot.rects.viewport
        {
            let entries = build_context_menu_entries(
                open.target,
                &snapshot.rects,
                &snapshot.group_presentations,
                &snapshot.column_modes,
                self.controller.clone(),
                false,
            );
            if !entries.is_empty() {
                root = root.child(ContextMenu::render(
                    "floating-context-menu",
                    viewport,
                    open.anchor,
                    &entries,
                    open.open_submenu,
                    ContextMenuCallbacks {
                        on_submenu: toggle_context_submenu,
                        on_dismiss: dismiss_context_menu,
                    },
                    self.theme,
                ));
            }
        }

        let manager_for_escape = self.manager.clone();
        let controller_for_escape = self.controller.clone();
        let tab_press_for_escape = self.tab_press.clone();
        let group_press_for_escape = self.group_press.clone();
        let context_menu_is_open = self.open_context_menu.is_some();
        root = root.capture_key_down(move |event, window, app| {
            if context_menu_is_open
                || !event.keystroke.key.eq_ignore_ascii_case("escape")
            {
                return;
            }
            if let Some(manager) = manager_for_escape.upgrade() {
                if manager.update(app, |manager, _| manager.native_move_active()) {
                    return;
                }
                let drag_host_manager =
                    manager.update(app, |manager, _| manager.drag_host_manager());
                if drag_host_manager.is_some_and(|drag_host| {
                    drag_host.update(app, |manager, manager_cx| {
                        manager.cancel_active(manager_cx)
                    })
                }) {
                    tab_press_for_escape.set(None);
                    group_press_for_escape.set(None);
                    if DockInteractionCoordinator::has_capture(&pointer_capture_for_escape) {
                        DockInteractionCoordinator::release(window, &pointer_capture_for_escape);
                    }
                    app.stop_propagation();
                    return;
                }
            }
            if tab_press_for_escape.get().is_none()
                && group_press_for_escape.get().is_none()
            {
                return;
            }
            tab_press_for_escape.set(None);
            group_press_for_escape.set(None);
            DockInteractionCoordinator::cancel_on_escape(
                &controller_for_escape,
                window,
                &pointer_capture_for_escape,
                app,
            );
            app.stop_propagation();
        });

        root
    }
}

#[cfg(test)]
mod floating_tab_drag_host_tests {
    use super::floating_tab_drag_host_required;

    #[test]
    fn tab_drag_inside_source_strip_keeps_the_original_host() {
        assert!(!floating_tab_drag_host_required(false, false));
    }

    #[test]
    fn tab_drag_leaving_its_source_strip_uses_drag_host() {
        assert!(floating_tab_drag_host_required(true, false));
    }

    #[test]
    fn dragging_over_another_docking_window_uses_drag_host() {
        assert!(floating_tab_drag_host_required(false, true));
    }
}

#[cfg(test)]
mod floating_dock_area_header_hit_tests {
    use dock_core::{ColumnRect, DockMetrics, NodeId, Point, Rect, Size};
    use slotmap::Key;

    #[test]
    fn dock_area_header_takes_priority_over_its_overlapping_group_header() {
        let column = Rect::new(Point::new(10.0, 20.0), Size::new(240.0, 500.0));
        let metrics = DockMetrics::default();
        let columns = [ColumnRect::new(NodeId::null(), column)];

        assert_eq!(
            super::floating_dock_area_title_pointer_region(
                column,
                &columns,
                Point::new(100.0, 30.0),
                metrics,
            ),
            Some(true),
        );
        let collapse_button_center = column.origin.x
            + super::CompositionDragArea::collapse_button_left(
                column,
                columns[0],
                metrics.column_control_button_width,
            )
            + metrics.column_control_button_width * 0.5;
        assert_eq!(
            super::floating_dock_area_title_pointer_region(
                column,
                &columns,
                Point::new(collapse_button_center, 30.0),
                metrics,
            ),
            Some(false),
        );
        assert_eq!(
            super::floating_dock_area_title_pointer_region(
                column,
                &columns,
                Point::new(240.0, 30.0),
                metrics,
            ),
            Some(false),
        );
        assert_eq!(
            super::floating_dock_area_title_pointer_region(
                column,
                &columns,
                Point::new(100.0, 60.0),
                metrics,
            ),
            None,
        );
    }
}

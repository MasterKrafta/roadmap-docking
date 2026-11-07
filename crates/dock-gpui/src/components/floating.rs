use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use dock_core::{
    ColumnDisplayItem, ColumnMode, ColumnRect, ComputeRectsError, DockCommand, DockMetrics,
    DragHandle,
    DragPreview, DropZoneRect, FloatingGroupId, FloatingGroupRect, FlyoutState, GroupRect,
    LayoutRects, Node, NodeId, PanelId, PanelRect, Point, Rect, SplitterRect, TabHeaderRect,
    TabOverflowRect, TabStripRect, VerticalOverflow,
};
use gpui::{
    AnyView, Context, DispatchPhase, Entity, HitboxBehavior, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Render, ScrollHandle,
    FocusHandle, Styled, WeakEntity, Window, WindowId, canvas, div,
    prelude::InteractiveElement as _, px, rgb,
};
use ui::Theme;

use crate::{
    DockController, FloatingWindowManager,
    dock_components::{DockAreaDragArea, DockAreaView, DockColumnView, DockTab},
    input::drag::{
        begin_floating_tab_tearoff, finish_floating_tab_drag, update_floating_tab_drag,
        FloatingTabDrag,
    },
    dock_host::{
        build_context_menu_entries, collect_subtree_nodes, context_menu_target_exists,
    },
};

use super::{
    column::ColumnView,
    drop_overlay::{DragGhost, DropOverlay},
    context_menu::{
        ContextMenu, ContextMenuCallbacks, ContextMenuDismiss, ContextMenuSubmenuToggle,
        ContextMenuTarget, GroupMenuToggle,
    },
    group::GroupView,
    panel_host::PanelHost,
    splitter::SplitterView,
    tab_strip::GroupPresentation,
    collect_group_presentations,
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
        panels: Vec<(PanelRect, AnyView)>,
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

        // Each Column remains a normal Core subtree. Its groups, tab strips,
        // panel views and internal vertical splitters use the same components
        // as the main dock, only translated into this window's local space.
        for column in columns {
            let group_ids = column_groups
                .get(&column.column)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let column_group_rects = groups
                .iter()
                .filter(|group| group_ids.contains(&group.group))
                .copied()
                .collect::<Vec<_>>();
            let column_splitters = splitters
                .iter()
                .filter(|splitter| {
                    splitter.owner == column.column && splitter.axis == dock_core::Axis::Y
                })
                .copied()
                .collect::<Vec<_>>();
            let overflow = vertical_overflow
                .iter()
                .find(|overflow| overflow.owner == column.column)
                .copied();
            let column_active = group_ids
                .iter()
                .filter_map(|group| {
                    group_presentations
                        .get(group)
                        .and_then(|presentation| presentation.active_panel)
                })
                .collect::<HashSet<_>>();
            let column_panels = panels
                .iter()
                .filter(|(panel, _)| column_active.contains(&panel.panel))
                .cloned()
                .collect::<Vec<_>>();
            let mode = column_modes
                .get(&column.column)
                .copied()
                .unwrap_or(ColumnMode::Expanded);

            // Use the same dock-components Column implementation as MainDockView
            // when this Core Column is the supported single-group/single-tab shape.
            // Its AnyView is the controller's cached panel view; it is not rebuilt.
            let single_active_panel = if column_group_rects.len() == 1 {
                column_group_rects.first().and_then(|group| {
                    let presentation = group_presentations.get(&group.group)?;
                    if presentation.tabs.len() != 1 {
                        return None;
                    }
                    let tab = presentation.tabs.first()?;
                    if presentation.active_panel != Some(tab.panel) {
                        return None;
                    }
                    let panel_view = column_panels
                        .iter()
                        .find(|(panel, _)| panel.panel == tab.panel)
                        .map(|(_, view)| view.clone())?;
                    Some((tab.title.to_string(), panel_view))
                })
            } else {
                None
            };

            if let Some((title, panel_view)) = single_active_panel {
                root = root.child(
                    div()
                        .id(format!("floating-dock-components-column-{:?}", column.column))
                        .absolute()
                        .left(px(column.rect.origin.x))
                        .top(px(column.rect.origin.y))
                        .w(px(column.rect.size.width))
                        .h(px(column.rect.size.height))
                        .child(DockColumnView::render_active_with_area_header(
                            format!("floating-dock-components-{:?}", column.column),
                            title,
                            panel_view,
                            metrics,
                            theme,
                            !hide_header,
                        )),
                );
            } else {
                root = root.child(ColumnView::render(
                    *column,
                    mode,
                    &column_group_rects,
                    column_panels,
                    tab_strips,
                    tab_headers,
                    tab_overflows,
                    group_presentations,
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
                    &column_splitters,
                    overflow,
                    scroll_handles.get(&column.column).cloned(),
                    controller.clone(),
                    theme,
                    origin,
                    Some(floating.group),
                    !hide_header,
                ));
            }
        }

        // Tabs nodes that are not owned by a Column still use GroupView.
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

            root = root.child(GroupView::render(
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
                Some(floating.group),
                context_menu.clone(),
                middle_click_closes_tabs,
            ));
        }

        // X splits and splitters outside Columns are painted once above the
        // child surfaces. Column-owned Y splitters were already rendered by
        // ColumnView, where their scroll offset is available.
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
        root = root.child(PanelHost::render(loose_panels, origin));

        if !hide_header {
            root = root.child(DockAreaView::render_header(
                format!("floating-dock-area-{:?}", floating.group),
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
    drag_handle: Option<DragHandle>,
    drag_zones: Vec<DropZoneRect>,
    active_drop_zone: Option<dock_core::DropZoneRect>,
}

/// Gives the shared DockAreaHeader input priority over every Column below it.
/// Returns Some(false) for the collapse and close controls on the header.
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

    Some(DockAreaDragArea::contains(
        area_rect,
        columns,
        point,
        header_height,
        metrics.column_control_button_width,
    ))
}

fn floating_dock_component_group_zones(
    snapshot: &FloatingHostSnapshot,
) -> HashMap<NodeId, Rect> {
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
        if presentation.tabs.len() == 1
            && presentation.active_panel == presentation.tabs.first().map(|tab| tab.panel)
        {
            zones.insert(
                group.group,
                DockTab::drag_zone(group.rect, snapshot.metrics),
            );
        }
    }
    zones
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
        if presentation.tabs.len() == 1
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
    let drag_handle = controller.drag().session().handle();
    let drag_resolver = controller.floating_window_drop_resolver(window_id, group);
    let drag_zones = drag_resolver
        .map(|resolver| resolver.zones().to_vec())
        .unwrap_or_default();
    let active_drop_zone = drag_preview.and_then(|preview| {
        let zone = drag_resolver?
            .resolve_zone(preview.point)
            .ok()?
            .filter(|zone| preview.target == Some(zone.target))?;
        Some(zone)
    });

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
        drag_handle,
        drag_zones,
        active_drop_zone,
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
    pub(crate) fn set_host_group(&mut self, group: Option<FloatingGroupId>) {
        if self.group == group {
            return;
        }
        self.group = group;
        self.column_scroll_handles.clear();
        self.open_context_menu = None;
        self.tab_press.set(None);
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
            cx.on_focus_out(&focus_handle, window, move |_host, _event, window, app| {
                let preserve_interaction = manager.upgrade().is_some_and(|manager| {
                    manager.update(app, |manager, manager_cx| {
                        manager.should_preserve_pointer_interaction_on_focus_out(manager_cx)
                    })
                });
                if preserve_interaction {
                    return;
                }
                controller.update(app, |controller, controller_cx| {
                    let result = controller.cancel_pointer_interaction_on_focus_loss();
                    controller_cx.notify();
                    if let Err(error) = result {
                        tracing::warn!(
                            %error,
                            "could not cancel floating dock interaction on focus loss"
                        );
                    }
                });
                window.release_pointer();
                tab_press.set(None);
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

        // The shared DockAreaHeader moves the complete floating composition; DockTab remains a tab drag source.
        // Legacy TabHeaderView instances keep their individual-panel tear-off path.
        let manager = self.manager.clone();
        let controller = self.controller.clone();
        let tab_press = self.tab_press.clone();
        let input_window_id = window_id;
        let floating_rect = snapshot.as_ref().map(|snapshot| snapshot.floating.rect);
        let dock_component_tab_zones = snapshot
            .as_ref()
            .map(floating_dock_component_tab_zones)
            .unwrap_or_default();
        let dock_component_group_zones = snapshot
            .as_ref()
            .map(floating_dock_component_group_zones)
            .unwrap_or_default();
        // Each mouse-event handler owns its map because all three closures are `move`.
        let dock_component_tab_zones_for_down = dock_component_tab_zones.clone();
        let dock_component_tab_zones_for_move = dock_component_tab_zones.clone();
        let dock_component_tab_zones_for_up = dock_component_tab_zones;
        let dock_component_group_zones_for_down = dock_component_group_zones;
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
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, app| {
                        if phase != DispatchPhase::Capture
                            || event.button != MouseButton::Left
                            || !hitbox.is_hovered(window)
                        {
                            return;
                        }
                        let Ok(point) = crate::geometry::point_from_gpui(event.position) else {
                            return;
                        };
                        let hit = controller_for_down.update(app, |controller, _| {
                            controller
                                .hit_floating_window(input_window_id, group, point)
                                .ok()
                        });
                        eprintln!(
                            "[DOCK-INPUT][floating] down floating_group={:?} point=({:.1}, {:.1}) hit={:?}",
                            group, point.x, point.y, hit.unwrap_or(dock_core::HitResult::Miss)
                        );

                        // A DockTab inside an already-floating DockColumnView moves
                        // that same native Column window. It must not run the old
                        // Panel tear-off path, which would create another Floating.
                        let dock_component_group = match hit {
                            Some(dock_core::HitResult::TabHeader { group, .. })
                            | Some(dock_core::HitResult::TabStrip { group })
                            | Some(dock_core::HitResult::Group { group }) => Some(group),
                            _ => None,
                        };
                        if let Some(tab_group) = dock_component_group
                            && dock_component_group_zones_for_down
                                .get(&tab_group)
                                .is_some_and(|zone| !DockTab::leaves_strip_zone(*zone, point))
                        {
                            if let Some(manager) = manager_for_down.upgrade()
                                && manager.update(app, |manager, manager_cx| {
                                    manager.begin_existing_window_move_from_tab(
                                        group,
                                        point,
                                        manager_cx,
                                    )
                                })
                            {
                                eprintln!(
                                    "[DOCK-INPUT][floating] window_move_start source=DockTab group={:?} point=({:.1}, {:.1})",
                                    group, point.x, point.y
                                );
                                tab_press_for_down.set(None);
                                app.stop_propagation();
                            }
                            return;
                        }
                        // Give the visible floating header priority over Core hit-testing:
                        // in a one-column Floating composition its header overlaps the root
                        // TabHeaderRect, so Core can otherwise report a tab instead of the
                        // actual window drag surface.
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
                                && manager.update(app, |manager, manager_cx| {
                                    manager.begin_existing_window_move(group, point, manager_cx)
                                })
                            {
                                eprintln!(
                                    "[DOCK-INPUT][floating] window_move_start source=DockAreaHeader group={:?} point=({:.1}, {:.1})",
                                    group, point.x, point.y
                                );
                                app.stop_propagation();
                            }
                            // Also reserve the non-draggable header controls. They must
                            // never fall through and start a tab tear-off underneath.
                            return;
                        }

                        let press = controller_for_down.update(app, |controller, _| {
                            let dock_core::HitResult::TabHeader {
                                group: tab_group,
                                panel,
                                ..
                            } = controller
                                .hit_floating_window(input_window_id, group, point)
                                .ok()?
                            else {
                                return None;
                            };
                            let header = controller
                                .floating_geometry(input_window_id, group)
                                .ok()?
                                .tab_headers
                                .iter()
                                .find(|header| header.group == tab_group && header.panel == panel)?;
                            // DockTab has no close button: its complete header is draggable.
                            // Legacy TabHeaderView reserves the trailing close-button hit area.
                            if !dock_component_tab_zones_for_down.contains_key(&panel) {
                                let close_width = 18.0_f32.min(header.rect.size.width);
                                let header_right = header.rect.origin.x + header.rect.size.width;
                                if point.x >= header_right - close_width {
                                    return None;
                                }
                            }
                            Some((tab_group, panel, controller.layout().version()))
                        });
                        if let Some((tab_group, panel, layout_version)) = press {
                            tab_press_for_down.set(Some(FloatingTabPress {
                                tab_group,
                                panel,
                                origin: point,
                                layout_version,
                            }));
                            eprintln!(
                                "[DOCK-INPUT][floating] tab_press group={:?} panel={:?} point=({:.1}, {:.1})",
                                tab_group, panel, point.x, point.y
                            );
                            // Capture at press so leaving this native window before
                            // the next GPUI move event still reaches the drag pipeline.
                            window.capture_pointer(hitbox);
                        }
                    });

                    let manager_for_move = manager.clone();
                    let controller_for_move = controller.clone();
                    let tab_press_for_move = tab_press.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, app| {
                        if phase != DispatchPhase::Capture || !event.dragging() {
                            return;
                        }
                        let Some(press) = tab_press_for_move.get() else {
                            return;
                        };
                        let manager_entity = manager_for_move.upgrade();
                        if manager_entity.as_ref().is_some_and(|manager| {
                            manager
                                .update(app, |manager, _| manager.native_move_active())
                                
                        }) {
                            return;
                        }
                        let Ok(point) = crate::geometry::point_from_gpui(event.position) else {
                            return;
                        };
                        let drag = FloatingTabDrag {
                            source_window_id: input_window_id,
                            source_group: group,
                            panel: press.panel,
                            press_origin: press.origin,
                            layout_version: press.layout_version,
                        };
                        let was_dragging = controller_for_move.update(app, |controller, _| {
                            matches!(
                                controller.drag().session().state(),
                                dock_core::DragState::Dragging {
                                    payload: dock_core::DragPayload::Panel { panel },
                                    ..
                                } if panel == press.panel
                            )
                        });
                        let dragging = update_floating_tab_drag(
                            &controller_for_move,
                            drag,
                            point,
                            app,
                        );
                        if !dragging {
                            return;
                        }
                        if !was_dragging {
                            let hit = controller_for_move.update(app, |controller, _| {
                                controller
                                    .hit_floating_window(input_window_id, group, point)
                                    .unwrap_or(dock_core::HitResult::Miss)
                            });
                            eprintln!(
                                "[DOCK-INPUT][floating] drag_start group={:?} panel={:?} origin=({:.1}, {:.1}) pointer=({:.1}, {:.1}) hit={:?}",
                                group, press.panel, press.origin.x, press.origin.y, point.x, point.y, hit
                            );
                        }

                        // Capture only after the Core threshold is crossed. A
                        // normal click therefore continues to reach tab controls.
                        window.capture_pointer(hitbox);
                        if window.captured_hitbox() != Some(hitbox) {
                            tab_press_for_move.set(None);
                            let _ = controller_for_move.update(app, |controller, _| {
                                controller.cancel_pointer_interaction()
                            });
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
                        if (leaves_source || over_other_window)
                            && let Some(manager) = manager_entity.as_ref()
                            && begin_floating_tab_tearoff(
                                &controller_for_move,
                                manager,
                                drag,
                                point,
                                app,
                            )
                        {
                            tab_press_for_move.set(None);
                            window.release_pointer();
                            app.stop_propagation();
                            return;
                        }
                        app.stop_propagation();
                    });

                    let manager_for_up = manager.clone();
                    let controller_for_up = controller.clone();
                    let tab_press_for_up = tab_press.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, app| {
                        if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                            return;
                        }
                        if let Some(manager) = manager_for_up.upgrade()
                            && manager
                                .update(app, |manager, _| manager.native_move_active())
                                
                        {
                            return;
                        }
                        let Some(press) = tab_press_for_up.get() else {
                            return;
                        };
                        tab_press_for_up.set(None);
                        let Ok(point) = crate::geometry::point_from_gpui(event.position) else {
                            window.release_pointer();
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
                        let was_dragging = controller_for_up.update(app, |controller, _| {
                            matches!(
                                controller.drag().session().state(),
                                dock_core::DragState::Dragging {
                                    payload: dock_core::DragPayload::Panel { panel },
                                    ..
                                } if panel == press.panel
                            )
                        });
                        let dragging = update_floating_tab_drag(
                            &controller_for_up,
                            drag,
                            point,
                            app,
                        );
                        if dragging && !was_dragging {
                            let hit = controller_for_up.update(app, |controller, _| {
                                controller
                                    .hit_floating_window(input_window_id, group, point)
                                    .unwrap_or(dock_core::HitResult::Miss)
                            });
                            eprintln!(
                                "[DOCK-INPUT][floating] drag_start_on_release group={:?} panel={:?} origin=({:.1}, {:.1}) pointer=({:.1}, {:.1}) hit={:?}",
                                group, press.panel, press.origin.x, press.origin.y, point.x, point.y, hit
                            );
                        }
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
                            if (leaves_source || over_other_window)
                                && let Some(manager) = manager_entity.as_ref()
                                && begin_floating_tab_tearoff(
                                    &controller_for_up,
                                    manager,
                                    drag,
                                    point,
                                    app,
                                )
                            {
                                window.release_pointer();
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
                            window.release_pointer();
                            app.stop_propagation();
                            return;
                        }

                        window.release_pointer();
                        if was_pressed {
                            let _ = controller_for_up.update(app, |controller, _| {
                                controller.cancel_pointer_interaction()
                            });
                        }
                        let should_activate = controller_for_up.update(app, |controller, _| {
                            if controller.layout().version() != press.layout_version {
                                return false;
                            }
                            let dock_core::HitResult::TabHeader {
                                group: released_group,
                                panel: released_panel,
                                ..
                            } = controller
                                .hit_floating_window(input_window_id, group, point)
                                .ok()
                                .unwrap_or(dock_core::HitResult::Miss)
                            else {
                                return false;
                            };
                            if released_group != press.tab_group || released_panel != press.panel {
                                return false;
                            }
                            let Some(header) = controller
                                .floating_geometry(input_window_id, group)
                                .ok()
                                .and_then(|rects| {
                                    rects
                                        .tab_headers
                                        .iter()
                                        .find(|header| {
                                            header.group == released_group
                                                && header.panel == released_panel
                                        })
                                        .copied()
                                })
                            else {
                                return false;
                            };
                            let close_width = 18.0_f32.min(header.rect.size.width);
                            point.x < header.rect.origin.x + header.rect.size.width - close_width
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
            .filter_map(|panel_rect| {
                let panel_rect = *panel_rect;
                self.controller.update(cx, |controller, panel_cx| {
                    controller
                        .panel_view(panel_rect.panel, window, panel_cx)
                        .map(|view| (panel_rect, view))
                })
            })
            .collect::<Vec<_>>();

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

        if let Some(preview) = snapshot.drag_preview {
            let floating_rects = [snapshot.floating];
            root = root.child(DropOverlay::render(
                snapshot.active_drop_zone,
                Some(preview),
                &snapshot.rects.groups,
                &snapshot.rects.tab_headers,
                &snapshot.rects.tab_strips,
                &floating_rects,
                self.theme,
            ));
            if snapshot.drag_handle != Some(DragHandle::FloatingCapture)
                && snapshot.drag_handle != Some(DragHandle::FloatingTitle)
            {
                root = root.child(DragGhost::render(
                    Some(preview),
                    &snapshot.group_presentations,
                    snapshot.rects.viewport,
                    self.theme,
                ));
            }
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
        let context_menu_is_open = self.open_context_menu.is_some();
        root = root.capture_key_down(move |event, window, app| {
            if context_menu_is_open
                || !event.keystroke.key.eq_ignore_ascii_case("escape")
                || tab_press_for_escape.get().is_none()
            {
                return;
            }
            if let Some(manager) = manager_for_escape.upgrade() {
                if manager.update(app, |manager, _| manager.native_move_active()) {
                    return;
                }
            }
            tab_press_for_escape.set(None);
            controller_for_escape.update(app, |controller, controller_cx| {
                if let Err(error) = controller.cancel_pointer_interaction_on_escape() {
                    tracing::debug!(%error, "could not cancel floating tab drag with Escape");
                }
                controller_cx.notify();
            });
            window.release_pointer();
            app.stop_propagation();
        });

        root
    }
}

#[cfg(test)]
mod floating_dock_area_header_hit_tests {
    use dock_core::{ColumnRect, DockMetrics, NodeId, Point, Rect, Size};
    use slotmap::Key;

    #[test]
    fn column_header_takes_priority_over_its_overlapping_tab_header() {
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

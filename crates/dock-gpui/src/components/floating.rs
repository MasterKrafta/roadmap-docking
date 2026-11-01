use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use dock_core::{
    ColumnDisplayItem, ColumnMode, ColumnRect, ComputeRectsError, DockCommand, DockMetrics,
    DragPreview, DropZoneRect, FloatingGroupId, FloatingGroupRect, FlyoutState, GroupRect,
    Layout, LayoutRects, Node, NodeId, PanelId, PanelRect, Point, SplitterRect, TabHeaderRect,
    TabOverflowRect, TabStripRect, VerticalOverflow,
};
use gpui::{
    AnyView, Context, DispatchPhase, Entity, HitboxBehavior, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Render, ScrollHandle,
    StatefulInteractiveElement, Styled, WeakEntity, Window, WindowId, canvas, div,
    prelude::InteractiveElement as _, px, rgb,
};
use ui::Theme;

use crate::{
    DockController, FloatingWindowManager,
    actions::dispatch_command,
    input::drag::{
        begin_floating_tab_tearoff, finish_floating_tab_drag, update_floating_tab_drag,
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
pub struct FloatingWindowView;

impl FloatingWindowView {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
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
        title: &str,
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
            ));
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
            .filter_map(|(mut panel, view)| {
                if column_active_panels.contains(&panel.panel) {
                    return None;
                }

                // A standalone Panel has no Tabs header to reserve the title
                // band. Keep its content below the floating title bar.
                let title_bottom = origin.y + metrics.header_height.min(floating.rect.size.height);
                if panel.rect.origin.y < title_bottom {
                    let clipped = (title_bottom - panel.rect.origin.y).min(panel.rect.size.height);
                    panel.rect.origin.y += clipped;
                    panel.rect.size.height -= clipped;
                    if panel.rect.size.height <= 0.0 {
                        return None;
                    }
                }

                Some((panel, view))
            })
            .collect();
        root = root.child(PanelHost::render(loose_panels, origin));

        // For expanded roots this overlays the first group's existing header;
        // Core reserves the same band for a collapsed composite root. The
        // active tab caption therefore stays in a single, predictable title row.
        let title_height = metrics.header_height.min(floating.rect.size.height);
        let close_size = metrics.group_control_button_width.min(title_height);
        let title_grip = div()
            .id(format!("floating-title-{:?}", floating.group))
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .px(px(8.0))
            .overflow_hidden()
            .cursor_move()
            .text_color(rgb(theme.colors.primary_text))
            .text_size(px(theme.typography.tab_size))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(title.to_owned()),
            );

        let close_button = div()
            .id(format!("floating-close-{:?}", floating.group))
            .w(px(close_size.max(1.0)))
            .h(px(close_size.max(1.0)))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.icon_button_background))
            .border_1()
            .border_color(rgb(theme.colors.icon_button_border))
            .text_color(rgb(theme.colors.primary_text))
            .text_size(px(18.0))
            .child("×")
            .on_click({
                let controller = controller.clone();
                move |_, _, app| {
                    dispatch_command(
                        &controller,
                        DockCommand::CloseFloatingGroup {
                            group: floating.group,
                        },
                        app,
                    );
                }
            });

        let title_bar = div()
            .id(format!("floating-title-bar-{:?}", floating.group))
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(px(title_height))
            .flex()
            .items_center()
            .gap(px(theme.metrics.button_gap))
            .pr(px(4.0))
            .bg(rgb(theme.colors.flyout_header_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .child(title_grip)
            .child(close_button);
        root = root.child(title_bar);


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
    title: String,
    metrics: DockMetrics,
    drag_preview: Option<DragPreview>,
    drag_zones: Vec<DropZoneRect>,
    active_drop_zone: Option<dock_core::DropZoneRect>,
}

/// Resolves the display title from the first active tab in tree order.
fn floating_root_title(
    layout: &Layout,
    root: NodeId,
    presentations: &HashMap<NodeId, GroupPresentation>,
    registry: &crate::PanelRegistry,
) -> Option<String> {
    match layout.node(root)? {
        Node::Split(split) => split
            .children
            .iter()
            .find_map(|child| floating_root_title(layout, *child, presentations, registry)),
        Node::Column(column) => column
            .groups
            .iter()
            .find_map(|group| floating_root_title(layout, *group, presentations, registry)),
        Node::Tabs(_) => {
            let presentation = presentations.get(&root)?;
            let active_panel = presentation.active_panel?;
            presentation
                .tabs
                .iter()
                .find(|tab| tab.panel == active_panel)
                .map(|tab| tab.title.to_string())
        }
        Node::Panel(panel) => {
            let instance = layout.panel_instance(panel.panel_id)?;
            registry
                .get(&instance.key)
                .map(|descriptor| descriptor.title().to_string())
                .or_else(|| Some(instance.key.to_string()))
        }
    }
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
        collect_group_presentations(layout, controller.registry(), &node_ids);
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
    let title = floating_root_title(
        layout,
        area.root,
        &group_presentations,
        controller.registry(),
    )
    .unwrap_or_else(|| "Floating panels".to_string());
    let drag_preview = controller.floating_window_drag_preview(window_id, group);
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
        title,
        metrics,
        drag_preview,
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
    group: FloatingGroupId,
    theme: Theme,
    column_scroll_handles: HashMap<NodeId, ScrollHandle>,
    open_context_menu: Option<FloatingHostOpenMenu>,
    tab_press: Rc<Cell<Option<FloatingTabPress>>>,
    controller_observed: bool,
    middle_click_closes_tabs: bool,
}

impl FloatingHost {
    /// Creates a root view for one existing floating group in the shared Layout.
    pub fn new(
        controller: Entity<DockController>,
        group: FloatingGroupId,
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
            middle_click_closes_tabs: true,
        }
    }

    /// Returns the Core floating-group ID rendered by this root view.
    pub fn group(&self) -> FloatingGroupId {
        self.group
    }

    /// Rebinds this hidden host when its native window is reused for a new group.
    pub(crate) fn set_group(&mut self, group: FloatingGroupId) {
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
        if !self.controller_observed {
            cx.observe(&self.controller, |_, _, host_cx| host_cx.notify())
                .detach();
            self.controller_observed = true;
        }

        // Reuse the controller's persistent PanelId -> AnyView cache. This call
        // is idempotent; factories run only for cache misses.
        self.controller.update(cx, |controller, panel_cx| {
            controller.ensure_panel_views(window, panel_cx);
        });

        let window_id = window.window_handle().window_id();
        let snapshot = self.controller.update(cx, |controller, _| {
            floating_host_snapshot(controller, window_id, self.group)
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

        let mut root = div()
            .id(format!("floating-host-root-{:?}", self.group))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(self.theme.colors.panel_surface_background));

        // The native window moves from a frame-driven platform poll. Only the
        // custom title grip starts that session; close controls remain clickable.
        let manager = self.manager.clone();
        let controller = self.controller.clone();
        let tab_press = self.tab_press.clone();
        let group = self.group;
        let input_window_id = window_id;
        let floating_rect = snapshot.as_ref().map(|snapshot| snapshot.floating.rect);
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
                        if matches!(
                            hit,
                            Some(dock_core::HitResult::FloatingTitle { group: hit_group })
                                if hit_group == group
                        ) {
                            let is_title_grip = controller_for_down.update(app, |controller, _| {
                                let metrics = controller.metrics();
                                let width = controller
                                    .floating_geometry(input_window_id, group)
                                    .ok()
                                    .and_then(|rects| rects.floating_groups.first())
                                    .map(|floating| floating.rect.size.width);
                                let title_height = controller
                                    .floating_geometry(input_window_id, group)
                                    .ok()
                                    .and_then(|rects| rects.floating_groups.first())
                                    .map(|floating| metrics.header_height.min(floating.rect.size.height))
                                    .unwrap_or(0.0);
                                let close_width = metrics.group_control_button_width.min(title_height);
                                width.is_some_and(|width| point.x < width - close_width)
                            });
                            if is_title_grip
                                && let Some(manager) = manager_for_down.upgrade()
                                && manager.update(app, |manager, manager_cx| {
                                    manager.begin_existing_window_move(group, point, manager_cx)
                                })
                            {
                                app.stop_propagation();
                            }
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
                            let close_width = 18.0_f32.min(header.rect.size.width);
                            let header_right = header.rect.origin.x + header.rect.size.width;
                            if point.x >= header_right - close_width {
                                return None;
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
                                .unwrap_or(false)
                        }) {
                            return;
                        }
                        let Ok(point) = crate::geometry::point_from_gpui(event.position) else {
                            return;
                        };
                        let dragging = update_floating_tab_drag(
                            &controller_for_move,
                            input_window_id,
                            group,
                            press.panel,
                            press.origin,
                            press.layout_version,
                            point,
                            app,
                        );
                        if !dragging {
                            return;
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

                        let leaves_source = floating_rect.is_some_and(|rect| {
                            rect.contains(point).map(|contains| !contains).unwrap_or(true)
                        });
                        let over_other_window = manager_entity.as_ref().is_some_and(|manager| {
                            manager
                                .update(app, |manager, _| {
                                    manager.cursor_over_other_docking_window(group)
                                })
                                .unwrap_or(false)
                        });
                        if leaves_source || over_other_window {
                            if let Some(manager) = manager_entity.as_ref()
                                && begin_floating_tab_tearoff(
                                    &controller_for_move,
                                    manager,
                                    input_window_id,
                                    group,
                                    press.panel,
                                    press.origin,
                                    point,
                                    app,
                                )
                            {
                                tab_press_for_move.set(None);
                                window.release_pointer();
                                app.stop_propagation();
                                return;
                            }
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
                                .unwrap_or(false)
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

        root = root.child(FloatingWindowView::render(
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
            &snapshot.title,
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
            root = root.child(DragGhost::render(
                Some(preview),
                &snapshot.group_presentations,
                snapshot.rects.viewport,
                self.theme,
            ));
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

        root
    }
}

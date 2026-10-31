use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use dock_core::{
    ColumnDisplayItem, ColumnMode, ComputeRectsError, DockCommand, DockSide, DragHandle,
    DropTarget, GeometryError, HitResult, Layout, LayoutRects, Node, NodeId, PanelId, Point,
};
use gpui::{
    AnyView, App, Context, Entity, FocusHandle, HitboxBehavior, HitboxId, IntoElement,
    ParentElement, Render, ScrollHandle,
    Styled, Window, canvas, div, px, rgb,
    prelude::InteractiveElement as _,
};

use crate::{
    geometry::{point_from_gpui, host_geometry_from_gpui_bounds},
    DockController, DockTheme,
    components::{
        CanvasSlot, ContextMenu, ContextMenuEntry, ContextMenuItem, ContextMenuTarget,
        ContextMenuToggle, DragGhost, DropOverlay, FloatingWindowView, FlyoutView, MainDockView,
        collect_group_presentations,
    },
    input::{
        floating_bounds, map_hit_test_point, register_pointer_events, resize_preview_for_interaction,
        PointerPress, ResizePreview, ResizePreviewKind, scroll_adjusted_hit_test_point,
    },
};

/// Root composition for an application's docking workspace.
///
/// The application's main menu remains outside this view. The toolbar,
/// Options bar, and document canvas are supplied by the application as GPUI
/// views; their visibility follows the current Core chrome mode.
///
/// Layer order is deliberate: CanvasSlot, main dock surfaces, flyout, floating
/// groups in ascending Core z-order, drag feedback, then the universal ContextMenu.
pub struct DockHost {
    controller: Entity<DockController>,
    theme: DockTheme,
    toolbar: Option<AnyView>,
    options_bar: Option<AnyView>,
    canvas: Option<AnyView>,
    /// GPUI scroll positions are transient and keyed by the Core owner NodeId.
    column_scroll_handles: HashMap<NodeId, ScrollHandle>,
    /// Keeps the actual captured hitbox identity across DockHost rerenders.
    pointer_capture: Rc<Cell<Option<HitboxId>>>,
    /// Semantic click candidate and layout revision retained until pointer-up.
    pointer_press: Rc<Cell<Option<PointerPress>>>,
    /// One transient menu can be open at a time; it never enters Core layout state.
    open_context_menu: Option<OpenContextMenu>,
    /// Whether middle-clicking a tab closes its panel instance.
    middle_click_closes_tabs: bool,
    /// Keeps this host reactive to controller-owned layout and drag-state changes.
    controller_observed: bool,
    /// Root focus boundary used to cancel gestures when this window loses focus.
    root_focus_handle: Option<FocusHandle>,
    focus_observed: bool,
}

#[derive(Clone)]
struct OpenContextMenu {
    target: ContextMenuTarget,
    anchor: Point,
    open_submenu: Option<usize>,
}

impl DockHost {
    /// Starts a host around the application's single DockController entity.
    pub fn new(controller: Entity<DockController>, theme: DockTheme) -> Self {
        Self {
            controller,
            theme,
            toolbar: None,
            options_bar: None,
            canvas: None,
            column_scroll_handles: HashMap::new(),
            pointer_capture: Rc::new(Cell::new(None)),
            pointer_press: Rc::new(Cell::new(None)),
            open_context_menu: None,
            middle_click_closes_tabs: true,
            controller_observed: false,
            root_focus_handle: None,
            focus_observed: false,
        }
    }

    /// Supplies the application's toolbar slot.
    ///
    /// The slot is rendered only when Core reports
    /// `ChromeMode::toolbar_visible()`.
    pub fn toolbar(mut self, view: impl Into<AnyView>) -> Self {
        self.toolbar = Some(view.into());
        self
    }

    /// Supplies the application's Options bar slot.
    ///
    /// The slot is rendered only when Core reports
    /// `ChromeMode::options_bar_visible()`.
    pub fn options_bar(mut self, view: impl Into<AnyView>) -> Self {
        self.options_bar = Some(view.into());
        self
    }

    /// Supplies the user-owned document canvas slot.
    ///
    /// When omitted, the canvas slot remains empty and paints no adapter background.
    pub fn canvas(mut self, view: impl Into<AnyView>) -> Self {
        self.canvas = Some(view.into());
        self
    }

    /// Enables or disables closing the clicked panel when a tab receives a middle click.
    ///
    /// Enabled by default, following the common desktop-tab interaction convention.
    pub fn middle_click_closes_tabs(mut self, enabled: bool) -> Self {
        self.middle_click_closes_tabs = enabled;
        self
    }

    /// Returns the controller entity used by this host.
    pub fn controller(&self) -> &Entity<DockController> {
        &self.controller
    }

    /// Returns the theme currently used by the host and its base surfaces.
    pub fn theme(&self) -> DockTheme {
        self.theme
    }

    /// Maps a pointer point in a scrolled main column back to Core geometry.
    ///
    /// Call this before DockController::hit_test for points inside the given
    /// column. The layout rectangles themselves remain unchanged.
    pub fn scroll_adjusted_hit_test_point(
        &self,
        column: NodeId,
        point: Point,
    ) -> Result<Point, GeometryError> {
        let offset_y = self
            .column_scroll_handles
            .get(&column)
            .map(|handle| point_from_gpui(handle.offset()).map(|offset| offset.y))
            .transpose()?
            .unwrap_or(0.0);
        scroll_adjusted_hit_test_point(point, offset_y)
    }

    /// Hit-tests a pointer position after translating it out of a scrolled
    /// main-column viewport and back into the immutable Core coordinate space.
    pub fn hit_test(
        &self,
        point: Point,
        app: &mut App,
    ) -> Result<HitResult, ComputeRectsError> {
        let content_point = self.controller.update(app, |controller, _| {
            let floating_title_height = controller.metrics().header_height;
            let rects = controller.geometry()?;
            map_hit_test_point(
                point,
                rects,
                &self.column_scroll_handles,
                floating_title_height,
            )
            .map_err(ComputeRectsError::Geometry)
        })?;

        self.controller
            .update(app, |controller, _| controller.hit(content_point))
    }
}

impl Render for DockHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus_handle = self
            .root_focus_handle
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        if !self.focus_observed {
            let controller = self.controller.clone();
            let pointer_capture = self.pointer_capture.clone();
            let pointer_press = self.pointer_press.clone();
            cx.on_focus_out(&focus_handle, window, move |_host, _event, window, app| {
                let _ = controller.update(app, |controller, controller_cx| {
                    let result = controller.cancel_pointer_interaction_on_focus_loss();
                    controller_cx.notify();
                    if let Err(error) = result {
                        tracing::warn!(%error, "could not cancel dock interaction on focus loss");
                    }
                });
                window.release_pointer();
                pointer_capture.set(None);
                pointer_press.set(None);
            })
            .detach();
            self.focus_observed = true;
        }

        // A model command may cancel a captured gesture from outside the
        // pointer pipeline. The controller observer schedules this render;
        // release any capture whose interaction has already terminated.
        if self.pointer_capture.get().is_some() {
            let interaction_active = self.controller.update(cx, |controller, _| {
                controller.has_active_pointer_interaction()
            });
            if !interaction_active {
                window.release_pointer();
                self.pointer_capture.set(None);
                self.pointer_press.set(None);
            }
        }

        if !self.controller_observed {
            cx.observe(&self.controller, |_, _, host_cx| host_cx.notify())
                .detach();
            self.controller_observed = true;
        }

        // One read transaction produces the geometry snapshot plus the IDs/modes
        // needed by the main-dock presentation layer. The Layout remains Core-owned.
        let (
            chrome,
            rects,
            main_node_ids,
            main_column_ids,
            main_column_groups,
            floating_node_ids,
            floating_column_ids,
            floating_column_groups,
            floating_panel_ids,
            floating_titles,
            all_column_ids,
            column_modes,
            group_presentations,
            rail_display_orders,
            active_flyout,
            drag_preview,
            drag_handle,
            drag_zones,
            active_drop_zone,
            free_area_preview_bounds,
            resize_preview,
            metrics,
        ) = self.controller.update(cx, |controller, _| {
                let rects = controller.geometry().cloned();
                let layout = controller.layout();
                let chrome = layout.chrome();
                let mut main_node_ids = HashSet::new();
                let mut main_column_ids = HashSet::new();
                let mut main_column_groups = HashMap::new();

                for side in [DockSide::Left, DockSide::Right] {
                    if let Some(area) = layout.main_area(side) {
                        collect_main_nodes(
                            layout,
                            area.root,
                            &mut main_node_ids,
                            &mut main_column_ids,
                            &mut main_column_groups,
                        );
                    }
                }

                // Collect each floating root independently. The maps contain
                // IDs only; Layout remains the single owner of every tree.
                let mut floating_node_ids = HashMap::new();
                let mut floating_column_ids = HashMap::new();
                let mut floating_column_groups = HashMap::new();
                let mut floating_panel_ids = HashMap::new();
                let mut all_node_ids = main_node_ids.clone();
                let mut all_column_ids = main_column_ids.clone();

                if let Ok(rects) = &rects {
                    for floating_rect in &rects.floating_groups {
                        let Some(floating) = layout.floating_group(floating_rect.group) else {
                            continue;
                        };
                        let Some(area) = layout.dock_area(floating.area) else {
                            continue;
                        };

                        let mut nodes = HashSet::new();
                        let mut columns = HashSet::new();
                        let mut column_groups = HashMap::new();
                        collect_main_nodes(
                            layout,
                            area.root,
                            &mut nodes,
                            &mut columns,
                            &mut column_groups,
                        );
                        let mut panels = HashSet::new();
                        collect_panel_ids(layout, area.root, &mut panels);

                        all_node_ids.extend(nodes.iter().copied());
                        all_column_ids.extend(columns.iter().copied());
                        floating_node_ids.insert(floating_rect.group, nodes);
                        floating_column_ids.insert(floating_rect.group, columns);
                        floating_column_groups.insert(floating_rect.group, column_groups);
                        floating_panel_ids.insert(floating_rect.group, panels);
                    }
                }

                let column_modes = all_column_ids
                    .iter()
                    .filter_map(|id| match layout.node(*id) {
                        Some(Node::Column(column)) => Some((*id, column.mode)),
                        _ => None,
                    })
                    .collect::<HashMap<_, _>>();
                let group_presentations =
                    collect_group_presentations(layout, controller.registry(), &all_node_ids);
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

                let floating_titles = floating_node_ids
                    .keys()
                    .filter_map(|floating_id| {
                        let floating = layout.floating_group(*floating_id)?;
                        let area = layout.dock_area(floating.area)?;
                        let title = floating_root_title(
                            layout,
                            area.root,
                            &group_presentations,
                            controller.registry(),
                        )
                        .unwrap_or_else(|| "Floating panels".to_string());
                        Some((*floating_id, title))
                    })
                    .collect::<HashMap<_, _>>();
                let active_flyout = layout.flyout();
                let drag_preview = controller.drag().session().preview();
                let drag_handle = controller.drag().session().handle();
                let resolver = controller.drag().resolver();
                let drag_zones = resolver
                    .map(|resolver| resolver.zones().to_vec())
                    .unwrap_or_default();
                let active_drop_zone = drag_preview.and_then(|preview| {
                    let zone = resolver?
                        .resolve_zone(preview.point)
                        .ok()?
                        .filter(|zone| preview.target == Some(zone.target))?;
                    Some(zone)
                });
                let free_area_preview_bounds = drag_preview
                    .filter(|preview| preview.target == Some(DropTarget::FreeArea))
                    .and_then(|preview| {
                        floating_bounds(controller, preview.payload, preview.point)
                    });
                let resize_preview = controller
                    .resize_interaction()
                    .and_then(resize_preview_for_interaction);
                let metrics = controller.metrics();

                (
                    chrome,
                    rects,
                    main_node_ids,
                    main_column_ids,
                    main_column_groups,
                    floating_node_ids,
                    floating_column_ids,
                    floating_column_groups,
                    floating_panel_ids,
                    floating_titles,
                    all_column_ids,
                    column_modes,
                    group_presentations,
                    rail_display_orders,
                    active_flyout,
                    drag_preview,
                    drag_handle,
                    drag_zones,
                    active_drop_zone,
                    free_area_preview_bounds,
                    resize_preview,
                    metrics,
                )
            });
        let rects = match rects {
            Ok(rects) => Some(rects),
            Err(error) => {
                tracing::warn!(%error, "dock host could not obtain layout geometry");
                None
            }
        };

        // Main-column panels are mounted by ColumnView so their content moves
        // with that column's own scroll handle. Floating panel views are selected
        // per floating root below and mounted inside that composition.
        let main_active_panels = main_column_groups
            .values()
            .flat_map(|groups| groups.iter())
            .filter_map(|group| {
                group_presentations
                    .get(group)
                    .and_then(|presentation| presentation.active_panel)
            })
            .collect::<HashSet<_>>();
        // Resolve/cache each active AnyView before building the presentation
        // tree. The returned elements then own their views and do not borrow
        // Window or App across sibling columns.
        let panel_views = rects
            .as_ref()
            .map(|rects| {
                rects
                    .panels
                    .iter()
                    .filter_map(|panel_rect| {
                        let panel = panel_rect.panel;
                        let rect = *panel_rect;
                        self.controller.update(cx, |controller, panel_cx| {
                            controller
                                .panel_view(panel, window, panel_cx)
                                .map(|view| (rect, view))
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let main_panel_views = panel_views
            .iter()
            .filter(|(panel, _)| main_active_panels.contains(&panel.panel))
            .cloned()
            .collect::<Vec<_>>();
        let flyout_panel_view = rects
            .as_ref()
            .and_then(|rects| rects.flyout)
            .and_then(|flyout_rect| {
                active_flyout
                    .filter(|flyout| flyout.panel == flyout_rect.panel)
                    .and_then(|flyout| {
                        self.controller.update(cx, |controller, panel_cx| {
                            controller
                                .panel_view(flyout.panel, window, panel_cx)
                                .map(|view| (flyout, view))
                        })
                    })
            });

        // Keep scroll state only while its Column node still owns an overflow
        // region. Removing/replacing the Core node or resolving the overflow
        // drops the associated GPUI handle and its transient scroll position.
        if let Some(rects) = rects.as_ref() {
            let overflow_columns = rects
                .vertical_overflow
                .iter()
                .filter(|overflow| all_column_ids.contains(&overflow.owner))
                .map(|overflow| overflow.owner)
                .collect::<HashSet<_>>();
            self.column_scroll_handles.retain(|owner, _| {
                all_column_ids.contains(owner) && overflow_columns.contains(owner)
            });
            for owner in overflow_columns {
                self.column_scroll_handles
                    .entry(owner)
                    .or_insert_with(ScrollHandle::new);
            }
        }

        if self.open_context_menu.as_ref().is_some_and(|open| {
            !context_menu_target_exists(
                open.target,
                &group_presentations,
                &all_column_ids,
                rects.as_ref(),
            )
        }) {
            self.open_context_menu = None;
        }

        let menu_host_entity = cx.entity().downgrade();
        let toggle_context_menu: ContextMenuToggle =
            Rc::new(move |target, anchor, app| {
                if let Some(host) = menu_host_entity.upgrade() {
                    let _ = host.update(app, |host, host_cx| {
                        host.open_context_menu = if host
                            .open_context_menu
                            .as_ref()
                            .is_some_and(|open| open.target == target)
                        {
                            None
                        } else {
                            Some(OpenContextMenu {
                                target,
                                anchor,
                                open_submenu: None,
                            })
                        };
                        host_cx.notify();
                    });
                }
            });

        let dismiss_host_entity = cx.entity().downgrade();
        let dismiss_context_menu: Rc<dyn Fn(&mut App)> = Rc::new(move |app| {
            if let Some(host) = dismiss_host_entity.upgrade() {
                let _ = host.update(app, |host, host_cx| {
                    host.open_context_menu = None;
                    host_cx.notify();
                });
            }
        });

        let submenu_host_entity = cx.entity().downgrade();
        let toggle_context_submenu: Rc<dyn Fn(Option<usize>, &mut App)> =
            Rc::new(move |index, app| {
                if let Some(host) = submenu_host_entity.upgrade() {
                    let _ = host.update(app, |host, host_cx| {
                        if let Some(open) = host.open_context_menu.as_mut() {
                            open.open_submenu = index;
                            host_cx.notify();
                        }
                    });
                }
            });

        let open_context_menu = self.open_context_menu.clone();
        let menu_group = open_context_menu.as_ref().and_then(|open| match open.target {
            ContextMenuTarget::Group(group) => Some(group),
            _ => None,
        });
        let context_menu_entries = open_context_menu
            .as_ref()
            .and_then(|open| {
                rects.as_ref().map(|rects| {
                    build_context_menu_entries(
                        open.target,
                        rects,
                        &group_presentations,
                        &column_modes,
                        self.controller.clone(),
                    )
                })
            })
            .unwrap_or_default();

        let middle_click_closes_tabs = self.middle_click_closes_tabs;
        let theme = self.theme;
        let context_menu_is_open = open_context_menu.is_some();
        let drag_cursor_style = drag_preview.map(|preview| {
            if preview.target.is_some() || drag_handle == Some(DragHandle::FloatingTitle) {
                gpui::CursorStyle::ClosedHand
            } else {
                gpui::CursorStyle::OperationNotAllowed
            }
        });
        let controller_for_measurement = self.controller.clone();
        let pointer_controller = self.controller.clone();
        let pointer_scroll_handles = self.column_scroll_handles.clone();
        let pointer_context_menu = toggle_context_menu.clone();
        let pointer_capture_for_measurement = self.pointer_capture.clone();
        let pointer_capture = self.pointer_capture.clone();
        let pointer_press_for_measurement = self.pointer_press.clone();
        let pointer_press_for_events = self.pointer_press.clone();
        let measurement = canvas(
            move |bounds, window, app| {
                let (viewport, window_origin) =
                    match host_geometry_from_gpui_bounds(bounds) {
                        Ok(geometry) => geometry,
                        Err(error) => {
                            tracing::debug!(%error, "dock host rejected invalid GPUI bounds");
                            return None;
                        }
                    };

                controller_for_measurement.update(app, |controller, controller_cx| {
                    match controller.set_viewport(viewport) {
                        Ok(true) => {
                            pointer_press_for_measurement.set(None);
                            if pointer_capture_for_measurement.get().is_some() {
                                window.release_pointer();
                                pointer_capture_for_measurement.set(None);
                            }
                            // geometry() performs a fresh valid calculation and updates
                            // Layout::canvas_bounds only when the measured Canvas rect changed.
                            if let Err(error) = controller.geometry() {
                                tracing::warn!(%error, "dock host geometry update failed");
                            }
                            controller_cx.notify();
                        }
                        Ok(false) => {}
                        Err(error) => {
                            tracing::warn!(%error, "dock host rejected measured viewport");
                        }
                    }
                });

                Some((
                    window.insert_hitbox(bounds, HitboxBehavior::Normal).id,
                    window_origin,
                ))
            },
            move |_, root_surface, window, _| {
                if let Some((root_hitbox, window_origin)) = root_surface {
                    register_pointer_events(
                        window,
                        root_hitbox,
                        window_origin,
                        pointer_controller.clone(),
                        pointer_scroll_handles.clone(),
                        pointer_context_menu.clone(),
                        pointer_capture.clone(),
                        pointer_press_for_events.clone(),
                    );
                }
                // Apply the drag cursor at window level so child buttons and
                // tab headers cannot override grabbing/not-allowed feedback.
                if let Some(style) = drag_cursor_style {
                    window.set_window_cursor_style(style);
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let mut root = div()
            .track_focus(&focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme.colors.window_background));

        if chrome.toolbar_visible()
            && let Some(toolbar) = self.toolbar.clone()
        {
            root = root.child(
                div()
                    .w_full()
                    .flex_none()
                    .bg(rgb(theme.colors.toolbar_background))
                    .child(toolbar),
            );
        }

        if chrome.options_bar_visible()
            && let Some(options_bar) = self.options_bar.clone()
        {
            root = root.child(
                div()
                    .w_full()
                    .flex_none()
                    .bg(rgb(theme.colors.toolbar_background))
                    .child(options_bar),
            );
        }

        let mut workspace = div()
            .relative()
            .flex_1()
            .w_full()
            .bg(rgb(theme.colors.window_background))
            .overflow_hidden()
            .child(measurement);

        // The current semantic target controls the cursor. A floating-title drag
        // intentionally has no drop target but is still a valid move gesture.
        if let Some(preview) = drag_preview {
            workspace = if preview.target.is_some() || drag_handle == Some(DragHandle::FloatingTitle) {
                workspace.cursor_grabbing()
            } else {
                workspace.cursor_not_allowed()
            };
        }

        // Escape first cancels an active pointer gesture. A flyout is dismissed
        // only when there is no drag/resize to cancel, so cancelling a flyout drag
        // leaves its source panel visible in the rail.
        {
            let controller_for_escape = self.controller.clone();
            let pointer_capture_for_escape = self.pointer_capture.clone();
            let pointer_press_for_escape = self.pointer_press.clone();
            workspace = workspace.capture_key_down(move |event, window, app| {
                if context_menu_is_open
                    || !event.keystroke.key.eq_ignore_ascii_case("escape")
                {
                    return;
                }

                let had_pointer_state = pointer_capture_for_escape.get().is_some()
                    || pointer_press_for_escape.get().is_some();
                pointer_press_for_escape.set(None);
                let (pointer_active, flyout_open) = controller_for_escape.update(app, |controller, _| {
                    (
                        controller.has_active_pointer_interaction(),
                        controller.layout().flyout().is_some(),
                    )
                });

                let cancel_pointer = pointer_active || pointer_capture_for_escape.get().is_some();
                if cancel_pointer {
                    let result = controller_for_escape.update(app, |controller, controller_cx| {
                        let result = controller.cancel_pointer_interaction_on_escape();
                        controller_cx.notify();
                        result
                    });
                    if let Err(error) = result {
                        tracing::debug!(%error, "could not cancel dock interaction with Escape");
                    }
                    window.release_pointer();
                    pointer_capture_for_escape.set(None);
                } else if flyout_open {
                    let result = controller_for_escape.update(app, |controller, controller_cx| {
                        controller.dispatch_transient_in(
                            DockCommand::DismissFlyout,
                            controller_cx,
                        )
                    });
                    if let Err(error) = result {
                        tracing::debug!(%error, "could not dismiss flyout with Escape");
                    }
                }

                if cancel_pointer || flyout_open || had_pointer_state {
                    app.stop_propagation();
                }
            });
        }

        if let Some(canvas_rect) = rects.as_ref().and_then(|rects| rects.canvas) {
            workspace = workspace.child(CanvasSlot::render(
                canvas_rect,
                self.canvas.clone(),
                free_area_preview_bounds,
                theme,
            ));
        }

        if chrome.dock_visible()
            && let Some(rects) = rects.as_ref()
        {
            let main_columns = rects
                .columns
                .iter()
                .filter(|column| main_column_ids.contains(&column.column))
                .copied()
                .collect::<Vec<_>>();
            workspace = workspace.child(MainDockView::render(
                &main_columns,
                &rects.groups,
                main_panel_views,
                &rects.tab_strips,
                &rects.tab_headers,
                &rects.tab_overflows,
                &group_presentations,
                &rail_display_orders,
                active_flyout,
                drag_preview,
                &drag_zones,
                metrics,
                menu_group,
                toggle_context_menu.clone(),
                middle_click_closes_tabs,
                &rects.splitters,
                &rects.vertical_overflow,
                &main_node_ids,
                &main_column_groups,
                &column_modes,
                &self.column_scroll_handles,
                self.controller.clone(),
                theme,
            ));

            // Flyout is an interactive overlay above the Canvas and main dock.
            if let Some(flyout) = rects.flyout {
                let title = active_flyout
                    .and_then(|state| group_presentations.get(&state.group))
                    .and_then(|presentation| {
                        presentation
                            .tabs
                            .iter()
                            .find(|tab| tab.panel == flyout.panel)
                            .map(|tab| tab.title.clone())
                    })
                    .unwrap_or_else(|| "Panel".into());
                let panel_view = flyout_panel_view
                    .as_ref()
                    .map(|(_, view)| view.clone());
                workspace = workspace.child(FlyoutView::render(
                    flyout,
                    title,
                    panel_view,
                    self.controller.clone(),
                    theme,
                ));
            }

            // Later children paint above earlier ones; Core z_order is preserved.
            let mut floating_groups = rects.floating_groups.clone();
            floating_groups.sort_by_key(|group| group.z_order);
            for floating in floating_groups {
                let node_ids = floating_node_ids
                    .get(&floating.group)
                    .cloned()
                    .unwrap_or_default();
                let column_ids = floating_column_ids
                    .get(&floating.group)
                    .cloned()
                    .unwrap_or_default();
                let empty_column_groups = HashMap::new();
                let column_group_map = floating_column_groups
                    .get(&floating.group)
                    .unwrap_or(&empty_column_groups);
                let panel_ids = floating_panel_ids
                    .get(&floating.group)
                    .cloned()
                    .unwrap_or_default();

                let floating_columns = rects
                    .columns
                    .iter()
                    .filter(|column| column_ids.contains(&column.column))
                    .copied()
                    .collect::<Vec<_>>();
                let floating_group_rects = rects
                    .groups
                    .iter()
                    .filter(|group| node_ids.contains(&group.group))
                    .copied()
                    .collect::<Vec<_>>();
                let floating_tab_strips = rects
                    .tab_strips
                    .iter()
                    .filter(|strip| node_ids.contains(&strip.group))
                    .copied()
                    .collect::<Vec<_>>();
                let floating_tab_headers = rects
                    .tab_headers
                    .iter()
                    .filter(|header| node_ids.contains(&header.group))
                    .copied()
                    .collect::<Vec<_>>();
                let floating_tab_overflows = rects
                    .tab_overflows
                    .iter()
                    .filter(|overflow| node_ids.contains(&overflow.group))
                    .cloned()
                    .collect::<Vec<_>>();
                let floating_splitters = rects
                    .splitters
                    .iter()
                    .filter(|splitter| node_ids.contains(&splitter.owner))
                    .copied()
                    .collect::<Vec<_>>();
                let floating_resize_edges = rects
                    .floating_resize_edges
                    .iter()
                    .filter(|edge| edge.group == floating.group)
                    .copied()
                    .collect::<Vec<_>>();
                let floating_overflow = rects
                    .vertical_overflow
                    .iter()
                    .filter(|overflow| column_ids.contains(&overflow.owner))
                    .copied()
                    .collect::<Vec<_>>();
                let floating_panel_views = panel_views
                    .iter()
                    .filter(|(panel, _)| panel_ids.contains(&panel.panel))
                    .cloned()
                    .collect::<Vec<_>>();

                workspace = workspace.child(FloatingWindowView::render(
                    floating,
                    &floating_resize_edges,
                    &floating_columns,
                    &floating_group_rects,
                    floating_panel_views,
                    &floating_tab_strips,
                    &floating_tab_headers,
                    &floating_tab_overflows,
                    &group_presentations,
                    column_group_map,
                    &column_modes,
                    &rail_display_orders,
                    &floating_splitters,
                    &floating_overflow,
                    &self.column_scroll_handles,
                    active_flyout,
                    drag_preview,
                    &drag_zones,
                    metrics,
                    floating_titles
                        .get(&floating.group)
                        .map(String::as_str)
                        .unwrap_or("Floating panels"),
                    menu_group,
                    toggle_context_menu.clone(),
                    middle_click_closes_tabs,
                    self.controller.clone(),
                    theme,
                ));
            }

            // One overlay consumes the same resolver snapshot used by hit testing.
            // The ghost is a visual-only sibling and has no pointer listeners.
            workspace = workspace.child(DropOverlay::render(
                active_drop_zone,
                drag_preview,
                &rects.groups,
                &rects.tab_headers,
                &rects.tab_strips,
                &rects.floating_groups,
                theme,
            ));
            workspace = workspace.child(DragGhost::render(
                drag_preview,
                &group_presentations,
                &floating_titles,
                rects.viewport,
                theme,
            ));
        }

        if let Some(mut preview) = resize_preview {
            if let Some(owner) = preview.scroll_owner {
                let scroll_offset = self
                    .column_scroll_handles
                    .get(&owner)
                    .map(|handle| handle.offset().y.as_f32().max(0.0))
                    .unwrap_or(0.0);
                preview.rect.origin.y -= scroll_offset;
            }
            workspace = workspace.child(render_resize_preview(preview, theme));
        }

        if let (Some(open), Some(viewport)) = (
            open_context_menu.as_ref(),
            rects.as_ref().and_then(|rects| rects.viewport),
        ) {
            workspace = workspace.child(ContextMenu::render(
                "dock-context-menu",
                viewport,
                open.anchor,
                &context_menu_entries,
                open.open_submenu,
                crate::components::context_menu::ContextMenuCallbacks {
                    on_submenu: toggle_context_submenu,
                    on_dismiss: dismiss_context_menu,
                },
                theme,
            ));
        }

        root.child(workspace)
    }
}

fn render_resize_preview(preview: ResizePreview, theme: DockTheme) -> impl IntoElement {
    let rect = preview.rect;
    let mut marker = div()
        .absolute()
        .left(px(rect.origin.x))
        .top(px(rect.origin.y))
        .w(px(rect.size.width))
        .h(px(rect.size.height));

    marker = match preview.kind {
        ResizePreviewKind::SplitterLine => marker
            .bg(rgb(theme.colors.drop_zone_border))
            .opacity(0.96),
        ResizePreviewKind::BoundsOutline => marker
            .bg(rgb(theme.colors.drop_zone))
            .opacity(0.14)
            .border_1()
            .border_color(rgb(theme.colors.drop_zone_border)),
    };
    marker
}

fn context_menu_target_exists(
    target: ContextMenuTarget,
    presentations: &HashMap<NodeId, crate::components::tab_strip::GroupPresentation>,
    columns: &HashSet<NodeId>,
    rects: Option<&LayoutRects>,
) -> bool {
    match target {
        ContextMenuTarget::Group(group) => presentations.contains_key(&group),
        ContextMenuTarget::Tab { group, panel } => presentations
            .get(&group)
            .is_some_and(|presentation| presentation.tabs.iter().any(|tab| tab.panel == panel)),
        ContextMenuTarget::Column(column) => columns.contains(&column),
        ContextMenuTarget::Overflow(group) => rects.is_some_and(|rects| {
            rects.tab_overflows.iter().any(|overflow| {
                overflow.group == group && !overflow.hidden_tabs.is_empty()
            })
        }),
    }
}

fn command_menu_item(
    label: impl Into<gpui::SharedString>,
    command: DockCommand,
    controller: Entity<DockController>,
) -> ContextMenuEntry {
    ContextMenuItem::new(label, move |app| {
        crate::actions::dispatch_command(&controller, command.clone(), app);
    })
    .into()
}

fn build_context_menu_entries(
    target: ContextMenuTarget,
    rects: &LayoutRects,
    presentations: &HashMap<NodeId, crate::components::tab_strip::GroupPresentation>,
    column_modes: &HashMap<NodeId, ColumnMode>,
    controller: Entity<DockController>,
) -> Vec<ContextMenuEntry> {
    match target {
        ContextMenuTarget::Group(group_id) => {
            let Some(group) = rects.groups.iter().find(|group| group.group == group_id) else {
                return Vec::new();
            };
            let Some(presentation) = presentations.get(&group_id) else {
                return Vec::new();
            };
            let mut entries = vec![command_menu_item(
                "Закрыть группу",
                DockCommand::CloseGroup { group: group_id },
                controller.clone(),
            )];
            if let Some(panel) = presentation.active_panel {
                entries.push(command_menu_item(
                    "Закрыть активную вкладку",
                    DockCommand::ClosePanel { panel },
                    controller.clone(),
                ));
            }
            entries.push(ContextMenuEntry::separator());
            entries.push(command_menu_item(
                if presentation.collapsed { "Развернуть" } else { "Свернуть" },
                DockCommand::CollapseGroup {
                    group: group_id,
                    collapsed: !presentation.collapsed,
                },
                controller.clone(),
            ));
            entries.push(ContextMenuEntry::separator());
            entries.push(command_menu_item(
                "Вынести в плавающее окно",
                DockCommand::Float {
                    node: group_id,
                    bounds: group.rect,
                },
                controller,
            ));
            entries
        }
        ContextMenuTarget::Tab { group, panel } => {
            let Some(presentation) = presentations.get(&group) else {
                return Vec::new();
            };
            let Some(tab) = presentation.tabs.iter().find(|tab| tab.panel == panel) else {
                return Vec::new();
            };
            vec![
                ContextMenuItem::new(
                    format!("Активировать «{}»", tab.title),
                    {
                        let controller = controller.clone();
                        move |app| {
                            crate::actions::dispatch_command(
                                &controller,
                                DockCommand::ActivateTab { group, panel },
                                app,
                            );
                        }
                    },
                )
                .selected(presentation.active_panel == Some(panel))
                .into(),
                ContextMenuEntry::separator(),
                command_menu_item(
                    "Закрыть вкладку",
                    DockCommand::ClosePanel { panel },
                    controller.clone(),
                ),
                command_menu_item(
                    "Закрыть группу",
                    DockCommand::CloseGroup { group },
                    controller,
                ),
            ]
        }
        ContextMenuTarget::Column(column) => {
            let Some(current_mode) = column_modes.get(&column).copied() else {
                return Vec::new();
            };
            let modes = [
                (ColumnMode::Expanded, "Развернутая"),
                (ColumnMode::IconsAndLabels, "Иконки и подписи"),
                (ColumnMode::IconsOnly, "Только иконки"),
            ];
            let submenu = modes
                .into_iter()
                .map(|(mode, label)| {
                    ContextMenuItem::new(label, {
                        let controller = controller.clone();
                        move |app| {
                            crate::actions::dispatch_command(
                                &controller,
                                DockCommand::SetColumnMode { column, mode },
                                app,
                            );
                        }
                    })
                    .selected(current_mode == mode)
                    .into()
                })
                .collect();
            vec![ContextMenuEntry::submenu("Режим колонки", None, submenu)]
        }
        ContextMenuTarget::Overflow(group) => {
            let Some(overflow) = rects.tab_overflows.iter().find(|item| item.group == group) else {
                return Vec::new();
            };
            let Some(presentation) = presentations.get(&group) else {
                return Vec::new();
            };
            let mut entries = Vec::new();
            for panel in &overflow.hidden_tabs {
                let Some(tab) = presentation.tabs.iter().find(|tab| tab.panel == *panel) else {
                    continue;
                };
                let controller = controller.clone();
                let panel = *panel;
                let mut item = ContextMenuItem::new(tab.title.clone(), move |app| {
                    crate::actions::dispatch_command(
                        &controller,
                        DockCommand::ActivateTab { group, panel },
                        app,
                    );
                })
                .selected(presentation.active_panel == Some(panel));
                if let Some(icon) = tab.icon.clone() {
                    item = item.with_icon(icon);
                }
                entries.push(item.into());
            }
            if entries.is_empty() {
                entries.push(ContextMenuItem::disabled("Нет скрытых вкладок").into());
            }
            entries
        }
    }
}

/// Collects only nodes reachable from the left/right main dock roots.
/// Floating compositions are deliberately rendered by their own later layer.
fn collect_main_nodes(
    layout: &Layout,
    id: NodeId,
    nodes: &mut HashSet<NodeId>,
    columns: &mut HashSet<NodeId>,
    column_groups: &mut HashMap<NodeId, Vec<NodeId>>,
) {
    if !nodes.insert(id) {
        return;
    }

    match layout.node(id) {
        Some(Node::Split(split)) => {
            for child in &split.children {
                collect_main_nodes(layout, *child, nodes, columns, column_groups);
            }
        }
        Some(Node::Column(column)) => {
            columns.insert(id);
            column_groups.insert(id, column.groups.clone());
            for group in &column.groups {
                collect_main_nodes(layout, *group, nodes, columns, column_groups);
            }
        }
        Some(Node::Tabs(tabs)) => {
            for child in &tabs.children {
                collect_main_nodes(layout, *child, nodes, columns, column_groups);
            }
        }
        Some(Node::Panel(_)) | None => {}
    }
}

/// Returns the first active tab title in deterministic tree order.
fn floating_root_title(
    layout: &Layout,
    root: NodeId,
    presentations: &HashMap<NodeId, crate::components::tab_strip::GroupPresentation>,
    registry: &crate::PanelRegistry,
) -> Option<String> {
    match layout.node(root)? {
        Node::Split(split) => split.children.iter().find_map(|child| {
            floating_root_title(layout, *child, presentations, registry)
        }),
        Node::Column(column) => column.groups.iter().find_map(|group| {
            floating_root_title(layout, *group, presentations, registry)
        }),
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

/// Collects panel IDs reachable from one tree root. Geometry emits only
/// rectangles for active panels, so this set can safely include inactive tabs.
fn collect_panel_ids(layout: &Layout, id: NodeId, panels: &mut HashSet<PanelId>) {
    match layout.node(id) {
        Some(Node::Split(split)) => {
            for child in &split.children {
                collect_panel_ids(layout, *child, panels);
            }
        }
        Some(Node::Column(column)) => {
            for group in &column.groups {
                collect_panel_ids(layout, *group, panels);
            }
        }
        Some(Node::Tabs(tabs)) => {
            for child in &tabs.children {
                collect_panel_ids(layout, *child, panels);
            }
        }
        Some(Node::Panel(panel)) => {
            panels.insert(panel.panel_id);
        }
        None => {}
    }
}

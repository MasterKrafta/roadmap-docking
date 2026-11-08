use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use dock_core::{
    ColumnDisplayItem, ColumnMode, ComputeRectsError, DockCommand, DockSide, DragHandle,
    DropTarget, GeometryError, HitResult, Layout, LayoutRects, Node, NodeId, Point,
};
use gpui::{
    AnyView, App, AppContext, Context, Entity, FocusHandle, HitboxBehavior, HitboxId, IntoElement,
    ParentElement, Render, ScrollHandle, Styled, Window, canvas, div,
    prelude::InteractiveElement as _, px, rgb,
};

use crate::{
    DockController, DockTheme, FloatingWindowManager,
    components::{
        CanvasSlot, ContextMenu, ContextMenuEntry, ContextMenuItem, ContextMenuTarget,
        ContextMenuToggle, DropOverlay, FlyoutView, MainDockView,
        collect_group_presentations,
    },
    geometry::{host_geometry_from_gpui_bounds, point_from_gpui},
    input::{
        PointerPress, ResizePreview, ResizePreviewKind, floating_bounds, map_hit_test_point,
        register_pointer_events, resize_preview_for_interaction, scroll_adjusted_hit_test_point,
        interaction::DockInteractionCoordinator,
    },
};

/// Root composition for an application's docking workspace.
///
/// The application's main menu remains outside this view. The toolbar,
/// Options bar, and document canvas are supplied by the application as GPUI
/// views; their visibility follows the current Core chrome mode.
///
/// Main-window layer order is deliberate: CanvasSlot, main dock surfaces,
/// flyout, main-window drop feedback, then the universal ContextMenu.
/// Floating compositions are rendered by their own FloatingHost PopUp windows;
/// native WS_THICKFRAME owns their resize interaction.
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
    floating_manager: Option<Entity<FloatingWindowManager>>,
    floating_manager_enabled: bool,
    floating_manager_attached: bool,
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
            floating_manager: None,
            floating_manager_enabled: false,
            floating_manager_attached: false,
        }
    }

    /// Enables native floating-window hosting for this DockHost.
    ///
    /// Without this opt-in, panels stay in the main window and no native panel
    /// windows are created or reconciled.
    pub fn with_floating_window_manager(
        mut self,
        manager: Entity<FloatingWindowManager>,
    ) -> Self {
        self.floating_manager = Some(manager);
        self.floating_manager_enabled = true;
        self
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
    pub fn hit_test(&self, point: Point, app: &mut App) -> Result<HitResult, ComputeRectsError> {
        let content_point = self.controller.update(app, |controller, _| {
            let rects = controller.geometry()?;
            map_hit_test_point(point, rects, &self.column_scroll_handles)
                .map_err(ComputeRectsError::Geometry)
        })?;

        self.controller
            .update(app, |controller, _| controller.hit(content_point))
    }
}

impl Render for DockHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let main_window_id = window.window_handle().window_id();
        self.controller.update(cx, |controller, _| {
            controller.set_main_window_id(main_window_id);
        });

        // Create and attach the native-window manager before focus/input listeners
        // are installed so they can distinguish a cross-window native drag.
        let floating_manager = if let Some(manager) = self.floating_manager.clone() {
            manager
        } else {
            let controller = self.controller.clone();
            let theme = self.theme;
            let manager = cx.new(|_| FloatingWindowManager::new(controller, theme));
            self.floating_manager = Some(manager.clone());
            manager
        };
        if self.floating_manager_enabled && !self.floating_manager_attached {
            floating_manager.update(cx, |manager, manager_cx| {
                manager.attach_main_window(window, manager_cx);
            });
            self.floating_manager_attached = true;
        }

        let focus_handle = self
            .root_focus_handle
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        if !self.focus_observed {
            let controller = self.controller.clone();
            let manager = floating_manager.clone();
            let pointer_capture = self.pointer_capture.clone();
            let pointer_press = self.pointer_press.clone();
            cx.on_focus_out(&focus_handle, window, move |_host, _event, window, app| {
                if manager.update(app, |manager, manager_cx| {
                    manager.should_preserve_pointer_interaction_on_focus_out(manager_cx)
                }) {
                    return;
                }
                DockInteractionCoordinator::cancel_on_focus_loss(
                    &controller,
                    window,
                    &pointer_capture,
                    app,
                );
                pointer_press.set(None);
            })
            .detach();
            self.focus_observed = true;
        }

        // A model command may cancel a captured gesture from outside the
        // pointer pipeline. The controller observer schedules this render;
        // release any capture whose interaction has already terminated.
        if DockInteractionCoordinator::has_capture(&self.pointer_capture)
            && !DockInteractionCoordinator::release_if_inactive(
                &self.controller,
                window,
                &self.pointer_capture,
                cx,
            )
        {
            self.pointer_press.set(None);
        }

        if !self.controller_observed {
            cx.observe(&self.controller, |_, _, host_cx| host_cx.notify())
                .detach();
            self.controller_observed = true;
        }

        // Materialize one persistent GPUI view for every open PanelId, including
        // inactive tabs. The factory runs only on cache misses; later host changes
        // reuse the same AnyView/Entity instead of constructing a replacement.
        self.controller.update(cx, |controller, panel_cx| {
            controller.ensure_panel_views(window, panel_cx);
        });

        // One read transaction produces only the main-window geometry and presentation data.
        let (
            chrome,
            rects,
            main_node_ids,
            main_column_ids,
            main_column_sides,
            main_column_groups,
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
            let mut main_column_sides = HashMap::new();
            let mut main_column_groups = HashMap::new();

            for side in [DockSide::Left, DockSide::Right] {
                if let Some(area) = layout.main_area(side) {
                    let mut area_node_ids = HashSet::new();
                    let mut area_column_ids = HashSet::new();
                    let mut area_column_groups = HashMap::new();
                    collect_subtree_nodes(
                        layout,
                        area.root,
                        &mut area_node_ids,
                        &mut area_column_ids,
                        &mut area_column_groups,
                    );

                    main_node_ids.extend(area_node_ids);
                    for column in &area_column_ids {
                        main_column_sides.insert(*column, side);
                    }
                    main_column_ids.extend(area_column_ids);
                    main_column_groups.extend(area_column_groups);
                }
            }

            let column_modes = main_column_ids
                .iter()
                .filter_map(|id| match layout.node(*id) {
                    Some(Node::Column(column)) => Some((*id, column.mode)),
                    _ => None,
                })
                .collect::<HashMap<_, _>>();
            let group_presentations =
                collect_group_presentations(layout, controller.registry(), &main_node_ids, rects.as_ref().ok());
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

            let active_flyout = layout.flyout();
            let drag_preview = controller.main_drag_preview();
            let drag_handle = drag_preview.and_then(|_| controller.drag().session().handle());
            let resolver = controller.main_drop_resolver();
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
                .and_then(|preview| floating_bounds(controller, preview.payload, preview.point));
            let resize_preview = controller
                .resize_interaction()
                .and_then(resize_preview_for_interaction);
            let metrics = controller.metrics();

            (
                chrome,
                rects,
                main_node_ids,
                main_column_ids,
                main_column_sides,
                main_column_groups,
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

        // Main-column panels are mounted by ColumnView. Floating panels are mounted
        // only by their matching FloatingHost native window.
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
                    .filter(|panel_rect| {
                        main_active_panels.contains(&panel_rect.panel)
                            || active_flyout.is_some_and(|flyout| flyout.panel == panel_rect.panel)
                    })
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
        // A PanelId can be mounted in one host only during this render pass.
        // This also protects against stale/overlapping geometry snapshots.
        let mut mounted_panel_ids = HashSet::new();
        let main_panel_views = panel_views
            .iter()
            .filter(|(panel, _)| {
                main_active_panels.contains(&panel.panel)
                    && !active_flyout.is_some_and(|flyout| flyout.panel == panel.panel)
                    && mounted_panel_ids.insert(panel.panel)
            })
            .cloned()
            .collect::<Vec<_>>();
        let flyout_panel_view =
            rects
                .as_ref()
                .and_then(|rects| rects.flyout)
                .and_then(|flyout_rect| {
                    let flyout = active_flyout.filter(|flyout| {
                        flyout.panel == flyout_rect.panel && mounted_panel_ids.insert(flyout.panel)
                    })?;
                    self.controller.update(cx, |controller, panel_cx| {
                        controller
                            .panel_view(flyout.panel, window, panel_cx)
                            .map(|view| (flyout, view))
                    })
                });

        // Keep scroll state only while its Column node still owns an overflow
        // region. Removing/replacing the Core node or resolving the overflow
        // drops the associated GPUI handle and its transient scroll position.
        if let Some(rects) = rects.as_ref() {
            let overflow_columns = rects
                .vertical_overflow
                .iter()
                .filter(|overflow| main_column_ids.contains(&overflow.owner))
                .map(|overflow| overflow.owner)
                .collect::<HashSet<_>>();
            self.column_scroll_handles.retain(|owner, _| {
                main_column_ids.contains(owner) && overflow_columns.contains(owner)
            });
            for owner in overflow_columns {
                self.column_scroll_handles.entry(owner).or_default();
            }
        }

        if self.open_context_menu.as_ref().is_some_and(|open| {
            !context_menu_target_exists(
                open.target,
                &group_presentations,
                &main_column_ids,
                rects.as_ref(),
            )
        }) {
            self.open_context_menu = None;
        }

        let menu_host_entity = cx.entity().downgrade();
        let toggle_context_menu: ContextMenuToggle = Rc::new(move |target, anchor, app| {
            if let Some(host) = menu_host_entity.upgrade() {
                host.update(app, |host, host_cx| {
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
                host.update(app, |host, host_cx| {
                    host.open_context_menu = None;
                    host_cx.notify();
                });
            }
        });

        let submenu_host_entity = cx.entity().downgrade();
        let toggle_context_submenu: crate::components::context_menu::ContextMenuSubmenuToggle =
            Rc::new(move |index, app| {
                if let Some(host) = submenu_host_entity.upgrade() {
                    host.update(app, |host, host_cx| {
                        if let Some(open) = host.open_context_menu.as_mut() {
                            open.open_submenu = index;
                            host_cx.notify();
                        }
                    });
                }
            });

        let open_context_menu = self.open_context_menu.clone();
        let menu_group = open_context_menu
            .as_ref()
            .and_then(|open| match open.target {
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
                        true,
                    )
                })
            })
            .unwrap_or_default();

        let middle_click_closes_tabs = self.middle_click_closes_tabs;
        let theme = self.theme;
        let context_menu_is_open = open_context_menu.is_some();
        let drag_cursor_style = drag_preview.map(|preview| {
            if preview.target.is_some() || drag_handle == Some(DragHandle::FloatingCapture) {
                gpui::CursorStyle::ClosedHand
            } else {
                gpui::CursorStyle::OperationNotAllowed
            }
        });
        let controller_for_measurement = self.controller.clone();
        let manager_for_measurement = floating_manager.clone();
        let pointer_controller = self.controller.clone();
        let pointer_scroll_handles = self.column_scroll_handles.clone();
        let pointer_context_menu = toggle_context_menu.clone();
        let pointer_capture_for_measurement = self.pointer_capture.clone();
        let pointer_capture = self.pointer_capture.clone();
        let pointer_press_for_measurement = self.pointer_press.clone();
        let pointer_press_for_events = self.pointer_press.clone();
        let floating_manager_for_pointer_events = floating_manager.clone();
        let measurement = canvas(
            move |bounds, window, app| {
                let (viewport, window_origin) = match host_geometry_from_gpui_bounds(bounds) {
                    Ok(geometry) => geometry,
                    Err(error) => {
                        tracing::debug!(%error, "dock host rejected invalid GPUI bounds");
                        return None;
                    }
                };

                manager_for_measurement.update(app, |manager, _| {
                    manager.set_main_host_origin(window_origin);
                });
                controller_for_measurement.update(app, |controller, controller_cx| {
                    match controller.set_viewport(viewport) {
                        Ok(true) => {
                            pointer_press_for_measurement.set(None);
                            if pointer_capture_for_measurement.get().is_some() {
                                DockInteractionCoordinator::release(
                                    window,
                                    &pointer_capture_for_measurement,
                                );
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
                        crate::input::PointerEventState {
                            manager: floating_manager_for_pointer_events.clone(),
                            pointer_capture: pointer_capture.clone(),
                            pointer_press: pointer_press_for_events.clone(),
                        },
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

        // A FloatingCapture drag remains a valid move while its native window
        // crosses non-actionable space between the main dock and another window.
        if let Some(preview) = drag_preview {
            workspace =
                if preview.target.is_some() || drag_handle == Some(DragHandle::FloatingCapture) {
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
            let manager_for_escape = floating_manager.clone();
            let pointer_capture_for_escape = self.pointer_capture.clone();
            let pointer_press_for_escape = self.pointer_press.clone();
            workspace = workspace.capture_key_down(move |event, window, app| {
                if context_menu_is_open || !event.keystroke.key.eq_ignore_ascii_case("escape") {
                    return;
                }
                if manager_for_escape.update(app, |manager, _| manager.native_move_active()) {
                    return;
                }

                let had_pointer_state = pointer_capture_for_escape.get().is_some()
                    || pointer_press_for_escape.get().is_some();
                pointer_press_for_escape.set(None);
                let (pointer_active, flyout_open) =
                    controller_for_escape.update(app, |controller, _| {
                        (
                            controller.has_active_pointer_interaction(),
                            controller.layout().flyout().is_some(),
                        )
                    });

                let cancel_pointer = pointer_active || pointer_capture_for_escape.get().is_some();
                if cancel_pointer {
                    DockInteractionCoordinator::cancel_on_escape(
                        &controller_for_escape,
                        window,
                        &pointer_capture_for_escape,
                        app,
                    );
                } else if flyout_open {
                    let result = controller_for_escape.update(app, |controller, controller_cx| {
                        controller.dispatch_transient_in(DockCommand::DismissFlyout, controller_cx)
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
                &main_column_sides,
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
                let panel_view = flyout_panel_view.as_ref().map(|(_, view)| view.clone());
                workspace = workspace.child(FlyoutView::render(
                    flyout,
                    title,
                    panel_view,
                    self.controller.clone(),
                    metrics,
                    theme,
                ));
            }

            // Drop feedback uses the same resolver snapshot as hit testing.
            // The cursor-following DragGhost is intentionally not rendered.
            workspace = workspace.child(DropOverlay::render(
                active_drop_zone,
                drag_preview,
                &rects.groups,
                &rects.tab_headers,
                &rects.tab_strips,
                &[],
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
        ResizePreviewKind::SplitterLine => {
            marker.bg(rgb(theme.colors.drop_zone_border)).opacity(0.96)
        }
        ResizePreviewKind::BoundsOutline => marker
            .bg(rgb(theme.colors.drop_zone))
            .opacity(0.14)
            .border_1()
            .border_color(rgb(theme.colors.drop_zone_border)),
    };
    marker
}

pub(crate) fn context_menu_target_exists(
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
            rects
                .tab_overflows
                .iter()
                .any(|overflow| overflow.group == group && !overflow.hidden_tabs.is_empty())
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

pub(crate) fn build_context_menu_entries(
    target: ContextMenuTarget,
    rects: &LayoutRects,
    presentations: &HashMap<NodeId, crate::components::tab_strip::GroupPresentation>,
    column_modes: &HashMap<NodeId, ColumnMode>,
    controller: Entity<DockController>,
    allow_float: bool,
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
                if presentation.collapsed {
                    "Развернуть"
                } else {
                    "Свернуть"
                },
                DockCommand::CollapseGroup {
                    group: group_id,
                    collapsed: !presentation.collapsed,
                },
                controller.clone(),
            ));
            if allow_float {
                entries.push(ContextMenuEntry::separator());
                entries.push(command_menu_item(
                    "Вынести в плавающее окно",
                    DockCommand::Float {
                        node: group_id,
                        bounds: group.rect,
                    },
                    controller,
                ));
            }
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
                ContextMenuItem::new(format!("Активировать «{}»", tab.title), {
                    let controller = controller.clone();
                    move |app| {
                        crate::actions::dispatch_command(
                            &controller,
                            DockCommand::ActivateTab { group, panel },
                            app,
                        );
                    }
                })
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

/// Collects node IDs reachable from one Core tree root.
/// The caller decides which window owns the resulting local presentation.
pub(crate) fn collect_subtree_nodes(
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
                collect_subtree_nodes(layout, *child, nodes, columns, column_groups);
            }
        }
        Some(Node::Column(column)) => {
            columns.insert(id);
            column_groups.insert(id, column.groups.clone());
            for group in &column.groups {
                collect_subtree_nodes(layout, *group, nodes, columns, column_groups);
            }
        }
        Some(Node::Tabs(tabs)) => {
            for child in &tabs.children {
                collect_subtree_nodes(layout, *child, nodes, columns, column_groups);
            }
        }
        Some(Node::Panel(_)) | None => {}
    }
}

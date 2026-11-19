use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
    time::Duration,
};

use dock_core::{
    ColumnDisplayItem, ColumnRect, ComputeRectsError, DockCommand, DockMetrics, FloatingGroupId,
    Node, NodeId,
    PanelId, Point, Rect, SplitterRect,
};
use gpui::{
    App, Context, DispatchPhase, Entity, FocusHandle, HitboxBehavior, HitboxId, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Render, Styled,
    WeakEntity, Window, WindowId, canvas,
};
use crate::{
    dock_components::{
        collect_group_presentations, CompositionDragArea, DockTab, GroupDragArea,
    },
    dock_host::collect_subtree_nodes,
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
        resize_preview_for_interaction,
    },
    DockController, DragSourceHost, FloatingWindowManager,
};

use super::drop_overlay::DropOverlay;

#[path = "floating_column_open_menu.rs"]
mod floating_column_open_menu;
use floating_column_open_menu::FloatingColumnOpenMenu;

#[path = "floating_tab_press.rs"]
mod floating_tab_press;
use floating_tab_press::FloatingTabPress;

#[path = "floating_group_press.rs"]
mod floating_group_press;
use floating_group_press::FloatingGroupPress;

#[path = "floating_column_snapshot.rs"]
mod floating_column_snapshot;
use floating_column_snapshot::FloatingColumnSnapshot;

#[path = "floating_column_host.rs"]
mod floating_column_host;
pub(crate) use floating_column_host::FloatingColumnHost;

#[path = "floating_window_content.rs"]
mod floating_window_content;
use floating_window_content::FloatingWindowContent;

/// Floating tab drags transfer one Panel to DragHost after leaving the source strip
/// or entering another docking window. The shared CompositionHeader moves the whole window.
fn floating_tab_drag_host_required(leaves_source: bool, over_other_window: bool) -> bool {
    leaves_source || over_other_window
}


/// Maps a pointer event from a Floating part HWND back into the composition's Core space.
fn floating_pointer_point(
    position: gpui::Point<gpui::Pixels>,
    core_origin: Point,
) -> Result<Point, dock_core::GeometryError> {
    let local = crate::geometry::DockGeometry::point_from_gpui(position)?;
    floating_local_point_to_core(local, core_origin)
}

fn floating_local_point_to_core(
    local: Point,
    core_origin: Point,
) -> Result<Point, dock_core::GeometryError> {
    let point = Point::new(local.x + core_origin.x, local.y + core_origin.y);
    point.validate()?;
    Ok(point)
}

/// Returns the physical client rectangle for one Floating Column HWND.
///
/// Core keeps splitter gaps in its logical geometry, but native windows are tiled
/// without those gaps. Every Column is shifted left by the cumulative width of
/// the Axis::X splitters that precede it; Core rects remain unchanged.
pub(crate) fn floating_column_host_rect(
    column: Rect,
    splitters: &[SplitterRect],
    splitter_thickness: f32,
) -> Rect {
    if !splitter_thickness.is_finite() || splitter_thickness <= 0.0 {
        return column;
    }

    let preceding_splitters = splitters
        .iter()
        .filter(|splitter| {
            splitter.axis == dock_core::Axis::X
                && splitter.rect.origin.x + splitter.rect.size.width * 0.5
                    < column.origin.x
        })
        .count() as f32;
    let horizontal_offset = preceding_splitters * splitter_thickness;

    Rect::new(
        Point::new(column.origin.x - horizontal_offset, column.origin.y),
        dock_core::Size::new(column.size.width, column.size.height),
    )
}

fn floating_drag_source(group: FloatingGroupId, column: NodeId) -> DragSourceHost {
    DragSourceHost::FloatingColumn { group, column }
}

/// Installs the shared pointer/capture/drag pipeline on one Floating Column HWND.
/// point_origin is the Core-space origin represented by the HWND client area.
fn render_floating_pointer_input(
    group: FloatingGroupId,
    manager: WeakEntity<FloatingWindowManager>,
    controller: Entity<DockController>,
    input_window_id: WindowId,
    floating_rect: Option<Rect>,
    dock_component_tab_zones: HashMap<PanelId, Rect>,
    dock_area_drag_area: Option<(Rect, Vec<ColumnRect>, Vec<SplitterRect>, DockMetrics, bool)>,
    hide_header: bool,
    tab_press: Rc<Cell<Option<FloatingTabPress>>>,
    group_press: Rc<Cell<Option<FloatingGroupPress>>>,
    pointer_capture: Rc<Cell<Option<HitboxId>>>,
    source_column: NodeId,
    point_origin: Point,
) -> impl IntoElement {
    let floating_column_for_resize = Some((group, source_column));
    let dock_component_tab_zones_for_down = dock_component_tab_zones.clone();
    let dock_component_tab_zones_for_move = dock_component_tab_zones.clone();
    let dock_component_tab_zones_for_up = dock_component_tab_zones;
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
                        let Ok(point) = floating_pointer_point(event.position, point_origin) else {
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
                                    input_window_id,
                                    resize_hit,
                                    point,
                                    &rects,
                                    floating_column_for_resize,
                                    app,
                                );
                                let resize_started = controller_for_down
                                    .update(app, |controller, _| {
                                        controller
                                            .resize_interaction_for_window(input_window_id)
                                            .is_some()
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
                                |(area_rect, columns, splitters, metrics, show_close_button)| {
                                    floating_dock_area_title_pointer_region(
                                        *area_rect,
                                        columns,
                                        splitters,
                                        point,
                                        *metrics,
                                        *show_close_button,
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
                                    manager.begin_existing_window_grid_move(group, source_column, point, manager_cx)
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
                                    manager.begin_single_tab_window_move(
                                        group,
                                        source_column,
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
                                floating_drag_source(group, source_column),
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
                                    manager.begin_single_tab_window_move(
                                        group,
                                        source_column,
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
                        let resize_owner = controller_for_move.update(app, |controller, _| {
                            controller.resize_interaction().map(|interaction| interaction.window_id)
                        });
                        if resize_owner.is_some_and(|owner| owner != input_window_id) {
                            app.stop_propagation();
                            return;
                        }
                        let resize_active = resize_owner == Some(input_window_id);
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
                                floating_pointer_point(event.position, point_origin)
                            else {
                                return;
                            };
                            let should_schedule = controller_for_move.update(app, |controller, _| {
                                controller.queue_resize_frame_point(input_window_id, point)
                            });
                            if should_schedule {
                                let controller = controller_for_move.clone();
                                window.on_next_frame(move |_, app| {
                                    let point = controller.update(app, |controller, _| {
                                        controller.take_resize_frame_point(input_window_id)
                                    });
                                    if let Some(point) = point {
                                        update_resize(&controller, input_window_id, point, app);
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
                                floating_pointer_point(event.position, point_origin)
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
                                            manager.cursor_over_other_docking_window_from_source(group, input_window_id, source_column)
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
                        let Ok(point) = floating_pointer_point(event.position, point_origin) else {
                            return;
                        };
                        let drag = FloatingTabDrag {
                            source_window_id: input_window_id,
                            source_group: group,
                            source_column,
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
                                    manager.cursor_over_other_docking_window_from_source(group, input_window_id, source_column)
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
                        let resize_owner = controller_for_up.update(app, |controller, _| {
                            controller.resize_interaction().map(|interaction| interaction.window_id)
                        });
                        if resize_owner.is_some_and(|owner| owner != input_window_id) {
                            app.stop_propagation();
                            return;
                        }
                        let resize_active = resize_owner == Some(input_window_id);
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
                                floating_pointer_point(event.position, point_origin)
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
                                controller.clear_resize_frame_point(input_window_id);
                            });
                            if let Some(rects) = floating_rects {
                                finish_resize_with_rects(
                                    &controller_for_up,
                                    input_window_id,
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
                                floating_pointer_point(event.position, point_origin)
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
                                            manager.cursor_over_other_docking_window_from_source(group, input_window_id, source_column)
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
                        let Ok(point) = floating_pointer_point(event.position, point_origin) else {
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
                            source_column,
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
                                    manager.cursor_over_other_docking_window_from_source(group, input_window_id, source_column)
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
                                let changes_active_tab = {
                                    let layout = controller.layout();
                                    match layout.node(press.tab_group) {
                                        Some(Node::Tabs(tabs)) => tabs
                                            .active_child()
                                            .and_then(|child| match layout.node(child) {
                                                Some(Node::Panel(value)) => Some(value.panel_id),
                                                _ => None,
                                            })
                                            != Some(press.panel),
                                        _ => false,
                                    }
                                };

                                let mut commands = vec![DockCommand::ActivateTab {
                                    group: press.tab_group,
                                    panel: press.panel,
                                }];
                                if changes_active_tab {
                                    let metrics = controller.metrics();
                                    let layout = controller.layout();
                                    if let (Some(bounds), Ok(preferred_height)) = (
                                        layout.floating_group(group).map(|floating| floating.bounds),
                                        layout.floating_preferred_height_for_tab(
                                            group,
                                            press.tab_group,
                                            press.panel,
                                            &metrics,
                                        ),
                                    ) {
                                        if bounds.size.height != preferred_height {
                                            commands.push(DockCommand::ResizeFloatingBounds {
                                                group,
                                                bounds: Rect::new(
                                                    bounds.origin,
                                                    dock_core::Size::new(
                                                        bounds.size.width,
                                                        preferred_height,
                                                    ),
                                                ),
                                            });
                                        }
                                    }
                                }

                                if let Err(error) = controller.dispatch_sequence_in(commands, cx) {
                                    tracing::debug!(
                                        %error,
                                        "could not activate and resize the floating tab"
                                    );
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

    title_input
}

fn handle_floating_escape(
    event: &KeyDownEvent,
    context_menu_is_open: bool,
    manager: &WeakEntity<FloatingWindowManager>,
    controller: &Entity<DockController>,
    tab_press: &Rc<Cell<Option<FloatingTabPress>>>,
    group_press: &Rc<Cell<Option<FloatingGroupPress>>>,
    pointer_capture: &Rc<Cell<Option<HitboxId>>>,
    window: &mut Window,
    app: &mut App,
) {
    if context_menu_is_open || !event.keystroke.key.eq_ignore_ascii_case("escape") {
        return;
    }
    let window_id = window.window_handle().window_id();
    let resize_owner = controller.update(app, |controller, _| {
        controller.resize_interaction().map(|interaction| interaction.window_id)
    });
    if resize_owner.is_some_and(|owner| owner != window_id) {
        return;
    }
    if let Some(manager) = manager.upgrade() {
        if manager.update(app, |manager, _| manager.native_move_active()) {
            return;
        }
        let drag_host_manager = manager.update(app, |manager, _| manager.drag_host_manager());
        if drag_host_manager.is_some_and(|drag_host| {
            drag_host.update(app, |manager, manager_cx| manager.cancel_active(manager_cx))
        }) {
            tab_press.set(None);
            group_press.set(None);
            if DockInteractionCoordinator::has_capture(pointer_capture) {
                DockInteractionCoordinator::release(window, pointer_capture);
            }
            app.stop_propagation();
            return;
        }
    }
    if tab_press.get().is_none() && group_press.get().is_none() && resize_owner.is_none() {
        return;
    }
    tab_press.set(None);
    group_press.set(None);
    DockInteractionCoordinator::cancel_on_escape(controller, window, pointer_capture, app);
    app.stop_propagation();
}
fn observe_floating_focus_loss<T: 'static + Render>(
    cx: &mut Context<T>,
    focus_handle: &FocusHandle,
    window: &mut Window,
    controller: Entity<DockController>,
    manager: WeakEntity<FloatingWindowManager>,
    tab_press: Rc<Cell<Option<FloatingTabPress>>>,
    group_press: Rc<Cell<Option<FloatingGroupPress>>>,
    pointer_capture: Rc<Cell<Option<HitboxId>>>,
) {
    cx.on_focus_out(focus_handle, window, move |_host, _event, window, _app| {
        // on_focus_out is FnMut: clone the captured state for this event before
        // scheduling the one-shot callback on the next frame.
        let controller = controller.clone();
        let manager = manager.clone();
        let tab_press = tab_press.clone();
        let group_press = group_press.clone();
        let pointer_capture = pointer_capture.clone();

        // Focus can change while FloatingWindowManager is hiding or
        // reconciling native windows. Defer its state query until the
        // current window/manager update has unwound.
        let window_handle = window.window_handle();
        window.on_next_frame(move |_, app| {
            app.spawn(async move |app| {
                // A next-frame callback can still run while a native-window
                // reconciliation is unwinding. Do not re-enter FloatingWindowManager.
                app.background_executor()
                    .timer(Duration::from_millis(1))
                    .await;
                let preserve_interaction = manager.upgrade().is_some_and(|manager| {
                    manager.update(app, |manager, manager_cx| {
                        manager.should_preserve_pointer_interaction_on_focus_out(manager_cx)
                    })
                });
                if preserve_interaction {
                    return;
                }
                let _ = window_handle.update(app, |_, window, app| {
                    DockInteractionCoordinator::cancel_on_focus_loss(
                        &controller,
                        window,
                        &pointer_capture,
                        app,
                    );
                });
                tab_press.set(None);
                group_press.set(None);
            })
            .detach();
        });
    })
    .detach();
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
    splitters: &[SplitterRect],
    point: Point,
    metrics: DockMetrics,
    show_close_button: bool,
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
        splitters,
        point,
        header_height,
        metrics.column_control_button_width,
        show_close_button,
    ))
}

fn floating_dock_component_tab_zones(snapshot: &FloatingColumnSnapshot) -> HashMap<PanelId, Rect> {
    let mut zones = HashMap::new();
    for group_ids in snapshot.column_groups.values() {
        for group_id in group_ids {
            let Some(group) = snapshot
                .rects
                .groups
                .iter()
                .find(|group| group.group == *group_id)
            else {
                continue;
            };
            let Some(presentation) = snapshot.group_presentations.get(group_id) else {
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
    }
    zones
}

fn floating_column_snapshot(
    controller: &mut DockController,
    window_id: WindowId,
    group: FloatingGroupId,
) -> Result<Option<FloatingColumnSnapshot>, ComputeRectsError> {
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
        .resize_interaction_for_window(window_id)
        .and_then(|interaction| resize_preview_for_interaction(interaction, window_id));

    Ok(Some(FloatingColumnSnapshot {
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


fn floating_layout_contains_node(
    layout: &dock_core::Layout,
    root: NodeId,
    target: NodeId,
) -> bool {
    if root == target {
        return true;
    }
    match layout.node(root) {
        Some(Node::Split(split)) => split
            .children
            .iter()
            .any(|child| floating_layout_contains_node(layout, *child, target)),
        Some(Node::Column(column)) => column
            .groups
            .iter()
            .any(|group| floating_layout_contains_node(layout, *group, target)),
        Some(Node::Tabs(tabs)) => tabs
            .children
            .iter()
            .any(|child| floating_layout_contains_node(layout, *child, target)),
        Some(Node::Panel(_)) | None => false,
    }
}

pub(crate) fn floating_group_for_column(
    layout: &dock_core::Layout,
    column: NodeId,
) -> Option<FloatingGroupId> {
    if !matches!(layout.node(column), Some(Node::Column(_))) {
        return None;
    }

    layout.floating_groups().find_map(|(group, floating)| {
        let area = layout.dock_area(floating.area)?;
        floating_layout_contains_node(layout, area.root, column).then_some(group)
    })
}

pub(crate) fn floating_group_column_for_panel(
    layout: &dock_core::Layout,
    panel: PanelId,
) -> Option<(FloatingGroupId, NodeId, NodeId)> {
    let group = layout.nodes().find_map(|(group, node)| {
        let Node::Tabs(tabs) = node else {
            return None;
        };
        tabs.children
            .iter()
            .any(|child| {
                matches!(
                    layout.node(*child),
                    Some(Node::Panel(value)) if value.panel_id == panel
                )
            })
            .then_some(group)
    })?;
    let (floating_group, column) = floating_group_and_column_for_group(layout, group)?;
    Some((floating_group, column, group))
}

pub(crate) fn floating_group_and_column_for_group(
    layout: &dock_core::Layout,
    group: NodeId,
) -> Option<(FloatingGroupId, NodeId)> {
    let column = layout.nodes().find_map(|(column_id, node)| {
        matches!(node, Node::Column(column) if column.groups.contains(&group))
            .then_some(column_id)
    })?;
    let floating_group = floating_group_for_column(layout, column)?;
    Some((floating_group, column))
}

pub(crate) fn floating_column_local_rect(rect: Rect, origin: Point) -> Rect {
    Rect::new(
        Point::new(rect.origin.x - origin.x, rect.origin.y - origin.y),
        rect.size,
    )
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
mod floating_tab_drag_zone_tests {
    use std::collections::{HashMap, HashSet};

    use dock_core::{
        ColumnRect, DockMetrics, FloatingGroupId, FloatingGroupRect, GroupRect, LayoutRects,
        NodeId, PanelId, Point, Rect, Size,
    };
    use slotmap::SlotMap;

    use crate::dock_components::{GroupPresentation, TabPresentation};

    use super::{FloatingColumnSnapshot, floating_dock_component_tab_zones};

    #[test]
    fn every_single_tab_group_in_a_multi_group_column_gets_its_own_drag_zone() {
        let mut node_ids = SlotMap::<NodeId, ()>::with_key();
        let column = node_ids.insert(());
        let first_group = node_ids.insert(());
        let second_group = node_ids.insert(());
        let mut panel_ids = SlotMap::<PanelId, ()>::with_key();
        let first_panel = panel_ids.insert(());
        let second_panel = panel_ids.insert(());
        let mut floating_ids = SlotMap::<FloatingGroupId, ()>::with_key();
        let floating_group = floating_ids.insert(());

        let first_group_rect = Rect::new(Point::new(0.0, 40.0), Size::new(420.0, 220.0));
        let second_group_rect = Rect::new(Point::new(0.0, 270.0), Size::new(420.0, 220.0));
        let metrics = DockMetrics::default();
        let snapshot = FloatingColumnSnapshot {
            rects: LayoutRects {
                columns: vec![ColumnRect::new(
                    column,
                    Rect::new(Point::ZERO, Size::new(420.0, 520.0)),
                )],
                groups: vec![
                    GroupRect::new(first_group, first_group_rect),
                    GroupRect::new(second_group, second_group_rect),
                ],
                ..LayoutRects::default()
            },
            floating: FloatingGroupRect::new(
                floating_group,
                Rect::new(Point::ZERO, Size::new(420.0, 520.0)),
                0,
            ),
            column_ids: HashSet::from([column]),
            column_groups: HashMap::from([(
                column,
                vec![first_group, second_group],
            )]),
            column_modes: HashMap::new(),
            group_presentations: HashMap::from([
                (
                    first_group,
                    GroupPresentation {
                        active_panel: Some(first_panel),
                        tabs: vec![TabPresentation {
                            panel: first_panel,
                            title: "First".into(),
                            icon: None,
                            index: 0,
                        }],
                        ..GroupPresentation::default()
                    },
                ),
                (
                    second_group,
                    GroupPresentation {
                        active_panel: Some(second_panel),
                        tabs: vec![TabPresentation {
                            panel: second_panel,
                            title: "Second".into(),
                            icon: None,
                            index: 0,
                        }],
                        ..GroupPresentation::default()
                    },
                ),
            ]),
            rail_display_orders: HashMap::new(),
            metrics,
            drag_preview: None,
            drag_zones: Vec::new(),
            active_drop_zone: None,
            resize_preview: None,
        };

        let zones = floating_dock_component_tab_zones(&snapshot);

        assert_eq!(zones.len(), 2);
        assert_eq!(
            zones.get(&first_panel),
            Some(&DockTab::drag_zone(first_group_rect, metrics)),
        );
        assert_eq!(
            zones.get(&second_panel),
            Some(&DockTab::drag_zone(second_group_rect, metrics)),
        );
    }
}

#[cfg(test)]
mod floating_dock_area_header_hit_tests {
    use dock_core::{Axis, ColumnRect, DockMetrics, NodeId, Point, Rect, Size, SplitterRect};
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
                &[],
                Point::new(100.0, 30.0),
                metrics,
                true,
            ),
            Some(true),
        );

        let rightmost_collapse_button_center = column.origin.x
            + super::CompositionDragArea::collapse_button_left(
                column,
                columns[0],
                metrics.column_control_button_width,
                true,
            )
            + metrics.column_control_button_width * 0.5;
        assert_eq!(
            super::floating_dock_area_title_pointer_region(
                column,
                &columns,
                &[],
                Point::new(rightmost_collapse_button_center, 30.0),
                metrics,
                true,
            ),
            Some(false),
        );

        // The close control owns the far-right button only on the rightmost Header.
        assert_eq!(
            super::floating_dock_area_title_pointer_region(
                column,
                &columns,
                &[],
                Point::new(240.0, 30.0),
                metrics,
                true,
            ),
            Some(false),
        );

        // A Header without X uses the outermost button slot for its own collapse control.
        let regular_collapse_button_center = column.origin.x
            + super::CompositionDragArea::collapse_button_left(
                column,
                columns[0],
                metrics.column_control_button_width,
                false,
            )
            + metrics.column_control_button_width * 0.5;
        assert_eq!(
            super::floating_dock_area_title_pointer_region(
                column,
                &columns,
                &[],
                Point::new(regular_collapse_button_center, 30.0),
                metrics,
                false,
            ),
            Some(false),
        );

        assert_eq!(
            super::floating_dock_area_title_pointer_region(
                column,
                &columns,
                &[],
                Point::new(100.0, 60.0),
                metrics,
                true,
            ),
            None,
        );

        let splitters = [SplitterRect::new(
            NodeId::null(),
            0,
            Axis::X,
            Rect::new(Point::new(246.0, 20.0), Size::new(8.0, 32.0)),
        )];
        assert_eq!(
            super::floating_dock_area_title_pointer_region(
                Rect::new(Point::new(10.0, 20.0), Size::new(500.0, 500.0)),
                &[
                    ColumnRect::new(
                        NodeId::null(),
                        Rect::new(Point::new(10.0, 20.0), Size::new(236.0, 500.0)),
                    ),
                    ColumnRect::new(
                        NodeId::null(),
                        Rect::new(Point::new(250.0, 20.0), Size::new(260.0, 500.0)),
                    ),
                ],
                &splitters,
                Point::new(250.0, 30.0),
                metrics,
                true,
            ),
            Some(false),
        );
    }
}

#[cfg(test)]
mod floating_column_local_geometry_tests {
    use super::{floating_local_point_to_core, floating_column_local_rect};
    use dock_core::{Point, Rect, Size};

    #[test]
    fn column_window_pointer_is_rebased_to_the_core_composition_origin() {
        let core = floating_local_point_to_core(
            Point::new(12.0, 24.0),
            Point::new(180.0, 96.0),
        )
        .unwrap();

        assert_eq!(core, Point::new(192.0, 120.0));
    }

    #[test]
    fn column_local_geometry_is_rebased_to_the_window_origin() {
        let local = floating_column_local_rect(
            Rect::new(Point::new(180.0, 96.0), Size::new(420.0, 260.0)),
            Point::new(180.0, 96.0),
        );

        assert_eq!(
            local,
            Rect::new(Point::ZERO, Size::new(420.0, 260.0)),
        );
    }
}


#[cfg(test)]
mod floating_column_host_geometry_tests {
    use super::floating_column_host_rect;
    use dock_core::{NodeId, Point, Rect, Size, SplitterRect};
    use slotmap::SlotMap;

    #[test]
    fn native_column_hosts_collapse_all_core_splitter_gaps() {
        let mut node_ids = SlotMap::<NodeId, ()>::with_key();
        let first_splitter = node_ids.insert(());
        let second_splitter = node_ids.insert(());
        let splitters = [
            SplitterRect::new(
                first_splitter,
                0,
                dock_core::Axis::X,
                Rect::new(Point::new(237.0, 0.0), Size::new(14.0, 400.0)),
            ),
            SplitterRect::new(
                second_splitter,
                1,
                dock_core::Axis::X,
                Rect::new(Point::new(505.0, 0.0), Size::new(14.0, 400.0)),
            ),
        ];

        let first = floating_column_host_rect(
            Rect::new(Point::ZERO, Size::new(240.0, 400.0)),
            &splitters,
            8.0,
        );
        let second = floating_column_host_rect(
            Rect::new(Point::new(248.0, 0.0), Size::new(260.0, 400.0)),
            &splitters,
            8.0,
        );
        let third = floating_column_host_rect(
            Rect::new(Point::new(516.0, 0.0), Size::new(200.0, 400.0)),
            &splitters,
            8.0,
        );

        assert_eq!(first, Rect::new(Point::ZERO, Size::new(240.0, 400.0)));
        assert_eq!(second, Rect::new(Point::new(240.0, 0.0), Size::new(260.0, 400.0)));
        assert_eq!(third, Rect::new(Point::new(500.0, 0.0), Size::new(200.0, 400.0)));
        assert_eq!(first.origin.x + first.size.width, second.origin.x);
        assert_eq!(second.origin.x + second.size.width, third.origin.x);
    }

    #[test]
    fn a_column_without_a_preceding_x_splitter_keeps_its_core_origin() {
        let column = Rect::new(Point::new(32.0, 16.0), Size::new(240.0, 400.0));

        assert_eq!(floating_column_host_rect(column, &[], 8.0), column);
    }
}

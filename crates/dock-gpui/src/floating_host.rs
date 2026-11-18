use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use dock_core::{FloatingGroupId, NodeId, Point};
use gpui::{
    Context, Entity, FocusHandle, HitboxId, IntoElement, ParentElement, Render, ScrollHandle,
    Styled, WeakEntity, Window, div, prelude::InteractiveElement as _, rgb,
};
use ui::Theme;

use crate::{
    dock_host::{build_context_menu_entries, context_menu_target_exists, render_resize_preview},
    dock_components::PanelTransferPresentation,
    input::interaction::DockInteractionCoordinator,
    DockController, FloatingWindowManager, PanelHost,
};

use super::{
    floating_dock_component_tab_zones, floating_host_snapshot, handle_floating_escape,
    observe_floating_focus_loss, render_floating_pointer_input, FloatingGroupPress,
    FloatingHostOpenMenu, FloatingTabPress, FloatingWindowContent,
};
use super::super::{
    context_menu::{
        ContextMenu, ContextMenuCallbacks, ContextMenuDismiss, ContextMenuSubmenuToggle,
        GroupMenuToggle,
    },
    drop_overlay::DropOverlay,
};

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
    native_bounds_observed: bool,
    native_resize_observation_pending: bool,
    controller_observed: bool,
    manager_observed: bool,
    hide_header: bool,
    hide_header_query_pending: bool,
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
            native_bounds_observed: false,
            native_resize_observation_pending: false,
            controller_observed: false,
            manager_observed: false,
            hide_header: false,
            hide_header_query_pending: false,
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
        if !self.native_bounds_observed {
            cx.observe_window_bounds(window, |host, window, host_cx| {
                let Some(group) = host.group else {
                    return;
                };
                let window_id = window.window_handle().window_id();

                // bounds_changed() can be invoked by FloatingWindowManager while
                // it is already reconciling a native HWND. GPUI calls this
                // observer synchronously, so re-entering manager.update here
                // would panic. Defer until the current effect/update cycle ends.
                host_cx.defer_in(window, move |host, _, deferred_cx| {
                    if host.group != Some(group) {
                        return;
                    }
                    if let Some(manager) = host.manager.upgrade() {
                        let _ = manager.update(deferred_cx, |manager, manager_cx| {
                            manager.sync_native_resize_bounds(group, window_id, manager_cx);
                        });
                    }
                });
            })
            .detach();
            self.native_bounds_observed = true;
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
            observe_floating_focus_loss(
                cx,
                &focus_handle,
                window,
                self.controller.clone(),
                self.manager.clone(),
                self.tab_press.clone(),
                self.group_press.clone(),
                self.pointer_capture.clone(),
            );
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
        if !self.hide_header_query_pending {
            self.hide_header_query_pending = true;
            let manager = self.manager.clone();
            let host = cx.entity().downgrade();
            window.on_next_frame(move |_, app| {
                let hide_header = manager.upgrade().is_some_and(|manager| {
                    manager.update(app, |manager, _| manager.hide_floating_header(group))
                });
                if let Some(host) = host.upgrade() {
                    host.update(app, |host, host_cx| {
                        host.hide_header_query_pending = false;
                        if host.group == Some(group) && host.hide_header != hide_header {
                            host.hide_header = hide_header;
                            host_cx.notify();
                        }
                    });
                }
            });
        }
        let hide_header = self.hide_header;

        let mut root = div()
            .id(format!("floating-host-root-{:?}", group))
            .track_focus(&focus_handle)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(self.theme.colors.panel_surface_background));

        let pointer_capture_for_escape = self.pointer_capture.clone();
        let input_window_id = window_id;
        let dock_component_tab_zones = snapshot
            .as_ref()
            .map(floating_dock_component_tab_zones)
            .unwrap_or_default();
        let dock_area_drag_area = snapshot.as_ref().map(|snapshot| {
            (
                snapshot.floating.rect,
                snapshot.rects.columns.clone(),
                snapshot.rects.splitters.clone(),
                snapshot.metrics,
                true,
            )
        });
        root = root.child(render_floating_pointer_input(
            group,
            self.manager.clone(),
            self.controller.clone(),
            input_window_id,
            snapshot.as_ref().map(|snapshot| snapshot.floating.rect),
            dock_component_tab_zones,
            dock_area_drag_area,
            hide_header,
            self.tab_press.clone(),
            self.group_press.clone(),
            self.pointer_capture.clone(),
            None,
            Point::ZERO,
        ));

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

        let transferred_panels = self.controller.update(cx, |controller, _| {
            controller.transferred_panel_views_from(window_id)
        });
        let mut group_presentations = snapshot.group_presentations.clone();
        let transfer_presentation =
            PanelTransferPresentation::new(&group_presentations, &transferred_panels);
        transfer_presentation.apply_to_group_presentations(&mut group_presentations);

        // Every PanelRect in compute_floating_rects belongs to this group and
        // is already expressed in local (0, 0) coordinates. When the active tab
        // is transported, reuse its content rectangle for the remaining active tab.
        let panel_views = transfer_presentation
            .panel_rects_with_active_replacements(&snapshot.rects.panels)
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
        PanelHost::confirm_source_detached_after_render(
            &self.controller,
            window,
            window_id,
            pending_transfers,
            &panel_views,
        );

        // Query and confirm handoff only after this render/update has unwound.
        // The manager can notify this legacy host while reconciling a Drop.
        let rendered_panels = panel_views
            .iter()
            .filter_map(|(rect, view)| view.as_ref().map(|_| rect.panel))
            .collect::<HashSet<_>>();
        let handoff_manager = self.manager.clone();
        window.on_next_frame(move |_, app| {
            let Some(manager) = handoff_manager.upgrade() else {
                return;
            };
            let Some(required_panels) = manager.update(app, |manager, manager_cx| {
                manager.pending_drag_host_handoff_panels(group, window_id, manager_cx)
            }) else {
                return;
            };
            if required_panels
                .iter()
                .all(|panel| rendered_panels.contains(panel))
            {
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

        let rendered_groups = snapshot
            .rects
            .groups
            .iter()
            .filter(|group| {
                !transfer_presentation
                    .groups_without_visible_tabs
                    .contains(&group.group)
                    || dragged_group == Some(group.group)
            })
            .copied()
            .collect::<Vec<_>>();
        let rendered_tab_strips = snapshot
            .rects
            .tab_strips
            .iter()
            .filter(|strip| {
                !transfer_presentation
                    .groups_without_visible_tabs
                    .contains(&strip.group)
                    || dragged_group == Some(strip.group)
            })
            .copied()
            .collect::<Vec<_>>();
        let column_group_ids = snapshot
            .column_groups
            .values()
            .flat_map(|groups| groups.iter().copied())
            .collect::<HashSet<_>>();
        let rendered_tab_headers = transfer_presentation.visible_tab_headers(
            &snapshot.rects.tab_headers,
            &snapshot.rects.tab_strips,
            &snapshot.rects.tab_overflows,
            &group_presentations,
            &column_group_ids,
            snapshot.metrics,
        );
        let rendered_tab_overflows = snapshot
            .rects
            .tab_overflows
            .iter()
            .filter(|overflow| {
                !transfer_presentation
                    .groups_without_visible_tabs
                    .contains(&overflow.group)
            })
            .cloned()
            .collect::<Vec<_>>();
        let rendered_panel_views = panel_views
            .into_iter()
            .filter(|(panel, _)| {
                !transfer_presentation.transferred_panels.contains(&panel.panel)
                    && !transfer_presentation
                        .panels_in_hidden_groups
                        .contains(&panel.panel)
            })
            .collect::<Vec<_>>();

        if self.open_context_menu.as_ref().is_some_and(|open| {
            !context_menu_target_exists(
                open.target,
                &group_presentations,
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
            &rendered_groups,
            rendered_panel_views,
            dragged_group,
            &rendered_tab_strips,
            &rendered_tab_headers,
            &rendered_tab_overflows,
            &group_presentations,
            &snapshot.column_groups,
            &snapshot.column_modes,
            &snapshot.rail_display_orders,
            &snapshot.rects.splitters,
            snapshot.resize_preview.and_then(|preview| preview.splitter),
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
            self.manager.clone(),
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
                &group_presentations,
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
            handle_floating_escape(
                event,
                context_menu_is_open,
                &manager_for_escape,
                &controller_for_escape,
                &tab_press_for_escape,
                &group_press_for_escape,
                &pointer_capture_for_escape,
                window,
                app,
            );
        });

        root
    }
}


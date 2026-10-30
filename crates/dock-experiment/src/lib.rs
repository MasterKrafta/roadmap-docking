//! Minimal experimental docking application entry point.

use dock_core::{
    Axis, DockCommand, DockMetrics, DragDropAction, DragHandle, DragPayload, DragSession,
    DragState, DockSide, DropSource, DropTarget, DropZone, DropZoneRect, DropZoneResolver,
    FloatingGroupId, Layout, Node, NodeId, PanelId, Point, Rect, Size,
};
#[cfg(windows)]
use dock_core::{PanelConstraints, PanelKey, Profile};
use gpui::{
    AnyElement, AppContext, Bounds, Context, Entity, IntoElement, Pixels, Render, Subscription,
    WeakEntity, Window, WindowBounds, WindowHandle, WindowKind, WindowOptions, WindowVisibility,
    div, px, rgb, size,
};
use gpui::prelude::*;
use std::collections::HashSet;
#[cfg(windows)]
use std::time::Duration;
#[cfg(windows)]
use gpui::{App, MouseButton, TitlebarOptions};
#[cfg(windows)]
use gpui_platform::application;
#[cfg(windows)]
use platform::windows::NativeWindowHandle;

/// Starts the standalone docking experiment.
///
/// The experiment intentionally remains separate from the production
/// `dock-gpui` entry point while its architecture is being evaluated.
pub fn run() {
    #[cfg(windows)]
    run_windows();

    #[cfg(not(windows))]
    eprintln!("The docking experiment currently targets Windows.");
}

#[cfg(windows)]
fn run_windows() {
    platform::windows::begin_high_resolution_timer();

    application().run(|cx: &mut App| {
        let metrics = DockMetrics::default();
        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        let panel_size = size(px(280.0), px(360.0));
        let panel_top = bounds.origin.y + px(180.0);
        // Keep the two startup panels separate so one native popup cannot hide
        // the other at exactly the same centered position.
        let panel_bounds = [
            Bounds::new(
                gpui::Point::new(bounds.origin.x + px(48.0), panel_top),
                panel_size,
            ),
            Bounds::new(
                gpui::Point::new(
                    bounds.origin.x + bounds.size.width - px(328.0),
                    panel_top,
                ),
                panel_size,
            ),
        ];
        let (layout, panel_bindings) = match build_initial_layout(metrics, panel_bounds) {
            Ok(result) => result,
            Err(error) => {
                eprintln!("Could not build the initial dock trees: {error}");
                return;
            }
        };
        let layout = cx.new(|_| layout);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            show: false,
            focus: false,
            titlebar: Some(TitlebarOptions {
                title: Some("Docking experiment".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let main_window = match cx.open_window(options, move |window, cx| {
            let main_window =
                cx.new(|_| MainWindow::new(layout, metrics, panel_bindings));
            let weak_main_window = main_window.downgrade();

            window.on_window_should_close(cx, move |_window, app| {
                if let Err(error) = weak_main_window.update(app, |this, cx| {
                    this.close_floating_panels(cx);
                }) {
                    eprintln!("Could not close floating panels: {error:?}");
                }

                true
            });

            main_window
        }) {
            Ok(handle) => handle,
            Err(error) => {
                eprintln!("Could not open the experiment window: {error}");
                return;
            }
        };

        if let Err(error) = main_window.update(cx, |this, window, cx| {
            this.set_native_owner(window);
            this.open_startup_panels(cx);
        }) {
            eprintln!("Could not open the startup panels: {error:?}");
            return;
        }

        // All three windows have now completed GPUI's initial draw while hidden.
        // Show the main window first, then reveal the already-rendered panels.
        if let Err(error) = main_window.update(cx, |this, window, cx| {
            window.activate_window();
            this.show_floating_panels(cx);
        }) {
            eprintln!("Could not show the startup windows: {error:?}");
            return;
        }

        cx.activate(true);
    });
}

pub(crate) struct MainWindow {
    layout: Entity<Layout>,
    metrics: DockMetrics,
    panel_bindings: [PanelBinding; 2],
    floating_panels: Vec<FloatingPanelWindow>,
    panels_minimized_by_main: Vec<WindowHandle<FloatingPanel>>,
    visibility_subscription: Option<Subscription>,
    layout_subscription: Option<Subscription>,
    panel_drag: Option<PanelDrag>,
    last_dock_viewport: Option<Rect>,
    last_window_bounds: Option<Bounds<Pixels>>,
    last_viewport_size: Option<gpui::Size<Pixels>>,
    last_surface_top: f32,
    main_window_handle: Option<WindowHandle<MainWindow>>,
    #[cfg(windows)]
    native_owner: Option<NativeWindowHandle>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PanelKind {
    Layers,
    Properties,
}

#[derive(Clone, Copy)]
struct PanelBinding {
    kind: PanelKind,
    panel_id: PanelId,
    floating_group_id: Option<FloatingGroupId>,
    window_bounds: Bounds<Pixels>,
}

struct FloatingPanelWindow {
    panel_id: PanelId,
    handle: WindowHandle<FloatingPanel>,
    visible: bool,
    host: bool,
}

struct PanelDrag {
    panel_id: PanelId,
    session: DragSession,
    resolver: Option<DropZoneResolver>,
    zones: Vec<DropZoneRect>,
    target: Option<DropTarget>,
    pointer: Point,
    viewport: Rect,
    layout_version: u64,
    floating: bool,
    #[cfg(windows)]
    native_window: NativeWindowHandle,
    #[cfg(windows)]
    cursor_offset: Option<(i32, i32)>,
}

#[cfg(windows)]
enum PanelDragPoll {
    Continue,
    Stop,
    Docked,
}

impl PanelKind {
    const fn key(self) -> &'static str {
        match self {
            Self::Layers => "layers",
            Self::Properties => "properties",
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::Layers => "Слои",
            Self::Properties => "Свойства",
        }
    }
}

impl MainWindow {
    #[cfg(windows)]
    pub(crate) fn set_native_owner(&mut self, window: &Window) {
        self.main_window_handle = Some(WindowHandle::new(window.window_handle().window_id()));
        self.native_owner = NativeWindowHandle::from_window(window);
        if self.native_owner.is_none() {
            eprintln!("Could not obtain the main window's native handle");
        }
    }

    #[cfg(windows)]
    fn configure_floating_window(
        &self,
        panel: &WindowHandle<FloatingPanel>,
        cx: &mut Context<Self>,
    ) {
        let Some(owner) = self.native_owner else {
            eprintln!("Could not assign a native owner to a floating panel");
            return;
        };

        match panel.update(cx, |_, window, _| {
            let Some(panel_handle) = NativeWindowHandle::from_window(window) else {
                return false;
            };

            platform::windows::set_owner(panel_handle, owner);
            platform::windows::remove_topmost(panel_handle)
        }) {
            Ok(true) => {}
            Ok(false) => {
                eprintln!("Could not remove topmost state from a floating panel");
            }
            Err(error) => {
                eprintln!("Could not configure a floating panel: {error:?}");
            }
        }
    }

    pub(crate) fn new(
        layout: Entity<Layout>,
        metrics: DockMetrics,
        panel_bindings: [PanelBinding; 2],
    ) -> Self {
        Self {
            layout,
            metrics,
            panel_bindings,
            floating_panels: Vec::new(),
            panels_minimized_by_main: Vec::new(),
            visibility_subscription: None,
            layout_subscription: None,
            panel_drag: None,
            last_dock_viewport: None,
            last_window_bounds: None,
            last_viewport_size: None,
            last_surface_top: 0.0,
            main_window_handle: None,
            #[cfg(windows)]
            native_owner: None,
        }
    }

    fn apply_layout_command(
        &mut self,
        command: DockCommand,
        cx: &mut Context<Self>,
    ) -> Result<(), dock_core::CommandError> {
        self.layout.update(cx, |layout, layout_cx| {
            layout.apply(command, &self.metrics)?;
            layout_cx.notify();
            Ok(())
        })
    }


    /// Opens the panels that should be visible when the experiment starts.
    pub(crate) fn open_startup_panels(&mut self, cx: &mut Context<Self>) {
        let _ = self.open_floating_panel(PanelKind::Layers, false, cx);
        let _ = self.open_floating_panel(PanelKind::Properties, false, cx);
    }

    /// Shows the already-created panels after GPUI has rendered their first frame.
    pub(crate) fn show_floating_panels(&mut self, cx: &mut Context<Self>) {
        let floating_panel_ids = {
            let layout = self.layout.read(cx);
            self.floating_panels
                .iter()
                .filter(|panel| floating_group_for_panel(&layout, panel.panel_id).is_some())
                .map(|panel| panel.panel_id)
                .collect::<HashSet<_>>()
        };
        let mut handles = Vec::new();
        for panel in &mut self.floating_panels {
            if floating_panel_ids.contains(&panel.panel_id) {
                panel.visible = true;
                handles.push(panel.handle.clone());
            }
        }
        for handle in handles {
            if let Err(error) = handle.update(cx, |_, window, _| {
                #[cfg(windows)]
                if let Some(native_window) = NativeWindowHandle::from_window(window) {
                    platform::windows::show_window(native_window);
                }
                window.activate_window();
            }) {
                eprintln!("Could not show a startup panel: {error:?}");
            }
        }
    }

    fn handle_main_window_visibility(
        &mut self,
        visibility: WindowVisibility,
        cx: &mut Context<Self>,
    ) {
        match visibility {
            WindowVisibility::Hidden => {
                self.panels_minimized_by_main.clear();
                let floating_panel_ids = {
                    let layout = self.layout.read(cx);
                    self.panel_bindings
                        .iter()
                        .filter_map(|binding| {
                            floating_group_for_panel(&layout, binding.panel_id)
                                .map(|_| binding.panel_id)
                        })
                        .collect::<HashSet<_>>()
                };

                for panel in &self.floating_panels {
                    if !floating_panel_ids.contains(&panel.panel_id) || !panel.visible {
                        continue;
                    }
                    match panel.handle.update(cx, |_, window, _| {
                        window.minimize_window();
                    }) {
                        Ok(()) => self
                            .panels_minimized_by_main
                            .push(panel.handle.clone()),
                        Err(_error) => {
                            // The panel may already have been closed by the user.
                        }
                    }
                }
            }
            WindowVisibility::Visible => {
                let minimized_window_ids = self
                    .panels_minimized_by_main
                    .drain(..)
                    .map(|panel| panel.window_id())
                    .collect::<HashSet<_>>();
                let floating_panel_ids = {
                    let layout = self.layout.read(cx);
                    self.panel_bindings
                        .iter()
                        .filter_map(|binding| {
                            floating_group_for_panel(&layout, binding.panel_id)
                                .map(|_| binding.panel_id)
                        })
                        .collect::<HashSet<_>>()
                };
                let handles = self
                    .floating_panels
                    .iter()
                    .filter(|panel| {
                        panel.visible
                            && floating_panel_ids.contains(&panel.panel_id)
                            && minimized_window_ids.contains(&panel.handle.window_id())
                    })
                    .map(|panel| panel.handle.clone())
                    .collect::<Vec<_>>();

                for panel in handles {
                    if let Err(error) = panel.update(cx, |_, window, _| {
                        #[cfg(windows)]
                        if let Some(native_window) = NativeWindowHandle::from_window(window) {
                            platform::windows::show_window(native_window);
                        }
                        window.activate_window();
                    }) {
                        eprintln!("Could not restore a floating panel: {error:?}");
                    }
                }
            }
        }
    }

    fn floating_drop_highlight(
        &self,
        panel_id: PanelId,
        cx: &Context<Self>,
    ) -> Option<Rect> {
        let drag = self.panel_drag.as_ref()?;
        if !drag.floating {
            return None;
        }
        let target = drag.target?;
        let DropTarget::GroupEdge {
            group: target_node,
            zone: target_zone @ (DropZone::Left | DropZone::Right),
        } = target
        else {
            return None;
        };

        let layout = self.layout.read(cx);
        let panel_group = floating_group_for_panel(&layout, panel_id)?;
        let target_group = floating_group_for_node(&layout, target_node)?;
        if panel_group != target_group {
            return None;
        }

        let floating = layout.floating_group(panel_group)?;
        let zone = drag.zones.iter().find(|zone| {
            zone.target == target
                && zone.zone == target_zone
                && zone.rect.contains(drag.pointer).unwrap_or(false)
        })?;

        let local_origin = if panel_group == target_group {
            Point::new(
                zone.rect.origin.x - floating.bounds.origin.x,
                zone.rect.origin.y - floating.bounds.origin.y,
            )
        } else {
            #[cfg(windows)]
            {
                let source_group = floating_group_for_panel(&layout, drag.panel_id)?;
                if source_group != panel_group {
                    return None;
                }
                let scale = platform::windows::scale_factor(drag.native_window).max(0.01);
                let (offset_x, offset_y) = drag.cursor_offset?;
                Point::new(
                    zone.rect.origin.x - drag.pointer.x + offset_x as f32 / scale,
                    zone.rect.origin.y - drag.pointer.y + offset_y as f32 / scale,
                )
            }
            #[cfg(not(windows))]
            {
                return None;
            }
        };

        Some(Rect::new(local_origin, zone.rect.size))
    }

    #[cfg(windows)]
    fn sync_floating_panel_drop_highlights(&self, cx: &mut Context<Self>) {
        let panels = self
            .floating_panels
            .iter()
            .filter(|panel| panel.visible && panel.host)
            .map(|panel| {
                (
                    panel.handle.clone(),
                    self.floating_drop_highlight(panel.panel_id, cx),
                )
            })
            .collect::<Vec<_>>();

        for (handle, highlight) in panels {
            if let Err(error) = handle.update(cx, |panel, _window, panel_cx| {
                if panel.drop_highlight != highlight {
                    panel.drop_highlight = highlight;
                    panel_cx.notify();
                }
            }) {
                eprintln!("Could not update a floating panel drop preview: {error:?}");
            }
        }
    }

    fn sync_floating_panel_bounds(
        &mut self,
        group: FloatingGroupId,
        native_bounds: Bounds<Pixels>,
        viewport_size: gpui::Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> Result<(), dock_core::CommandError> {
        let is_being_dragged = self.panel_drag.as_ref().is_some_and(|drag| {
            drag.floating
                && self.panel_bindings.iter().any(|binding| {
                    binding.panel_id == drag.panel_id
                        && binding.floating_group_id == Some(group)
                })
        });
        if is_being_dragged {
            for binding in self
                .panel_bindings
                .iter_mut()
                .filter(|binding| binding.floating_group_id == Some(group))
            {
                binding.window_bounds = native_bounds;
            }
            return Ok(());
        }

        let (Some(parent_bounds), Some(parent_viewport)) =
            (self.last_window_bounds, self.last_viewport_size)
        else {
            // The main window has not rendered yet. Its next bounds observation
            // will provide the coordinate space needed for this conversion.
            return Ok(());
        };
        let parent_chrome_height =
            (parent_bounds.size.height.as_f32() - parent_viewport.height.as_f32()).max(0.0);
        let bounds = Rect::new(
            Point::new(
                native_bounds.origin.x.as_f32() - parent_bounds.origin.x.as_f32(),
                native_bounds.origin.y.as_f32()
                    - parent_bounds.origin.y.as_f32()
                    - parent_chrome_height
                    - self.last_surface_top,
            ),
            Size::new(
                viewport_size.width.as_f32(),
                viewport_size.height.as_f32(),
            ),
        );

        self.layout.update(cx, |layout, layout_cx| {
            layout.apply(
                DockCommand::ResizeFloatingBounds { group, bounds },
                &self.metrics,
            )?;
            layout_cx.notify();
            Ok(())
        })?;

        for binding in self
            .panel_bindings
            .iter_mut()
            .filter(|binding| binding.floating_group_id == Some(group))
        {
            binding.window_bounds = native_bounds;
        }

        cx.notify();
        Ok(())
    }

    fn open_floating_panel(
        &mut self,
        kind: PanelKind,
        show_immediately: bool,
        cx: &mut Context<Self>,
    ) -> Option<WindowHandle<FloatingPanel>> {
        let Some(binding) = self
            .panel_bindings
            .iter()
            .find(|binding| binding.kind == kind)
            .copied()
        else {
            eprintln!("No dock-core tree is registered for panel '{}'", kind.key());
            return None;
        };

        let floating_group_id = {
            let layout = self.layout.read(cx);
            floating_group_for_panel(&layout, binding.panel_id)
        };

        let Some(floating_group_id) = floating_group_id else {
            let docked_group = {
                let layout = self.layout.read(cx);
                main_tabs_group_for_panel(&layout, binding.panel_id)
            };
            if let Some(group) = docked_group {
                if let Err(error) = self.apply_layout_command(
                    DockCommand::ActivateTab {
                        group,
                        panel: binding.panel_id,
                    },
                    cx,
                ) {
                    eprintln!("Could not activate docked panel '{}': {error}", kind.key());
                }
            } else {
                eprintln!("Panel '{}' is not present in the dock layout", kind.key());
            }
            return self
                .floating_panels
                .iter()
                .find(|panel| panel.panel_id == binding.panel_id)
                .map(|panel| panel.handle.clone());
        };

        if let Err(error) = self.apply_layout_command(
            DockCommand::RaiseFloating {
                group: floating_group_id,
            },
            cx,
        ) {
            eprintln!("Could not raise floating panel '{}': {error}", kind.key());
            return None;
        }

        let composition_panel_ids = {
            let layout = self.layout.read(cx);
            floating_group_panel_ids(&layout, floating_group_id)
        };
        let host_panel_id = self
            .floating_panels
            .iter()
            .find(|panel| panel.host && composition_panel_ids.contains(&panel.panel_id))
            .or_else(|| {
                self.floating_panels
                    .iter()
                    .find(|panel| composition_panel_ids.contains(&panel.panel_id))
            })
            .map(|panel| panel.panel_id)
            .unwrap_or(binding.panel_id);

        if let Some(existing) = self
            .floating_panels
            .iter()
            .find(|panel| panel.panel_id == host_panel_id)
        {
            let handle = existing.handle.clone();
            if show_immediately {
                for panel in self
                    .floating_panels
                    .iter_mut()
                    .filter(|panel| composition_panel_ids.contains(&panel.panel_id))
                {
                    panel.visible = panel.panel_id == host_panel_id;
                }
            }
            if show_immediately
                && let Err(error) = handle.update(cx, |_, window, _| {
                    #[cfg(windows)]
                    if let Some(native_window) = NativeWindowHandle::from_window(window) {
                        platform::windows::show_window(native_window);
                    }
                    window.activate_window();
                })
            {
                eprintln!("Could not activate floating panel '{}': {error:?}", kind.key());
            }
            return Some(handle);
        }

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(binding.window_bounds)),
            // Configure ownership and z-order before making this window visible.
            show: false,
            focus: false,
            titlebar: None,
            kind: WindowKind::PopUp,
            is_movable: false,
            ..Default::default()
        };

        let owner = cx.entity().downgrade();
        let layout = self.layout.clone();
        let panel_id = binding.panel_id;
        let close_owner = owner.clone();
        match cx.open_window(options, move |window, cx| {
            #[cfg(windows)]
            window.on_window_should_close(cx, move |window, app| {
                if let Some(native_window) = NativeWindowHandle::from_window(window) {
                    platform::windows::hide_window(native_window);
                }

                let main_window = close_owner.update(app, |main, cx| {
                    let composition_panel_ids = {
                        let layout = main.layout.read(cx);
                        floating_group_for_panel(&layout, panel_id)
                            .map(|group| floating_group_panel_ids(&layout, group))
                            .unwrap_or_else(|| vec![panel_id])
                    };
                    for panel in main
                        .floating_panels
                        .iter_mut()
                        .filter(|panel| composition_panel_ids.contains(&panel.panel_id))
                    {
                        panel.visible = false;
                    }
                    main.main_window_handle
                });
                if let Ok(Some(main_window)) = main_window {
                    let _ = main_window.update(app, |_, window, _| window.activate_window());
                }
                false
            });

            cx.new(|_| FloatingPanel {
                panel_id,
                owner,
                layout,
                layout_subscription: None,
                bounds_subscription: None,
                drop_highlight: None,
            })
        }) {
            Ok(handle) => {
                #[cfg(windows)]
                self.configure_floating_window(&handle, cx);

                self.floating_panels.push(FloatingPanelWindow {
                    panel_id,
                    handle: handle.clone(),
                    visible: show_immediately,
                    host: true,
                });

                if show_immediately
                    && let Err(error) = handle.update(cx, |_, window, _| {
                        #[cfg(windows)]
                        if let Some(native_window) = NativeWindowHandle::from_window(window) {
                            platform::windows::show_window(native_window);
                        }
                        window.activate_window();
                    })
                {
                    eprintln!("Could not show floating panel '{}': {error:?}", kind.key());
                }
                Some(handle)
            }
            Err(error) => {
                eprintln!("Could not open floating panel '{}': {error}", kind.key());
                None
            }
        }
    }

    #[cfg(windows)]
    fn begin_docked_panel_drag(
        &mut self,
        group: NodeId,
        panel_id: PanelId,
        cx: &mut Context<Self>,
    ) {
        if self.panel_drag.is_some() {
            return;
        }

        let Some(viewport) = self.last_dock_viewport else {
            return;
        };
        let (cursor_x, cursor_y) = platform::windows::cursor_position();
        let Some(pointer) = self.external_panel_pointer_from_screen(cursor_x, cursor_y) else {
            return;
        };

        let is_valid_source = {
            let layout = self.layout.read(cx);
            matches!(
                layout.node(group),
                Some(Node::Tabs(tabs))
                    if tabs.children.iter().any(|child| {
                        matches!(layout.node(*child), Some(Node::Panel(panel)) if panel.panel_id == panel_id)
                    })
            ) && floating_group_for_panel(&layout, panel_id).is_none()
        };
        if !is_valid_source {
            return;
        }

        if let Err(error) = self.apply_layout_command(
            DockCommand::ActivateTab {
                group,
                panel: panel_id,
            },
            cx,
        ) {
            eprintln!("Could not activate the panel before dragging: {error}");
            return;
        }

        let handle = self
            .floating_panels
            .iter()
            .find(|panel| panel.panel_id == panel_id)
            .map(|panel| panel.handle.clone());
        let Some(handle) = handle else {
            eprintln!("The persistent window for the docked panel is missing");
            return;
        };
        let native_window = match handle.update(cx, |_, window, _| {
            NativeWindowHandle::from_window(window)
        }) {
            Ok(Some(native_window)) => native_window,
            Ok(None) => {
                eprintln!("Could not get the persistent native window for the docked panel");
                return;
            }
            Err(error) => {
                eprintln!("Could not access the persistent docked panel window: {error:?}");
                return;
            }
        };

        let layout_version = self.layout.read(cx).version();
        self.start_panel_drag(
            panel_id,
            native_window,
            None,
            false,
            pointer,
            viewport,
            layout_version,
            cx,
        );
    }

    #[cfg(windows)]
    fn begin_floating_panel_drag(
        &mut self,
        panel_id: PanelId,
        native_window: NativeWindowHandle,
        cx: &mut Context<Self>,
    ) {
        if self.panel_drag.is_some() {
            return;
        }

        let Some(viewport) = self.last_dock_viewport else {
            return;
        };
        let (cursor_x, cursor_y) = platform::windows::cursor_position();
        let Some((window_left, window_top, _, _)) =
            platform::windows::window_rect(native_window)
        else {
            eprintln!("Could not read the floating panel's native bounds");
            return;
        };
        let Some(pointer) = self.external_panel_pointer_from_screen(cursor_x, cursor_y) else {
            return;
        };

        let layout_version = {
            let layout = self.layout.read(cx);
            if floating_group_for_panel(&layout, panel_id).is_none() {
                eprintln!("Panel drag source is not inside a floating group");
                return;
            }
            layout.version()
        };

        self.start_panel_drag(
            panel_id,
            native_window,
            Some((cursor_x - window_left, cursor_y - window_top)),
            true,
            pointer,
            viewport,
            layout_version,
            cx,
        );
    }

    #[cfg(windows)]
    #[allow(clippy::too_many_arguments)]
    fn start_panel_drag(
        &mut self,
        panel_id: PanelId,
        native_window: NativeWindowHandle,
        cursor_offset: Option<(i32, i32)>,
        floating: bool,
        pointer: Point,
        viewport: Rect,
        layout_version: u64,
        cx: &mut Context<Self>,
    ) {
        if self.panel_drag.is_some() {
            return;
        }

        let mut session = match DragSession::new(&self.metrics) {
            Ok(session) => session,
            Err(error) => {
                eprintln!("Could not create a panel drag session: {error}");
                return;
            }
        };
        if let Err(error) = session.pointer_down_with_handle(
            DragPayload::Panel { panel: panel_id },
            DragHandle::PanelTab,
            pointer,
        ) {
            eprintln!("Could not begin a panel drag session: {error}");
            return;
        }

        self.panel_drag = Some(PanelDrag {
            panel_id,
            session,
            resolver: None,
            zones: Vec::new(),
            target: None,
            pointer,
            viewport,
            layout_version,
            floating,
            native_window,
            cursor_offset,
        });

        let main_window_handle = self.main_window_handle;
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(4))
                .await;

            match this.update(cx, |main, cx| main.poll_panel_drag(cx)) {
                Ok(PanelDragPoll::Continue) => {}
                Ok(PanelDragPoll::Docked) => {
                    if let Some(main_window) = main_window_handle
                        && let Err(error) =
                            main_window.update(cx, |_, window, _| window.activate_window())
                    {
                        eprintln!("Could not activate the main window after docking: {error:?}");
                    }
                    break;
                }
                Ok(PanelDragPoll::Stop) | Err(_) => break,
            }
        })
        .detach();
    }

    #[cfg(windows)]
    fn poll_panel_drag(&mut self, cx: &mut Context<Self>) -> PanelDragPoll {
        let Some(drag) = self.panel_drag.as_ref() else {
            return PanelDragPoll::Stop;
        };
        let panel_id = drag.panel_id;
        let viewport = drag.viewport;
        let layout_version = drag.layout_version;
        let (cursor_x, cursor_y) = platform::windows::cursor_position();
        let left_button_down = platform::windows::left_button_down();

        let Some(pointer) = self.external_panel_pointer_from_screen(cursor_x, cursor_y) else {
            let was_floating = self.panel_drag.take().is_some_and(|drag| drag.floating);
            self.sync_floating_panel_drop_highlights(cx);
            if was_floating {
                self.sync_floating_panel_after_drag(panel_id, cx);
            }
            cx.notify();
            return PanelDragPoll::Stop;
        };

        if self.last_dock_viewport != Some(viewport)
            || self.layout.read(cx).version() != layout_version
        {
            let was_floating = self.panel_drag.take().is_some_and(|drag| drag.floating);
            self.sync_floating_panel_drop_highlights(cx);
            if was_floating {
                self.sync_floating_panel_after_drag(panel_id, cx);
            }
            cx.notify();
            return PanelDragPoll::Stop;
        }

        let pointer_update_error = match self.panel_drag.as_mut() {
            Some(drag) => drag.session.pointer_move(pointer).err(),
            None => return PanelDragPoll::Stop,
        };
        if let Some(error) = pointer_update_error {
            eprintln!("Could not update the panel drag pointer: {error}");
            let was_floating = self.panel_drag.take().is_some_and(|drag| drag.floating);
            self.sync_floating_panel_drop_highlights(cx);
            if was_floating {
                self.sync_floating_panel_after_drag(panel_id, cx);
            }
            cx.notify();
            return PanelDragPoll::Stop;
        }

        let crossed_threshold = self.panel_drag.as_ref().is_some_and(|drag| {
            matches!(drag.session.state(), DragState::Dragging { .. })
        });
        let is_floating = self
            .panel_drag
            .as_ref()
            .is_some_and(|drag| drag.floating);

        if crossed_threshold
            && !is_floating
            && !self.detach_docked_panel_for_drag(panel_id, pointer, cursor_x, cursor_y, cx)
        {
            self.panel_drag = None;
            self.sync_floating_panel_drop_highlights(cx);
            cx.notify();
            return PanelDragPoll::Stop;
        }

        let needs_resolver = self.panel_drag.as_ref().is_some_and(|drag| {
            drag.floating
                && matches!(drag.session.state(), DragState::Dragging { .. })
                && drag.resolver.is_none()
        });
        let mut resolver_created = false;
        if needs_resolver {
            let prepared = {
                let layout = self.layout.read(cx);
                let visible_floating_groups = self
                    .floating_panels
                    .iter()
                    .filter(|panel| panel.visible)
                    .filter_map(|panel| floating_group_for_panel(&layout, panel.panel_id))
                    .collect::<HashSet<_>>();
                prepare_external_panel_resolver(
                    &layout,
                    viewport,
                    self.metrics,
                    panel_id,
                    &visible_floating_groups,
                )
                .map(|(resolver, zones)| (layout.version(), resolver, zones))
            };
            match prepared {
                Ok((version, resolver, zones)) => {
                    let expected_version = self
                        .panel_drag
                        .as_ref()
                        .map(|drag| drag.layout_version)
                        .unwrap_or_default();
                    if version != expected_version {
                        self.panel_drag = None;
                        self.sync_floating_panel_drop_highlights(cx);
                        self.sync_floating_panel_after_drag(panel_id, cx);
                        cx.notify();
                        return PanelDragPoll::Stop;
                    }
                    if let Some(drag) = self.panel_drag.as_mut() {
                        drag.resolver = Some(resolver);
                        drag.zones = zones;
                        resolver_created = true;
                    }
                }
                Err(error) => {
                    eprintln!("Could not prepare panel drop zones: {error}");
                    self.panel_drag = None;
                    self.sync_floating_panel_drop_highlights(cx);
                    self.sync_floating_panel_after_drag(panel_id, cx);
                    cx.notify();
                    return PanelDragPoll::Stop;
                }
            }
        }

        let mut target_changed = false;
        let mut pointer_changed = false;
        if let Some(drag) = self.panel_drag.as_mut() {
            pointer_changed = drag.pointer.x != pointer.x || drag.pointer.y != pointer.y;
            drag.pointer = pointer;
            let target = drag
                .resolver
                .as_ref()
                .and_then(|resolver| resolve_edge_drop_target(resolver, &drag.zones, pointer));
            target_changed = drag.target != target;
            drag.target = target;
            if drag.resolver.is_some()
                && let Err(error) = drag.session.set_drop_target(target)
            {
                eprintln!("Could not update the panel drag target: {error}");
            }
        }

        let movement = self.panel_drag.as_ref().and_then(|drag| {
            if matches!(drag.session.state(), DragState::Dragging { .. }) {
                drag.cursor_offset
                    .map(|offset| (drag.native_window, offset))
            } else {
                None
            }
        });
        if let Some((native_window, (offset_x, offset_y))) = movement
            && !platform::windows::move_window(
                native_window,
                cursor_x - offset_x,
                cursor_y - offset_y,
            )
        {
            eprintln!("Could not move the floating panel during the docking drag");
        }

        if pointer_changed || target_changed || resolver_created {
            self.sync_floating_panel_drop_highlights(cx);
        }
        if pointer_changed || target_changed || resolver_created {
            cx.notify();
        }

        if left_button_down {
            PanelDragPoll::Continue
        } else if self.finish_panel_drag(pointer, cx) {
            PanelDragPoll::Docked
        } else {
            PanelDragPoll::Stop
        }
    }

    #[cfg(windows)]
    fn detach_docked_panel_for_drag(
        &mut self,
        panel_id: PanelId,
        pointer: Point,
        cursor_x: i32,
        cursor_y: i32,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(binding) = self
            .panel_bindings
            .iter()
            .find(|binding| binding.panel_id == panel_id)
            .copied()
        else {
            eprintln!("No persistent window binding exists for the dragged panel");
            return false;
        };
        let Some(viewport) = self.last_dock_viewport else {
            return false;
        };

        let source = {
            let layout = self.layout.read(cx);
            let Some(column) = main_column_for_panel(&layout, panel_id) else {
                eprintln!("Could not find the dock column containing the dragged panel");
                return false;
            };
            let Some(group) = tabs_group_for_panel(&layout, column, panel_id) else {
                eprintln!("Could not find the tab group containing the dragged panel");
                return false;
            };
            let Some(Node::Tabs(tabs)) = layout.node(group) else {
                eprintln!("The dragged panel is not inside a tab group");
                return false;
            };
            let Some(Node::Column(column_node)) = layout.node(column) else {
                eprintln!("The dragged panel's owner is not a column");
                return false;
            };
            let Ok(rects) = layout.compute_rects(viewport, &self.metrics) else {
                eprintln!("Could not compute the source column bounds for undocking");
                return false;
            };
            let Some(column_rect) = rects
                .columns
                .iter()
                .find(|entry| entry.column == column)
                .map(|entry| entry.rect)
            else {
                eprintln!("The source column has no computed bounds");
                return false;
            };
            (
                column,
                group,
                tabs.children.len(),
                column_node.groups.len(),
                column_rect,
            )
        };

        let Some(parent_bounds) = self.last_window_bounds else {
            return false;
        };
        let Some(parent_viewport) = self.last_viewport_size else {
            return false;
        };
        let parent_chrome_height =
            (parent_bounds.size.height.as_f32() - parent_viewport.height.as_f32()).max(0.0);
        let window_bounds = Bounds::new(
            gpui::Point::new(
                parent_bounds.origin.x + px(source.4.origin.x),
                parent_bounds.origin.y
                    + px(parent_chrome_height + self.last_surface_top + source.4.origin.y),
            ),
            binding.window_bounds.size,
        );
        let floating_bounds = Rect::new(
            Point::ZERO,
            Size::new(
                window_bounds.size.width.as_f32(),
                window_bounds.size.height.as_f32(),
            ),
        );

        let handle = self
            .floating_panels
            .iter()
            .find(|panel| panel.panel_id == panel_id)
            .map(|panel| panel.handle.clone());
        let Some(handle) = handle else {
            eprintln!("The persistent PopUp window for the dragged panel is missing");
            return false;
        };
        let native_window = match handle.update(cx, |_, window, _| {
            NativeWindowHandle::from_window(window)
        }) {
            Ok(Some(native_window)) => native_window,
            Ok(None) => {
                eprintln!("Could not access the persistent PopUp window's native handle");
                return false;
            }
            Err(error) => {
                eprintln!("Could not inspect the persistent PopUp window: {error:?}");
                return false;
            }
        };

        let Some(main_window) = self.native_owner else {
            return false;
        };
        let scale = platform::windows::scale_factor(main_window).max(0.01);
        let cursor_offset = (
            ((pointer.x - source.4.origin.x) * scale).round() as i32,
            ((pointer.y - source.4.origin.y) * scale).round() as i32,
        );

        let command = if source.2 > 1 {
            DockCommand::FloatPanelAsGroup {
                panel: panel_id,
                bounds: floating_bounds,
            }
        } else if source.3 > 1 {
            DockCommand::FloatGroupAsColumn {
                group: source.1,
                bounds: floating_bounds,
            }
        } else {
            DockCommand::Float {
                node: source.0,
                bounds: floating_bounds,
            }
        };

        if let Err(error) = self.apply_layout_command(command, cx) {
            eprintln!("Could not undock the dragged panel: {error}");
            return false;
        }

        let floating_group_id = {
            let layout = self.layout.read(cx);
            floating_group_for_panel(&layout, panel_id)
        };
        let Some(floating_group_id) = floating_group_id else {
            eprintln!("Dock-core did not create a floating group after undocking");
            return false;
        };

        if let Some(binding) = self
            .panel_bindings
            .iter_mut()
            .find(|binding| binding.panel_id == panel_id)
        {
            binding.floating_group_id = Some(floating_group_id);
            binding.window_bounds = window_bounds;
        }
        if let Some(panel) = self
            .floating_panels
            .iter_mut()
            .find(|panel| panel.panel_id == panel_id)
        {
            panel.visible = true;
        }

        // Update the shared drag state before moving the native window so any
        // bounds observer sees this as an active drag and cannot invalidate it.
        let layout_version = self.layout.read(cx).version();
        if let Some(drag) = self.panel_drag.as_mut() {
            drag.native_window = native_window;
            drag.cursor_offset = Some(cursor_offset);
            drag.floating = true;
            drag.layout_version = layout_version;
            drag.resolver = None;
            drag.zones.clear();
            drag.target = None;
            drag.pointer = pointer;
        }

        if !platform::windows::move_window(
            native_window,
            cursor_x - cursor_offset.0,
            cursor_y - cursor_offset.1,
        ) {
            eprintln!("Could not position the persistent PopUp window after undocking");
        }
        platform::windows::show_window(native_window);

        cx.notify();
        true
    }

    #[cfg(windows)]
    fn finish_panel_drag(&mut self, pointer: Point, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.panel_drag.take() else {
            return false;
        };
        self.sync_floating_panel_drop_highlights(cx);

        let panel_id = drag.panel_id;
        let native_window = drag.native_window;
        let was_floating = drag.floating;
        let target = drag.target;
        let request = match {
            let mut session = drag.session;
            session.pointer_up(pointer, target)
        } {
            Ok(request) => request,
            Err(error) => {
                eprintln!("Could not finish the panel drag session: {error}");
                if was_floating {
                    self.sync_floating_panel_after_drag(panel_id, cx);
                }
                cx.notify();
                return false;
            }
        };

        if was_floating
            && request.is_some()
            && let Some(DropTarget::GroupEdge {
                group: target_node,
                zone: zone @ (DropZone::Left | DropZone::Right),
            }) = target
        {
            let joins_another_floating_group = {
                let layout = self.layout.read(cx);
                let source_group = floating_group_for_panel(&layout, panel_id);
                let target_group = floating_group_for_node(&layout, target_node);
                source_group.is_some() && target_group.is_some() && source_group != target_group
            };
            if joins_another_floating_group {
                self.dock_floating_panel_to_composition(
                    panel_id,
                    native_window,
                    DropTarget::GroupEdge {
                        group: target_node,
                        zone,
                    },
                    cx,
                );
                cx.notify();
                return false;
            }
        }

        let metrics = self.metrics;
        let docked = match request {
            Some(request) => match self.layout.update(cx, |layout, layout_cx| {
                let action = layout.resolve_drag_drop(&request, &metrics)?;
                match action {
                    DragDropAction::NoOp => Ok::<bool, dock_core::CommandError>(false),
                    DragDropAction::Command(command) => {
                        layout.apply(command, &metrics)?;
                        layout_cx.notify();
                        Ok::<bool, dock_core::CommandError>(
                            floating_group_for_panel(layout, panel_id).is_none()
                                && main_tabs_group_for_panel(layout, panel_id).is_some(),
                        )
                    }
                }
            }) {
                Ok(docked) => docked,
                Err(error) => {
                    eprintln!("Could not apply the panel drop: {error}");
                    false
                }
            },
            None => false,
        };

        if docked {
            if let Some(binding) = self
                .panel_bindings
                .iter_mut()
                .find(|binding| binding.panel_id == panel_id)
            {
                binding.floating_group_id = None;
            }
            if let Some(panel) = self
                .floating_panels
                .iter_mut()
                .find(|panel| panel.panel_id == panel_id)
            {
                panel.visible = false;
            }
            platform::windows::hide_window(native_window);
        } else if was_floating {
            self.sync_floating_panel_after_drag(panel_id, cx);
        }

        cx.notify();
        docked
    }

    #[cfg(windows)]
    fn dock_floating_panel_to_composition(
        &mut self,
        source_panel_id: PanelId,
        source_native: NativeWindowHandle,
        target: DropTarget,
        cx: &mut Context<Self>,
    ) -> bool {
        let DropTarget::GroupEdge {
            group: target_node,
            zone: zone @ (DropZone::Left | DropZone::Right),
        } = target
        else {
            return false;
        };

        let composition_members = {
            let layout = self.layout.read(cx);
            let Some(source_group) = floating_group_for_panel(&layout, source_panel_id) else {
                return false;
            };
            let Some(target_group) = floating_group_for_node(&layout, target_node) else {
                return false;
            };
            if source_group == target_group {
                return false;
            }
            Some((
                source_group,
                target_group,
                floating_group_panel_ids(&layout, source_group),
                floating_group_panel_ids(&layout, target_group),
            ))
        };
        let Some((source_group, target_group, source_panel_ids, target_panel_ids)) =
            composition_members
        else {
            self.sync_floating_panel_after_drag(source_panel_id, cx);
            return false;
        };

        let target_host = self
            .floating_panels
            .iter()
            .find(|panel| {
                panel.host && panel.visible && target_panel_ids.contains(&panel.panel_id)
            })
            .or_else(|| {
                self.floating_panels.iter().find(|panel| {
                    panel.visible && target_panel_ids.contains(&panel.panel_id)
                })
            })
            .map(|panel| (panel.panel_id, panel.handle.clone()));
        let Some((target_host_panel_id, target_handle)) = target_host else {
            eprintln!("The target floating composition has no visible popup host");
            self.sync_floating_panel_after_drag(source_panel_id, cx);
            return false;
        };

        let target_native = match target_handle.update(cx, |_, window, _| {
            NativeWindowHandle::from_window(window)
        }) {
            Ok(Some(handle)) => handle,
            Ok(None) => {
                eprintln!("Could not obtain the target composition's native handle");
                self.sync_floating_panel_after_drag(source_panel_id, cx);
                return false;
            }
            Err(error) => {
                eprintln!("Could not access the target composition window: {error:?}");
                self.sync_floating_panel_after_drag(source_panel_id, cx);
                return false;
            }
        };
        let Some((source_left, source_top, source_right, source_bottom)) =
            platform::windows::window_rect(source_native)
        else {
            eprintln!("Could not read the dragged composition window bounds");
            self.sync_floating_panel_after_drag(source_panel_id, cx);
            return false;
        };
        let Some((target_left, target_top, target_right, target_bottom)) =
            platform::windows::window_rect(target_native)
        else {
            eprintln!("Could not read the destination composition window bounds");
            self.sync_floating_panel_after_drag(source_panel_id, cx);
            return false;
        };

        if let Err(error) = self.apply_layout_command(
            DockCommand::DockFloating {
                group: source_group,
                target,
            },
            cx,
        ) {
            eprintln!("Could not join the floating compositions: {error}");
            self.sync_floating_panel_after_drag(source_panel_id, cx);
            return false;
        }

        for binding in &mut self.panel_bindings {
            if source_panel_ids.contains(&binding.panel_id)
                || target_panel_ids.contains(&binding.panel_id)
            {
                binding.floating_group_id = Some(target_group);
            }
        }
        for panel in &mut self.floating_panels {
            if source_panel_ids.contains(&panel.panel_id) {
                panel.visible = false;
                panel.host = false;
            } else if target_panel_ids.contains(&panel.panel_id) {
                panel.host = panel.panel_id == target_host_panel_id;
                panel.visible = panel.panel_id == target_host_panel_id;
            }
        }

        let source_width = (source_right - source_left).max(1);
        let source_height = (source_bottom - source_top).max(1);
        let target_width = (target_right - target_left).max(1);
        let target_height = (target_bottom - target_top).max(1);
        let new_left = if zone == DropZone::Left {
            target_left - source_width
        } else {
            target_left
        };
        let new_width = target_width.saturating_add(source_width);
        let new_height = target_height.max(source_height);

        if !platform::windows::move_window(target_native, new_left, target_top) {
            eprintln!("Could not reposition the floating composition host");
        }
        if !platform::windows::resize_window(target_native, new_width, new_height) {
            eprintln!("Could not resize the floating composition host");
        }
        platform::windows::hide_window(source_native);

        if let Ok((bounds, viewport_size)) = target_handle.update(cx, |_, window, _| {
            (window.window_bounds().get_bounds(), window.viewport_size())
        }) {
            if let Err(error) =
                self.sync_floating_panel_bounds(target_group, bounds, viewport_size, cx)
            {
                eprintln!("Could not synchronize the joined composition bounds: {error}");
            }
        }

        if let Err(error) = target_handle.update(cx, |_, window, _| window.activate_window()) {
            eprintln!("Could not activate the joined composition window: {error:?}");
        }

        cx.notify();
        true
    }

    #[cfg(windows)]
    fn sync_floating_panel_after_drag(&mut self, panel_id: PanelId, cx: &mut Context<Self>) {
        let group = {
            let layout = self.layout.read(cx);
            floating_group_for_panel(&layout, panel_id)
        };
        let handle = self
            .floating_panels
            .iter()
            .find(|panel| panel.panel_id == panel_id)
            .map(|panel| panel.handle.clone());

        let (Some(group), Some(handle)) = (group, handle) else {
            return;
        };

        match handle.update(cx, |_, window, _| {
            (window.window_bounds().get_bounds(), window.viewport_size())
        }) {
            Ok((bounds, viewport_size)) => {
                if let Err(error) =
                    self.sync_floating_panel_bounds(group, bounds, viewport_size, cx)
                {
                    eprintln!("Could not sync the floating panel after dragging: {error}");
                }
            }
            Err(error) => {
                eprintln!("Could not read floating panel bounds after dragging: {error:?}");
            }
        }
    }

    #[cfg(windows)]
    fn external_panel_pointer_from_screen(&self, screen_x: i32, screen_y: i32) -> Option<Point> {
        let main_window = self.native_owner?;
        let (client_x, client_y) = platform::windows::client_origin(main_window)?;
        let scale = platform::windows::scale_factor(main_window).max(0.01);
        let pointer = Point::new(
            (screen_x - client_x) as f32 / scale,
            (screen_y - client_y) as f32 / scale - self.last_surface_top,
        );
        pointer.is_finite().then_some(pointer)
    }

    /// Closes every still-open panel before GPUI closes the main window.
    pub(crate) fn close_floating_panels(&mut self, cx: &mut Context<Self>) {
        self.panel_drag = None;
        self.panels_minimized_by_main.clear();

        for panel in self.floating_panels.drain(..) {
            match panel
                .handle
                .update(cx, |_, window, _| window.remove_window())
            {
                Ok(()) => {}
                Err(_error) => {
                    // If the user already closed this panel, cleanup is already complete.
                }
            }
        }
    }
}

fn menu_button(kind: PanelKind, owner: WeakEntity<MainWindow>) -> impl IntoElement {
    div()
        .id(kind.key())
        .px_3()
        .py_2()
        .bg(rgb(0x394451))
        .text_color(rgb(0xf0f2f5))
        .rounded_sm()
        .cursor_pointer()
        .child(kind.title())
        .on_click(move |_, _, cx| {
            if let Err(error) = owner.update(cx, |this, cx| {
                let _ = this.open_floating_panel(kind, true, cx);
            }) {
                eprintln!("Could not access the main window to open a panel: {error:?}");
            }
        })
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.visibility_subscription.is_none() {
            self.visibility_subscription = Some(cx.observe_window_visibility(
                window,
                |this, visibility, _window, cx| {
                    this.handle_main_window_visibility(visibility, cx);
                },
            ));
        }

        if self.layout_subscription.is_none() {
            let layout = self.layout.clone();
            self.layout_subscription =
                Some(cx.observe(&layout, |_, _, cx| cx.notify()));
        }

        let window_size = window.viewport_size();
        let width = window_size.width.as_f32().max(1.0);
        let height = window_size.height.as_f32().max(1.0);
        let surface_top = 48.0;
        let content_height = (height - surface_top).max(1.0);
        let viewport = Rect::new(Point::ZERO, Size::new(width, content_height));
        self.last_dock_viewport = Some(viewport);
        self.last_window_bounds = Some(window.window_bounds().get_bounds());
        self.last_viewport_size = Some(window_size);
        self.last_surface_top = surface_top;

        let owner = cx.entity().downgrade();
        let layout = self.layout.read(cx);
        let left_dock = render_main_dock(
            &layout,
            DockSide::Left,
            self.metrics,
            owner.clone(),
        );
        let right_dock = render_main_dock(
            &layout,
            DockSide::Right,
            self.metrics,
            owner.clone(),
        );

        let mut workspace = div()
            .relative()
            .flex()
            .flex_row()
            .flex_1()
            .size_full()
            .bg(rgb(0x20252b));

        if let Some(left_dock) = left_dock {
            workspace = workspace.child(left_dock);
        }

        workspace = workspace.child(
            div()
                .flex_1()
                .size_full()
                .bg(rgb(0x20252b)),
        );

        if let Some(right_dock) = right_dock {
            workspace = workspace.child(right_dock);
        }

        if let Some(drag) = self.panel_drag.as_ref()
            && drag.resolver.is_some()
        {
            if let Some(zone) = drag
                .target
                .and_then(|target| {
                    drag.zones.iter().find(|zone| {
                        zone.target == target
                            && zone.zone == DropZone::DockEdge
                            && zone.rect.contains(drag.pointer).unwrap_or(false)
                    })
                })
            {
                workspace = workspace.child(
                    div()
                        .absolute()
                        .left(px(zone.rect.origin.x))
                        .top(px(zone.rect.origin.y))
                        .w(px(zone.rect.size.width))
                        .h(px(zone.rect.size.height))
                        .bg(rgb(0x286b9e))
                        .border_2()
                        .border_color(rgb(0x69c3ff)),
                );
            }
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x20252b))
            .text_color(rgb(0xe8ebef))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .h(px(48.0))
                    .px_3()
                    .bg(rgb(0x2b323a))
                    .border_b_1()
                    .border_color(rgb(0x414a55))
                    .child("Панели")
                    .child(menu_button(PanelKind::Layers, owner.clone()))
                    .child(menu_button(PanelKind::Properties, owner.clone())),
            )
            .child(workspace)
    }
}

struct FloatingPanel {
    panel_id: PanelId,
    owner: WeakEntity<MainWindow>,
    layout: Entity<Layout>,
    layout_subscription: Option<Subscription>,
    bounds_subscription: Option<Subscription>,
    drop_highlight: Option<Rect>,
}

impl FloatingPanel {
    fn sync_native_bounds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let native_bounds = window.window_bounds().get_bounds();
        let viewport_size = window.viewport_size();
        let group = {
            let layout = self.layout.read(cx);
            floating_group_for_panel(&layout, self.panel_id)
        };
        let Some(group) = group else {
            return;
        };

        match self.owner.update(cx, |main, cx| {
            main.sync_floating_panel_bounds(group, native_bounds, viewport_size, cx)
        }) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                eprintln!("Could not synchronize floating panel bounds: {error}");
            }
            Err(error) => {
                eprintln!("Could not access the dock layout for floating bounds: {error:?}");
            }
        }
    }
}

impl Render for FloatingPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.bounds_subscription.is_none() {
            self.bounds_subscription =
                Some(cx.observe_window_bounds(window, |this, window, cx| {
                    this.sync_native_bounds(window, cx);
                }));
        }

        if self.layout_subscription.is_none() {
            let layout = self.layout.clone();
            self.layout_subscription =
                Some(cx.observe(&layout, |_, _, cx| cx.notify()));
        }

        let layout = self.layout.read(cx);
        let floating_group = floating_group_for_panel(&layout, self.panel_id);
        let content = floating_group
            .and_then(|group| render_floating_group(&layout, group, self.owner.clone()))
            .unwrap_or_else(|| {
                div()
                    .flex_1()
                    .p_4()
                    .text_color(rgb(0xf08a8a))
                    .child("Дерево панели не найдено")
                    .into_any_element()
            });

        let drop_highlight = self.drop_highlight;

        #[cfg(windows)]
        let owner = self.owner.clone();
        #[cfg(windows)]
        let panel_id = self.panel_id;
        let mut root = div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x292f37))
            .text_color(rgb(0xe8ebef))
            .child(content);

        if let Some(zone) = drop_highlight {
            root = root.child(
                div()
                    .absolute()
                    .left(px(zone.origin.x))
                    .top(px(zone.origin.y))
                    .w(px(zone.size.width))
                    .h(px(zone.size.height))
                    .bg(rgb(0x286b9e))
                    .border_2()
                    .border_color(rgb(0x69c3ff)),
            );
        }

        let drag_zone = div()
            .id("composition-drag-zone")
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .w_full()
            .h(px(8.0));

        #[cfg(windows)]
        let drag_zone = {
            let drag_owner = owner;
            drag_zone
                .occlude()
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    let Some(native_window) = NativeWindowHandle::from_window(window) else {
                        eprintln!("Could not obtain the composition window handle for dragging");
                        return;
                    };
                    if let Err(error) = drag_owner.update(cx, |main, cx| {
                        main.begin_floating_panel_drag(panel_id, native_window, cx);
                    }) {
                        eprintln!("Could not begin composition dragging: {error:?}");
                    }
                })
        };

        root.child(drag_zone)
    }
}

#[cfg(windows)]
fn close_floating_composition(
    owner: WeakEntity<MainWindow>,
    panel_id: PanelId,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(native_window) = NativeWindowHandle::from_window(window) {
        platform::windows::hide_window(native_window);
    }

    let main_window = owner.update(cx, |this, cx| {
        let composition_panel_ids = {
            let layout = this.layout.read(cx);
            floating_group_for_panel(&layout, panel_id)
                .map(|group| floating_group_panel_ids(&layout, group))
                .unwrap_or_else(|| vec![panel_id])
        };
        for panel in this
            .floating_panels
            .iter_mut()
            .filter(|panel| composition_panel_ids.contains(&panel.panel_id))
        {
            panel.visible = false;
        }
        this.main_window_handle
    });

    match main_window {
        Ok(Some(main_window)) => {
            if let Err(error) =
                main_window.update(cx, |_, main_window, _| main_window.activate_window())
            {
                eprintln!("Could not activate the main window after hiding a panel: {error:?}");
            }
        }
        Ok(None) => {
            eprintln!("Could not activate the main window after hiding a panel: handle unavailable");
        }
        Err(error) => {
            eprintln!("Could not access the main window while hiding a panel: {error:?}");
        }
    }
}

fn collect_subtree_nodes(layout: &Layout, node_id: NodeId, nodes: &mut HashSet<NodeId>) {
    if !nodes.insert(node_id) {
        return;
    }

    match layout.node(node_id) {
        Some(Node::Split(split)) => {
            for child in &split.children {
                collect_subtree_nodes(layout, *child, nodes);
            }
        }
        Some(Node::Column(column)) => {
            for child in &column.groups {
                collect_subtree_nodes(layout, *child, nodes);
            }
        }
        Some(Node::Tabs(tabs)) => {
            for child in &tabs.children {
                collect_subtree_nodes(layout, *child, nodes);
            }
        }
        Some(Node::Panel(_)) | None => {}
    }
}

fn prepare_external_panel_resolver(
    layout: &Layout,
    viewport: Rect,
    metrics: DockMetrics,
    panel_id: PanelId,
    visible_floating_groups: &HashSet<FloatingGroupId>,
) -> Result<(DropZoneResolver, Vec<DropZoneRect>), String> {
    let mut rects = layout
        .compute_rects(viewport, &metrics)
        .map_err(|error| format!("could not compute main dock geometry: {error}"))?;

    let source_group = floating_group_for_panel(layout, panel_id);
    let source_is_composition = source_group
        .and_then(|group| floating_group_root(layout, group))
        .is_some_and(|root| {
            matches!(layout.node(root), Some(Node::Split(split)) if split.axis == Axis::X)
        });
    let mut peer_zones = Vec::new();
    for floating_rect in rects.floating_groups.iter().copied() {
        if Some(floating_rect.group) == source_group
            || !visible_floating_groups.contains(&floating_rect.group)
        {
            continue;
        }
        let Some(root) = floating_group_root(layout, floating_rect.group) else {
            continue;
        };
        for zone in [DropZone::Left, DropZone::Right] {
            let Some(target_group) = floating_edge_group(layout, root, zone) else {
                continue;
            };
            let Some(rect) = floating_group_edge_rect(
                floating_rect.rect,
                zone,
                metrics.edge_drop_width,
            ) else {
                continue;
            };
            peer_zones.push(DropZoneRect::new(
                DropTarget::GroupEdge {
                    group: target_group,
                    zone,
                },
                zone,
                rect,
            ));
        }
    }

    let mut floating_nodes = HashSet::new();
    for (_, floating) in layout.floating_groups() {
        if let Some(area) = layout.dock_area(floating.area) {
            collect_subtree_nodes(layout, area.root, &mut floating_nodes);
        }
    }
    let floating_panels = floating_nodes
        .iter()
        .filter_map(|node| match layout.node(*node) {
            Some(Node::Panel(panel)) => Some(panel.panel_id),
            _ => None,
        })
        .collect::<HashSet<_>>();

    // Native floating windows are not part of the visible main-surface target map.
    rects.columns.retain(|entry| !floating_nodes.contains(&entry.column));
    rects.groups.retain(|entry| !floating_nodes.contains(&entry.group));
    rects.tab_strips.retain(|entry| !floating_nodes.contains(&entry.group));
    rects.tab_headers.retain(|entry| !floating_nodes.contains(&entry.group));
    rects.tab_overflows.retain(|entry| !floating_nodes.contains(&entry.group));
    rects.panels.retain(|entry| !floating_panels.contains(&entry.panel));
    rects.splitters.retain(|entry| !floating_nodes.contains(&entry.owner));
    rects
        .vertical_overflow
        .retain(|entry| !floating_nodes.contains(&entry.owner));
    rects.floating_groups.clear();
    rects.floating_resize_edges.clear();
    rects.drop_zones.clear();

    let resolver = layout
        .prepare_drop_resolver(DropSource::Panel { panel: panel_id }, &rects, &metrics)
        .map_err(|error| format!("could not prepare dock-core drop zones: {error}"))?;
    let mut zones = resolver
        .zones()
        .iter()
        .copied()
        .filter(|zone| zone.zone == DropZone::DockEdge && !source_is_composition)
        .collect::<Vec<_>>();
    zones.extend(peer_zones);

    Ok((resolver, zones))
}

fn resolve_edge_drop_target(
    resolver: &DropZoneResolver,
    zones: &[DropZoneRect],
    pointer: Point,
) -> Option<DropTarget> {
    if let Some(zone) = zones.iter().find(|zone| {
        matches!(zone.zone, DropZone::Left | DropZone::Right)
            && zone.rect.contains(pointer).unwrap_or(false)
    }) {
        return Some(zone.target);
    }

    let target = resolver.resolve(pointer).ok().flatten()?;
    zones
        .iter()
        .find(|zone| zone.target == target && zone.rect.contains(pointer).unwrap_or(false))
        .map(|zone| zone.target)
}

fn node_contains_panel(layout: &Layout, node_id: NodeId, panel_id: PanelId) -> bool {
    match layout.node(node_id) {
        Some(Node::Panel(panel)) => panel.panel_id == panel_id,
        Some(Node::Column(column)) => column
            .groups
            .iter()
            .any(|child| node_contains_panel(layout, *child, panel_id)),
        Some(Node::Tabs(tabs)) => tabs
            .children
            .iter()
            .any(|child| node_contains_panel(layout, *child, panel_id)),
        Some(Node::Split(split)) => split
            .children
            .iter()
            .any(|child| node_contains_panel(layout, *child, panel_id)),
        None => false,
    }
}

fn floating_group_for_panel(layout: &Layout, panel_id: PanelId) -> Option<FloatingGroupId> {
    layout.floating_groups().find_map(|(group_id, floating)| {
        let root = layout.dock_area(floating.area)?.root;
        node_contains_panel(layout, root, panel_id).then_some(group_id)
    })
}

fn floating_group_root(layout: &Layout, group_id: FloatingGroupId) -> Option<NodeId> {
    let floating = layout.floating_group(group_id)?;
    layout.dock_area(floating.area).map(|area| area.root)
}

fn node_contains_node(layout: &Layout, root: NodeId, target: NodeId) -> bool {
    if root == target {
        return true;
    }
    match layout.node(root) {
        Some(Node::Column(column)) => column
            .groups
            .iter()
            .any(|child| node_contains_node(layout, *child, target)),
        Some(Node::Tabs(tabs)) => tabs
            .children
            .iter()
            .any(|child| node_contains_node(layout, *child, target)),
        Some(Node::Split(split)) => split
            .children
            .iter()
            .any(|child| node_contains_node(layout, *child, target)),
        Some(Node::Panel(_)) | None => false,
    }
}

fn floating_group_for_node(layout: &Layout, node_id: NodeId) -> Option<FloatingGroupId> {
    layout.floating_groups().find_map(|(group_id, floating)| {
        let root = layout.dock_area(floating.area)?.root;
        node_contains_node(layout, root, node_id).then_some(group_id)
    })
}

fn collect_subtree_panel_ids(layout: &Layout, node_id: NodeId, panels: &mut Vec<PanelId>) {
    match layout.node(node_id) {
        Some(Node::Panel(panel)) => panels.push(panel.panel_id),
        Some(Node::Column(column)) => {
            for child in &column.groups {
                collect_subtree_panel_ids(layout, *child, panels);
            }
        }
        Some(Node::Tabs(tabs)) => {
            for child in &tabs.children {
                collect_subtree_panel_ids(layout, *child, panels);
            }
        }
        Some(Node::Split(split)) => {
            for child in &split.children {
                collect_subtree_panel_ids(layout, *child, panels);
            }
        }
        None => {}
    }
}

fn floating_group_panel_ids(layout: &Layout, group_id: FloatingGroupId) -> Vec<PanelId> {
    let Some(root) = floating_group_root(layout, group_id) else {
        return Vec::new();
    };
    let mut panels = Vec::new();
    collect_subtree_panel_ids(layout, root, &mut panels);
    panels
}

fn floating_edge_group(layout: &Layout, root: NodeId, zone: DropZone) -> Option<NodeId> {
    let column_id = match layout.node(root)? {
        Node::Column(_) => root,
        Node::Split(split) if split.axis == Axis::X => {
            if zone == DropZone::Left {
                *split.children.first()?
            } else if zone == DropZone::Right {
                *split.children.last()?
            } else {
                return None;
            }
        }
        Node::Tabs(_) => return Some(root),
        Node::Split(_) | Node::Panel(_) => return None,
    };
    match layout.node(column_id)? {
        Node::Column(column) => column.groups.first().copied(),
        _ => None,
    }
}

fn floating_group_edge_rect(
    bounds: Rect,
    zone: DropZone,
    edge_width: f32,
) -> Option<Rect> {
    let x_band = edge_width.min(bounds.size.width / 3.0);
    let y_band = edge_width.min(bounds.size.height / 3.0);
    let side_height = (bounds.size.height - 2.0 * y_band).max(0.0);
    let rect = match zone {
        DropZone::Left => Rect::new(
            Point::new(bounds.origin.x, bounds.origin.y + y_band),
            Size::new(x_band, side_height),
        ),
        DropZone::Right => Rect::new(
            Point::new(
                bounds.origin.x + bounds.size.width - x_band,
                bounds.origin.y + y_band,
            ),
            Size::new(x_band, side_height),
        ),
        _ => return None,
    };
    rect.validate_non_empty().is_ok().then_some(rect)
}

fn tabs_group_for_panel(
    layout: &Layout,
    node_id: NodeId,
    panel_id: PanelId,
) -> Option<NodeId> {
    match layout.node(node_id)? {
        Node::Tabs(tabs) => tabs
            .children
            .iter()
            .any(|child| node_contains_panel(layout, *child, panel_id))
            .then_some(node_id),
        Node::Column(column) => column
            .groups
            .iter()
            .find_map(|child| tabs_group_for_panel(layout, *child, panel_id)),
        Node::Split(split) => split
            .children
            .iter()
            .find_map(|child| tabs_group_for_panel(layout, *child, panel_id)),
        Node::Panel(_) => None,
    }
}

#[cfg(windows)]
fn column_for_panel(layout: &Layout, node_id: NodeId, panel_id: PanelId) -> Option<NodeId> {
    match layout.node(node_id)? {
        Node::Column(column) => column
            .groups
            .iter()
            .any(|group| node_contains_panel(layout, *group, panel_id))
            .then_some(node_id),
        Node::Tabs(tabs) => tabs
            .children
            .iter()
            .find_map(|child| column_for_panel(layout, *child, panel_id)),
        Node::Split(split) => split
            .children
            .iter()
            .find_map(|child| column_for_panel(layout, *child, panel_id)),
        Node::Panel(_) => None,
    }
}

#[cfg(windows)]
fn main_column_for_panel(layout: &Layout, panel_id: PanelId) -> Option<NodeId> {
    [DockSide::Left, DockSide::Right]
        .into_iter()
        .filter_map(|side| layout.main_area(side).map(|area| area.root))
        .find_map(|root| column_for_panel(layout, root, panel_id))
}

fn main_tabs_group_for_panel(layout: &Layout, panel_id: PanelId) -> Option<NodeId> {
    [DockSide::Left, DockSide::Right]
        .into_iter()
        .filter_map(|side| layout.main_area(side).map(|area| area.root))
        .find_map(|root| tabs_group_for_panel(layout, root, panel_id))
}

fn main_area_width(layout: &Layout, side: DockSide, metrics: DockMetrics) -> Option<f32> {
    let area = layout.main_area(side)?;
    match layout.node(area.root)? {
        Node::Column(column) => Some(column.width),
        Node::Split(split) if split.axis == Axis::X => {
            let mut width = 0.0;
            for child in &split.children {
                let Some(Node::Column(column)) = layout.node(*child) else {
                    return None;
                };
                width += column.width;
            }
            width += metrics.splitter_thickness * split.children.len().saturating_sub(1) as f32;
            Some(width)
        }
        _ => None,
    }
}

fn render_main_dock(
    layout: &Layout,
    side: DockSide,
    metrics: DockMetrics,
    owner: WeakEntity<MainWindow>,
) -> Option<AnyElement> {
    let root = layout.main_area(side)?.root;
    let width = main_area_width(layout, side, metrics).unwrap_or(metrics.default_column_width);
    let content = render_core_node(layout, root, owner, true, true, false);

    let dock = div()
        .flex()
        .flex_col()
        .h_full()
        .w(px(width))
        .bg(rgb(0x262c33));

    let dock = match side {
        DockSide::Left => dock.border_r_1().border_color(rgb(0x414a55)),
        DockSide::Right => dock.border_l_1().border_color(rgb(0x414a55)),
    };

    Some(dock.child(content).into_any_element())
}

fn render_floating_group(
    layout: &Layout,
    group: FloatingGroupId,
    owner: WeakEntity<MainWindow>,
) -> Option<AnyElement> {
    let root = layout
        .floating_group(group)
        .and_then(|floating| layout.dock_area(floating.area))
        .map(|area| area.root)?;

    Some(render_core_node(layout, root, owner, true, false, true))
}

fn render_core_node(
    layout: &Layout,
    node_id: NodeId,
    owner: WeakEntity<MainWindow>,
    show_column_headers: bool,
    allow_column_header_drag: bool,
    floating_column_headers: bool,
) -> AnyElement {
    let Some(node) = layout.node(node_id) else {
        return div()
            .p_2()
            .text_color(rgb(0xf08a8a))
            .child("Узел дерева не найден")
            .into_any_element();
    };

    match node {
        Node::Column(column) => {
            let title = column
                .groups
                .first()
                .and_then(|group| core_node_active_panel_title(layout, *group))
                .unwrap_or_else(|| "Колонка".to_owned());

            if floating_column_headers {
                #[cfg(windows)]
                let panel_id = column
                    .groups
                    .first()
                    .copied()
                    .and_then(|group| core_node_active_panel_id(layout, group));
                let drag_area = div()
                    .id(format!("floating-column-title-{node_id:?}"))
                    .flex()
                    .items_center()
                    .flex_1()
                    .h_full()
                    .px_2()
                    .child(title);

                #[cfg(windows)]
                let drag_area = {
                    let drag_owner = owner.clone();
                    drag_area
                        .occlude()
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            let Some(panel_id) = panel_id else {
                                return;
                            };
                            let Some(native_window) = NativeWindowHandle::from_window(window)
                            else {
                                eprintln!(
                                    "Could not obtain the composition window handle for dragging"
                                );
                                return;
                            };
                            if let Err(error) = drag_owner.update(cx, |main, cx| {
                                main.begin_floating_panel_drag(panel_id, native_window, cx);
                            }) {
                                eprintln!(
                                    "Could not begin composition dragging from the column header: {error:?}"
                                );
                            }
                        })
                };

                let close_button = div()
                    .id(format!("close-floating-column-{node_id:?}"))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(28.0))
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x596777)))
                    .occlude()
                    .child("×");

                #[cfg(windows)]
                let close_button = {
                    let close_owner = owner.clone();
                    close_button.on_click(move |_, window, cx| {
                        if let Some(panel_id) = panel_id {
                            close_floating_composition(
                                close_owner.clone(),
                                panel_id,
                                window,
                                cx,
                            );
                        }
                    })
                };

                #[cfg(not(windows))]
                let close_button =
                    close_button.on_click(|_, window, _| window.remove_window());

                let header = div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .h(px(36.0))
                    .px_2()
                    .bg(rgb(0x2b323a))
                    .border_b_1()
                    .border_color(rgb(0x414a55))
                    .child(drag_area)
                    .child(close_button);

                let content = div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .w_full()
                    .gap_2()
                    .p_2()
                    .bg(rgb(0x262c33))
                    .children(column.groups.iter().map(|child| {
                        render_core_node(
                            layout,
                            *child,
                            owner.clone(),
                            show_column_headers,
                            allow_column_header_drag,
                            floating_column_headers,
                        )
                    }));

                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .w_full()
                    .bg(rgb(0x262c33))
                    .child(header)
                    .child(content)
                    .into_any_element()
            } else {
                let mut column_view = div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .w_full()
                    .gap_2()
                    .p_2()
                    .bg(rgb(0x262c33));

                if show_column_headers {
                    let header = div()
                        .flex()
                        .items_center()
                        .w_full()
                        .h(px(36.0))
                        .px_3()
                        .bg(rgb(0x2b323a))
                        .border_b_1()
                        .border_color(rgb(0x414a55))
                        .text_color(rgb(0xe8ebef))
                        .child(title);

                    #[cfg(windows)]
                    let header = if allow_column_header_drag {
                        let drag_source = column
                            .groups
                            .first()
                            .copied()
                            .and_then(|group| {
                                core_node_active_panel_id(layout, group)
                                    .map(|panel_id| (group, panel_id))
                            });
                        let drag_owner = owner.clone();
                        header.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            let Some((group, panel_id)) = drag_source else {
                                return;
                            };
                            if let Err(error) = drag_owner.update(cx, |main, cx| {
                                main.begin_docked_panel_drag(group, panel_id, cx);
                            }) {
                                eprintln!("Could not begin a column-header panel drag: {error:?}");
                            }
                        })
                    } else {
                        header
                    };

                    column_view = column_view.child(header);
                }

                column_view
                    .children(column.groups.iter().map(|child| {
                        render_core_node(
                            layout,
                            *child,
                            owner.clone(),
                            show_column_headers,
                            allow_column_header_drag,
                            floating_column_headers,
                        )
                    }))
                    .into_any_element()
            }
        },
        Node::Tabs(tabs) => {
            let tab_headers = tabs.children.iter().enumerate().map(|(index, child)| {
                let title = core_node_title(layout, *child);
                let active = index == tabs.active_index;
                let panel_id = match layout.node(*child) {
                    Some(Node::Panel(panel)) => Some(panel.panel_id),
                    _ => None,
                };

                let mut tab = div()
                    .id(format!("core-tab-{node_id:?}-{index}"))
                    .px_3()
                    .py_1()
                    .rounded_sm()
                    .bg(if active {
                        rgb(0x465260)
                    } else {
                        rgb(0x303841)
                    })
                    .text_color(if active {
                        rgb(0xf0f2f5)
                    } else {
                        rgb(0xaab4c0)
                    })
                    .child(title);

                if let Some(panel_id) = panel_id {
                    let tab_owner = owner.clone();
                    tab = tab
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(0x596777)))
                        .on_click(move |_, _, cx| {
                            match tab_owner.update(cx, |this, cx| {
                                this.apply_layout_command(
                                    DockCommand::ActivateTab {
                                        group: node_id,
                                        panel: panel_id,
                                    },
                                    cx,
                                )
                            }) {
                                Ok(Ok(())) => {}
                                Ok(Err(error)) => {
                                    eprintln!("Could not activate dock tab: {error}");
                                }
                                Err(error) => {
                                    eprintln!("Could not access the dock layout: {error:?}");
                                }
                            }
                        });

                    #[cfg(windows)]
                    {
                        let drag_owner = owner.clone();
                        tab = tab.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            if let Err(error) = drag_owner.update(cx, |main, cx| {
                                main.begin_docked_panel_drag(node_id, panel_id, cx);
                            }) {
                                eprintln!("Could not begin a docked panel drag: {error:?}");
                            }
                        });
                    }
                }

                tab
            });

            let header = div()
                .flex()
                .items_center()
                .gap_1()
                .w_full()
                .px_2()
                .py_2()
                .bg(rgb(0x2b323a))
                .children(tab_headers);

            let content = tabs
                .active_child()
                .map(|child| {
                    render_core_node(
                        layout,
                        child,
                        owner.clone(),
                        show_column_headers,
                        allow_column_header_drag,
                        floating_column_headers,
                    )
                })
                .unwrap_or_else(|| {
                    div()
                        .flex_1()
                        .p_3()
                        .text_color(rgb(0xaab4c0))
                        .child("Пустая группа")
                        .into_any_element()
                });

            div()
                .flex()
                .flex_col()
                .flex_1()
                .w_full()
                .bg(rgb(0x292f37))
                .border_b_1()
                .border_color(rgb(0x414a55))
                .child(header)
                .child(content)
                .into_any_element()
        }
        Node::Panel(panel) => {
            let title = layout
                .panel_instance(panel.panel_id)
                .map(|instance| panel_title(instance.key.as_str()))
                .unwrap_or("Неизвестная панель");

            div()
                .flex()
                .flex_col()
                .flex_1()
                .w_full()
                .p_3()
                .text_color(rgb(0xe8ebef))
                .child(title.to_owned())
                .into_any_element()
        }
        Node::Split(split) => {
            let container = match split.axis {
                Axis::X => div().flex().flex_row(),
                Axis::Y => div().flex().flex_col(),
            };

            container
                .flex_1()
                .size_full()
                .gap_2()
                .children(
                    split
                        .children
                        .iter()
                        .enumerate()
                        .map(|(index, child)| {
                            let ratio = split.ratios.get(index).copied().unwrap_or(1.0);
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .flex_grow(ratio)
                                .child(render_core_node(
                                    layout,
                                    *child,
                                    owner.clone(),
                                    show_column_headers,
                                    allow_column_header_drag,
                                    floating_column_headers,
                                ))
                        }),
                )
                .into_any_element()
        }
    }
}

#[cfg(windows)]
fn core_node_active_panel_id(layout: &Layout, node_id: NodeId) -> Option<PanelId> {
    match layout.node(node_id)? {
        Node::Panel(panel) => Some(panel.panel_id),
        Node::Tabs(tabs) => tabs
            .active_child()
            .and_then(|child| core_node_active_panel_id(layout, child)),
        Node::Column(column) => column
            .groups
            .first()
            .and_then(|child| core_node_active_panel_id(layout, *child)),
        Node::Split(split) => split
            .children
            .first()
            .and_then(|child| core_node_active_panel_id(layout, *child)),
    }
}

fn core_node_active_panel_title(layout: &Layout, node_id: NodeId) -> Option<String> {
    match layout.node(node_id)? {
        Node::Panel(panel) => layout
            .panel_instance(panel.panel_id)
            .map(|instance| panel_title(instance.key.as_str()).to_owned()),
        Node::Tabs(tabs) => tabs
            .active_child()
            .and_then(|child| core_node_active_panel_title(layout, child)),
        Node::Column(column) => column
            .groups
            .first()
            .and_then(|child| core_node_active_panel_title(layout, *child)),
        Node::Split(split) => split
            .children
            .first()
            .and_then(|child| core_node_active_panel_title(layout, *child)),
    }
}

fn core_node_title(layout: &Layout, node_id: NodeId) -> String {
    match layout.node(node_id) {
        Some(Node::Panel(panel)) => layout
            .panel_instance(panel.panel_id)
            .map(|instance| panel_title(instance.key.as_str()).to_owned())
            .unwrap_or_else(|| "Неизвестная панель".to_owned()),
        Some(Node::Tabs(_)) => "Группа".to_owned(),
        Some(Node::Column(_)) => "Колонка".to_owned(),
        Some(Node::Split(_)) => "Разделение".to_owned(),
        None => "Неизвестный узел".to_owned(),
    }
}


#[cfg(windows)]
fn build_initial_layout(
    metrics: DockMetrics,
    window_bounds: [Bounds<Pixels>; 2],
) -> Result<(Layout, [PanelBinding; 2]), String> {
    let [layers_bounds, properties_bounds] = window_bounds;
    let mut layout = Layout::new(Profile::Photoshop);

    let layers = create_floating_panel_tree(
        &mut layout,
        PanelKind::Layers,
        DockSide::Left,
        layers_bounds,
        &metrics,
    )?;
    let properties = create_floating_panel_tree(
        &mut layout,
        PanelKind::Properties,
        DockSide::Right,
        properties_bounds,
        &metrics,
    )?;

    layout
        .validate(&metrics)
        .map_err(|errors| format!("dock-core rejected the initial panel trees: {errors:?}"))?;

    Ok((layout, [layers, properties]))
}

#[cfg(windows)]
fn create_floating_panel_tree(
    layout: &mut Layout,
    kind: PanelKind,
    side: DockSide,
    window_bounds: Bounds<Pixels>,
    metrics: &DockMetrics,
) -> Result<PanelBinding, String> {
    let key = PanelKey::new(kind.key())
        .map_err(|error| format!("invalid panel key '{}': {error}", kind.key()))?;

    layout
        .apply(
            DockCommand::OpenPanel {
                key: key.clone(),
                constraints: panel_constraints(),
                target: Some(DropTarget::MainColumn { side, index: 0 }),
            },
            metrics,
        )
        .map_err(|error| format!("could not create panel '{}': {error}", kind.key()))?;

    let root = layout
        .main_area(side)
        .map(|area| area.root)
        .ok_or_else(|| format!("dock-core did not create the {:?} panel area", side))?;
    let panel_id = layout
        .panel_instances()
        .find_map(|(panel_id, instance)| (instance.key == key).then_some(panel_id))
        .ok_or_else(|| format!("dock-core did not register panel '{}'", kind.key()))?;

    layout
        .apply(
            DockCommand::Float {
                node: root,
                // Core stores floating geometry in dock-surface coordinates,
                // not in absolute desktop/screen coordinates. The native window
                // reports its final local bounds once both windows have rendered.
                bounds: Rect::new(
                    Point::ZERO,
                    Size::new(
                        window_bounds.size.width.as_f32(),
                        window_bounds.size.height.as_f32(),
                    ),
                ),
            },
            metrics,
        )
        .map_err(|error| format!("could not float panel '{}': {error}", kind.key()))?;

    let floating_group_id = layout
        .floating_groups()
        .find_map(|(group_id, floating)| {
            layout
                .dock_area(floating.area)
                .is_some_and(|area| area.root == root)
                .then_some(group_id)
        })
        .ok_or_else(|| format!("dock-core did not create a floating group for '{}'", kind.key()))?;

    let is_expected_tree = match layout.node(root) {
        Some(Node::Column(column)) if column.groups.len() == 1 => {
            match layout.node(column.groups[0]) {
                Some(Node::Tabs(tabs)) if tabs.children.len() == 1 => {
                    matches!(
                        layout.node(tabs.children[0]),
                        Some(Node::Panel(panel)) if panel.panel_id == panel_id
                    )
                }
                _ => false,
            }
        }
        _ => false,
    };
    if !is_expected_tree {
        return Err(format!(
            "dock-core created an unexpected tree for '{}'; expected Column -> Tabs -> Panel",
            kind.key()
        ));
    }

    Ok(PanelBinding {
        kind,
        panel_id,
        floating_group_id: Some(floating_group_id),
        window_bounds,
    })
}

#[cfg(windows)]
fn panel_constraints() -> PanelConstraints {
    PanelConstraints::new(
        Size::new(120.0, 80.0),
        Size::new(280.0, 220.0),
        None,
        true,
    )
}

fn panel_title(key: &str) -> &str {
    match key {
        "layers" => "Слои",
        "color" => "Цвет",
        "properties" => "Свойства",
        "history" => "История",
        other => other,
    }
}

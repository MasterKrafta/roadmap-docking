//! Minimal experimental docking application entry point.

use dock_core::{
    Axis, DockCommand, DockMetrics, DragDropAction, DragHandle, DragPayload, DragSession,
    DragState, DockSide, DropSource, DropTarget, DropZone, DropZoneRect, DropZoneResolver,
    FloatingGroupId, Layout, Node, NodeId, PanelId, Point, Rect, Size,
};
#[cfg(windows)]
use dock_core::{PanelConstraints, PanelKey, Profile};
use gpui::{
    AnyElement, AppContext, Bounds, Context, Entity, IntoElement, MouseButton, Pixels, Render,
    Subscription, WeakEntity, Window, WindowBounds, WindowHandle, WindowKind, WindowOptions,
    WindowVisibility, div, px, rgb, size,
};
use gpui::prelude::*;
use std::collections::HashSet;
#[cfg(windows)]
use std::time::Duration;
#[cfg(windows)]
use gpui::{App, TitlebarOptions};
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
    external_panel_drag: Option<ExternalPanelDrag>,
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
    floating_group_id: FloatingGroupId,
    window_bounds: Bounds<Pixels>,
}

struct FloatingPanelWindow {
    panel_id: PanelId,
    handle: WindowHandle<FloatingPanel>,
}

struct ExternalPanelDrag {
    panel_id: PanelId,
    session: DragSession,
    resolver: Option<DropZoneResolver>,
    zones: Vec<DropZoneRect>,
    target: Option<DropTarget>,
    pointer: Point,
    viewport: Rect,
    layout_version: u64,
    #[cfg(windows)]
    native_window: NativeWindowHandle,
    #[cfg(windows)]
    cursor_offset: (i32, i32),
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
            external_panel_drag: None,
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
        self.open_floating_panel(PanelKind::Layers, false, cx);
        self.open_floating_panel(PanelKind::Properties, false, cx);
    }

    /// Shows the already-created panels after GPUI has rendered their first frame.
    pub(crate) fn show_floating_panels(&mut self, cx: &mut Context<Self>) {
        for panel in &self.floating_panels {
            if let Err(error) = panel
                .handle
                .update(cx, |_, window, _| window.activate_window())
            {
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

                for panel in &self.floating_panels {
                    match panel.handle.update(cx, |_, window, _| {
                        if window.is_visible() {
                            window.minimize_window();
                            true
                        } else {
                            false
                        }
                    }) {
                        Ok(true) => self
                            .panels_minimized_by_main
                            .push(panel.handle.clone()),
                        Ok(false) => {}
                        Err(_error) => {
                            // The panel may already have been closed by the user.
                        }
                    }
                }
            }
            WindowVisibility::Visible => {
                for panel in self.panels_minimized_by_main.drain(..) {
                    if let Err(error) =
                        panel.update(cx, |_, window, _| window.activate_window())
                    {
                        eprintln!("Could not restore a floating panel: {error:?}");
                    }
                }
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
        let is_being_dragged = self.external_panel_drag.as_ref().is_some_and(|drag| {
            self.panel_bindings
                .iter()
                .any(|binding| binding.panel_id == drag.panel_id && binding.floating_group_id == group)
        });
        if is_being_dragged {
            if let Some(binding) = self
                .panel_bindings
                .iter_mut()
                .find(|binding| binding.floating_group_id == group)
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

        if let Some(binding) = self
            .panel_bindings
            .iter_mut()
            .find(|binding| binding.floating_group_id == group)
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
    ) {
        let Some(binding) = self
            .panel_bindings
            .iter()
            .find(|binding| binding.kind == kind)
            .copied()
        else {
            eprintln!("No dock-core tree is registered for panel '{}'", kind.key());
            return;
        };

        if let Some(existing) = self
            .floating_panels
            .iter()
            .find(|panel| panel.panel_id == binding.panel_id)
        {
            if show_immediately
                && let Err(error) = existing
                    .handle
                    .update(cx, |_, window, _| window.activate_window())
            {
                eprintln!("Could not activate floating panel '{}': {error:?}", kind.key());
            }
            return;
        }

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
            return;
        };

        if let Err(error) = self.apply_layout_command(
            DockCommand::RaiseFloating {
                group: floating_group_id,
            },
            cx,
        ) {
            eprintln!("Could not raise floating panel '{}': {error}", kind.key());
            return;
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
        match cx.open_window(options, move |_window, cx| {
            cx.new(|_| FloatingPanel {
                panel_id,
                floating_group_id,
                owner,
                layout,
                layout_subscription: None,
                bounds_subscription: None,
            })
        }) {
            Ok(handle) => {
                #[cfg(windows)]
                self.configure_floating_window(&handle, cx);

                self.floating_panels.push(FloatingPanelWindow {
                    panel_id,
                    handle: handle.clone(),
                });

                if show_immediately
                    && let Err(error) =
                        handle.update(cx, |_, window, _| window.activate_window())
                {
                    eprintln!("Could not show floating panel '{}': {error:?}", kind.key());
                }
            }
            Err(error) => {
                eprintln!("Could not open floating panel '{}': {error}", kind.key());
            }
        }
    }

    #[cfg(windows)]
    fn begin_external_panel_drag(
        &mut self,
        panel_id: PanelId,
        native_window: NativeWindowHandle,
        cx: &mut Context<Self>,
    ) {
        if self.external_panel_drag.is_some() {
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

        self.external_panel_drag = Some(ExternalPanelDrag {
            panel_id,
            session,
            resolver: None,
            zones: Vec::new(),
            target: None,
            pointer,
            viewport,
            layout_version,
            native_window,
            cursor_offset: (cursor_x - window_left, cursor_y - window_top),
        });

        // Polling continues even after the pointer leaves the floating window.
        // The panel itself is moved by SetWindowPos, so there is no competing
        // Win32 caption-move loop.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(4))
                .await;

            if !matches!(
                this.update(cx, |main, cx| main.poll_external_panel_drag(cx)),
                Ok(true)
            ) {
                break;
            }
        })
        .detach();
    }

    #[cfg(windows)]
    fn poll_external_panel_drag(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.external_panel_drag.as_ref() else {
            return false;
        };
        let native_window = drag.native_window;
        let (offset_x, offset_y) = drag.cursor_offset;
        let (cursor_x, cursor_y) = platform::windows::cursor_position();
        let left_button_down = platform::windows::left_button_down();

        let Some(pointer) = self.external_panel_pointer_from_screen(cursor_x, cursor_y) else {
            let panel_id = drag.panel_id;
            self.external_panel_drag = None;
            self.sync_floating_panel_after_drag(panel_id, cx);
            cx.notify();
            return false;
        };

        if !self.update_external_panel_drag(pointer, cx) {
            return false;
        }

        let is_dragging = self.external_panel_drag.as_ref().is_some_and(|drag| {
            matches!(drag.session.state(), DragState::Dragging { .. })
        });
        if is_dragging
            && !platform::windows::move_window(
                native_window,
                cursor_x - offset_x,
                cursor_y - offset_y,
            )
        {
            eprintln!("Could not move the floating panel during docking drag");
        }

        if left_button_down {
            true
        } else {
            self.finish_external_panel_drag(pointer, cx);
            false
        }
    }

    #[cfg(windows)]
    fn update_external_panel_drag(&mut self, pointer: Point, cx: &mut Context<Self>) -> bool {
        let Some((panel_id, viewport, drag_version)) = self
            .external_panel_drag
            .as_ref()
            .map(|drag| (drag.panel_id, drag.viewport, drag.layout_version))
        else {
            return false;
        };

        if self.layout.read(cx).version() != drag_version
            || self.last_dock_viewport != Some(viewport)
        {
            self.external_panel_drag = None;
            self.sync_floating_panel_after_drag(panel_id, cx);
            cx.notify();
            return false;
        }

        let pointer_update_error = match self.external_panel_drag.as_mut() {
            Some(drag) => drag.session.pointer_move(pointer).err(),
            None => return false,
        };
        if let Some(error) = pointer_update_error {
            eprintln!("Could not update the panel drag pointer: {error}");
            self.external_panel_drag = None;
            self.sync_floating_panel_after_drag(panel_id, cx);
            cx.notify();
            return false;
        }

        let (needs_resolver, pointer_changed) = {
            let Some(drag) = self.external_panel_drag.as_mut() else {
                return false;
            };
            let pointer_changed = drag.pointer.x != pointer.x || drag.pointer.y != pointer.y;
            drag.pointer = pointer;
            (
                matches!(drag.session.state(), DragState::Dragging { .. })
                    && drag.resolver.is_none(),
                pointer_changed,
            )
        };

        let mut resolver_created = false;
        if needs_resolver {
            let prepared = {
                let layout = self.layout.read(cx);
                prepare_external_panel_resolver(&layout, viewport, self.metrics, panel_id)
                    .map(|(resolver, zones)| (layout.version(), resolver, zones))
            };
            match prepared {
                Ok((version, resolver, zones)) if version == drag_version => {
                    if let Some(drag) = self.external_panel_drag.as_mut() {
                        drag.resolver = Some(resolver);
                        drag.zones = zones;
                        resolver_created = true;
                    }
                }
                Ok(_) => {
                    self.external_panel_drag = None;
                    self.sync_floating_panel_after_drag(panel_id, cx);
                    cx.notify();
                    return false;
                }
                Err(error) => {
                    eprintln!("Could not prepare Photoshop edge drop zones: {error}");
                    self.external_panel_drag = None;
                    self.sync_floating_panel_after_drag(panel_id, cx);
                    cx.notify();
                    return false;
                }
            }
        }

        let Some(drag) = self.external_panel_drag.as_mut() else {
            return false;
        };
        let target = drag
            .resolver
            .as_ref()
            .and_then(|resolver| resolve_edge_drop_target(resolver, &drag.zones, pointer));
        let target_changed = drag.target != target;
        drag.target = target;
        if drag.resolver.is_some()
            && let Err(error) = drag.session.set_drop_target(target)
        {
            eprintln!("Could not update the panel drag target: {error}");
        }

        if pointer_changed || target_changed || resolver_created {
            cx.notify();
        }
        true
    }

    #[cfg(windows)]
    fn finish_external_panel_drag(&mut self, pointer: Point, cx: &mut Context<Self>) {
        let Some(drag) = self.external_panel_drag.take() else {
            return;
        };

        let request = match {
            let mut session = drag.session;
            session.pointer_up(pointer, drag.target)
        } {
            Ok(request) => request,
            Err(error) => {
                eprintln!("Could not finish the panel drag session: {error}");
                self.sync_floating_panel_after_drag(drag.panel_id, cx);
                cx.notify();
                return;
            }
        };

        let panel_id = drag.panel_id;
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
            let panel_handle = self
                .floating_panels
                .iter()
                .find(|panel| panel.panel_id == panel_id)
                .map(|panel| panel.handle.clone());
            if let Some(handle) = panel_handle
                && let Err(error) = handle.update(cx, |_, window, _| window.remove_window())
            {
                eprintln!("Could not close the panel window after docking: {error:?}");
            }

            let window_id = self
                .floating_panels
                .iter()
                .find(|panel| panel.panel_id == panel_id)
                .map(|panel| panel.handle.window_id());
            self.floating_panels
                .retain(|panel| panel.panel_id != panel_id);
            if let Some(window_id) = window_id {
                self.panels_minimized_by_main
                    .retain(|panel| panel.window_id() != window_id);
            }

            if let Some(main_window) = self.main_window_handle
                && let Err(error) =
                    main_window.update(cx, |_, window, _| window.activate_window())
            {
                eprintln!("Could not activate the main window after docking: {error:?}");
            }
        } else {
            self.sync_floating_panel_after_drag(panel_id, cx);
        }

        cx.notify();
    }

    #[cfg(windows)]
    fn sync_floating_panel_after_drag(&mut self, panel_id: PanelId, cx: &mut Context<Self>) {
        let group = self
            .panel_bindings
            .iter()
            .find(|binding| binding.panel_id == panel_id)
            .map(|binding| binding.floating_group_id);
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
        self.external_panel_drag = None;
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
            if let Err(error) =
                owner.update(cx, |this, cx| this.open_floating_panel(kind, true, cx))
            {
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

        if let Some(drag) = self.external_panel_drag.as_ref()
            && drag.resolver.is_some()
        {
            if let Some(zone) = drag
                .target
                .and_then(|target| drag.zones.iter().find(|zone| zone.target == target))
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

            let label = layout
                .panel_instance(drag.panel_id)
                .map(|instance| panel_title(instance.key.as_str()).to_owned())
                .unwrap_or_else(|| "Панель".to_owned());
            let ghost_width = 176.0_f32.min(width.max(1.0));
            let ghost_height = 34.0_f32;
            let ghost_x = (drag.pointer.x + 14.0).clamp(0.0, (width - ghost_width).max(0.0));
            let ghost_y = (drag.pointer.y + 14.0)
                .clamp(0.0, (content_height - ghost_height).max(0.0));
            workspace = workspace.child(
                div()
                    .absolute()
                    .left(px(ghost_x))
                    .top(px(ghost_y))
                    .w(px(ghost_width))
                    .h(px(ghost_height))
                    .px_3()
                    .flex()
                    .items_center()
                    .bg(rgb(0x344c60))
                    .border_1()
                    .border_color(rgb(0x69c3ff))
                    .text_color(rgb(0xf0f5fa))
                    .child(label),
            );
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
    floating_group_id: FloatingGroupId,
    owner: WeakEntity<MainWindow>,
    layout: Entity<Layout>,
    layout_subscription: Option<Subscription>,
    bounds_subscription: Option<Subscription>,
}

impl FloatingPanel {
    fn sync_native_bounds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let native_bounds = window.window_bounds().get_bounds();
        let viewport_size = window.viewport_size();
        let group = self.floating_group_id;

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
        let title = layout
            .panel_instance(self.panel_id)
            .map(|instance| panel_title(instance.key.as_str()).to_owned())
            .unwrap_or_else(|| "Панель".to_owned());
        let content = render_floating_group(
            &layout,
            self.floating_group_id,
            self.owner.clone(),
        )
        .unwrap_or_else(|| {
            div()
                .flex_1()
                .p_4()
                .text_color(rgb(0xf08a8a))
                .child("Дерево панели не найдено")
                .into_any_element()
        });

        let owner = self.owner.clone();
        let drag_owner = owner.clone();
        let panel_id = self.panel_id;
        let drag_area = div()
            .id("panel-drag-area")
            .flex()
            .items_center()
            .flex_1()
            .h_full()
            .px_2()
            .child(title.clone());
        #[cfg(windows)]
        let drag_area = drag_area.on_mouse_down(MouseButton::Left, move |_event, window, cx| {
            let Some(native_window) = NativeWindowHandle::from_window(window) else {
                eprintln!("Could not obtain the floating panel's native handle for dragging");
                return;
            };
            if let Err(error) = drag_owner.update(cx, |main, cx| {
                main.begin_external_panel_drag(panel_id, native_window, cx);
            }) {
                eprintln!("Could not begin panel docking drag: {error:?}");
            }
        });

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x292f37))
            .text_color(rgb(0xe8ebef))
            .child(
                div()
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
                    .child(
                        div()
                            .id("close-panel")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.0))
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0x596777)))
                            .occlude()
                            .on_click(move |_, window, cx| {
                                let window_id = window.window_handle().window_id();
                                let main_window = owner.update(cx, |this, _cx| {
                                    this.floating_panels
                                        .retain(|panel| panel.panel_id != panel_id);
                                    this.panels_minimized_by_main
                                        .retain(|panel| panel.window_id() != window_id);
                                    this.main_window_handle
                                });

                                match main_window {
                                    Ok(Some(main_window)) => {
                                        if let Err(error) = main_window.update(
                                            cx,
                                            |_, main_window, _| main_window.activate_window(),
                                        ) {
                                            eprintln!(
                                                "Could not activate the main window after closing a panel: {error:?}"
                                            );
                                        }
                                    }
                                    Ok(None) => {
                                        eprintln!(
                                            "Could not activate the main window after closing a panel: handle unavailable"
                                        );
                                    }
                                    Err(error) => {
                                        eprintln!(
                                            "Could not access the main window while closing a panel: {error:?}"
                                        );
                                    }
                                }

                                window.remove_window();
                            })
                            .child("×"),
                    ),
            )
            .child(content)
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
) -> Result<(DropZoneResolver, Vec<DropZoneRect>), String> {
    let mut rects = layout
        .compute_rects(viewport, &metrics)
        .map_err(|error| format!("could not compute main dock geometry: {error}"))?;

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
    let zones = resolver
        .zones()
        .iter()
        .copied()
        .filter(|zone| zone.zone == DropZone::DockEdge)
        .collect();

    Ok((resolver, zones))
}

fn resolve_edge_drop_target(
    resolver: &DropZoneResolver,
    zones: &[DropZoneRect],
    pointer: Point,
) -> Option<DropTarget> {
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
    let content = render_core_node(layout, root, owner);

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

    Some(render_core_node(layout, root, owner))
}

fn render_core_node(
    layout: &Layout,
    node_id: NodeId,
    owner: WeakEntity<MainWindow>,
) -> AnyElement {
    let Some(node) = layout.node(node_id) else {
        return div()
            .p_2()
            .text_color(rgb(0xf08a8a))
            .child("Узел дерева не найден")
            .into_any_element();
    };

    match node {
        Node::Column(column) => div()
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            .gap_2()
            .p_2()
            .bg(rgb(0x262c33))
            .children(
                column
                    .groups
                    .iter()
                    .map(|child| render_core_node(layout, *child, owner.clone())),
            )
            .into_any_element(),
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
                .map(|child| render_core_node(layout, child, owner.clone()))
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
                        .map(|child| render_core_node(layout, *child, owner.clone())),
                )
                .into_any_element()
        }
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
    let mut layout = Layout::new(Profile::Generic);

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
        floating_group_id,
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

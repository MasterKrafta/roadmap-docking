use std::time::Duration;

use gpui::{AnyWindowHandle, Context, WindowHandle};

use crate::{
    floating_manager::FloatingWindowManager,
    floating::FloatingColumnHost,
    drag_host_manager::DragHostSurface,
};

#[derive(Clone, Copy)]
pub(super) enum PanelWindowHandle {
    Column(WindowHandle<FloatingColumnHost>),
    DragHost(WindowHandle<DragHostSurface>),
}

impl PanelWindowHandle {
    pub(super) fn is_available(&self, cx: &mut Context<FloatingWindowManager>) -> bool {
        match self {
            // A FloatingColumnHost can be inside its own GPUI update while the
            // manager reconciles it. Its native HWND is checked by the manager;
            // never probe the root Entity through WindowHandle::update here.
            Self::Column(_) => true,
            Self::DragHost(window) => window.update(cx, |_, _, _| ()).is_ok(),
        }
    }

    /// Updates close-button ownership without mutating a permanent FloatingColumnHost.
    /// Its presentation state is shared with the manager through PanelWindowEntry.
    pub(super) fn set_column_close_button(
        &self,
        show: bool,
        cx: &mut Context<FloatingWindowManager>,
    ) -> Result<(), String> {
        match self {
            Self::Column(_) => Ok(()),
            Self::DragHost(window) => window
                .update(cx, |surface, _, surface_cx| {
                    surface.set_promoted_column_close_button(show, surface_cx);
                })
                .map_err(|error| error.to_string()),
        }
    }

    /// Refreshes cached native client bounds without updating the root Host Entity.
    pub(super) fn refresh_bounds_later(&self, cx: &mut Context<FloatingWindowManager>) {
        let window: AnyWindowHandle = match self {
            Self::Column(window) => (*window).into(),
            Self::DragHost(window) => (*window).into(),
        };
        cx.spawn(async move |_, app| {
            for _ in 0..16 {
                if window
                    .update(app, |_, window, app| window.bounds_changed(app))
                    .is_ok()
                {
                    break;
                }
                app.background_executor()
                    .timer(Duration::from_millis(8))
                    .await;
            }
        })
        .detach();
    }

    /// Requests a redraw from the exact GPUI window that owns this host.
    pub(super) fn notify_host(&self, cx: &mut Context<FloatingWindowManager>) -> Result<(), String> {
        match self {
            Self::Column(window) => {
                let window: AnyWindowHandle = (*window).into();
                window
                    .update(cx, |_, window, _| window.refresh())
                    .map_err(|error| error.to_string())
            }
            Self::DragHost(window) => window
                .update(cx, |_, _, host_cx| host_cx.notify())
                .map_err(|error| error.to_string()),
        }
    }

    pub(super) fn remove_window(&self, cx: &mut Context<FloatingWindowManager>) {
        match self {
            Self::Column(window) => {
                let window: AnyWindowHandle = (*window).into();
                cx.spawn(async move |_, app| {
                    for _ in 0..16 {
                        if window
                            .update(app, |_, window, _| window.remove_window())
                            .is_ok()
                        {
                            break;
                        }
                        app.background_executor()
                            .timer(Duration::from_millis(8))
                            .await;
                    }
                })
                .detach();
            }
            Self::DragHost(window) => {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            }
        }
    }
}

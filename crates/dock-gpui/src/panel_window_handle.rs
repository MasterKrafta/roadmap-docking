use std::time::Duration;

use dock_core::FloatingGroupId;
use gpui::{AnyWindowHandle, AppContext, Context, WindowHandle};

use crate::{
    components::{FloatingHost, floating::FloatingPartHost},
    drag_host_manager::DragHostSurface,
};

use super::FloatingWindowManager;

#[derive(Clone, Copy)]
pub(super) enum PanelWindowHandle {
    Floating(WindowHandle<FloatingHost>),
    Part(WindowHandle<FloatingPartHost>),
    DragHost(WindowHandle<DragHostSurface>),
}

impl PanelWindowHandle {
    pub(super) fn is_available(&self, cx: &mut Context<FloatingWindowManager>) -> bool {
        match self {
            Self::Floating(window) => window.update(cx, |_, _, _| ()).is_ok(),
            // A FloatingPartHost can be inside its own GPUI update while the
            // manager reconciles it. Its native HWND is checked by the manager;
            // never probe the root Entity through WindowHandle::update here.
            Self::Part(_) => true,
            Self::DragHost(window) => window.update(cx, |_, _, _| ()).is_ok(),
        }
    }

    pub(super) fn set_group(
        &self,
        group: Option<FloatingGroupId>,
        cx: &mut Context<FloatingWindowManager>,
    ) -> Result<(), String> {
        match self {
            Self::Floating(window) => window
                .update(cx, |host, window, host_cx| {
                    host.set_host_group(group, window);
                    host_cx.notify();
                })
                .map_err(|error| error.to_string()),
            Self::Part(_) | Self::DragHost(_) => {
                Err("Floating part windows have immutable Core ownership".into())
            }
        }
    }

    /// Refreshes cached native client bounds without updating the root Host Entity.
    ///
    /// A FloatingPartHost may be handling pointer input or rendering when the
    /// manager reconciles a resize. AnyWindowHandle updates the Window directly,
    /// avoiding a re-entrant update of the root entity. Retry while GPUI has the
    /// native Window temporarily taken for its current event/update cycle.
    pub(super) fn refresh_bounds_later(
        &self,
        cx: &mut Context<FloatingWindowManager>,
    ) {
        let window: AnyWindowHandle = match self {
            Self::Floating(window) => (*window).into(),
            Self::Part(window) => (*window).into(),
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
            Self::Floating(window) => window
                .update(cx, |_, _, host_cx| host_cx.notify())
                .map_err(|error| error.to_string()),
            Self::Part(window) => {
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
            Self::Floating(window) => {
                let _ = window.update(cx, |host, window, _| {
                    host.set_host_group(None, window);
                    window.remove_window();
                });
            }
            Self::Part(window) => {
                let window: AnyWindowHandle = (*window).into();
                let _ = window.update(cx, |_, window, _| window.remove_window());
            }
            Self::DragHost(window) => {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            }
        }
    }
}

use dock_core::FloatingGroupId;
use gpui::{Context, WindowHandle};

use crate::{
    components::{FloatingHost, floating::FloatingPartHost},
    drag_host_manager::DragHostSurface,
};

use super::FloatingWindowManager;

#[derive(Clone, Copy)]
pub(super) enum PanelWindowHandle {
    Floating(WindowHandle<FloatingHost>),
    Part(WindowHandle<FloatingPartHost>),
    DragHost(WindowHandle<crate::drag_host_manager::DragHostSurface>),
}

impl PanelWindowHandle {
    pub(super) fn is_available(&self, cx: &mut Context<FloatingWindowManager>) -> bool {
        match self {
            Self::Floating(window) => window.update(cx, |_, _, _| ()).is_ok(),
            Self::Part(window) => window.update(cx, |_, _, _| ()).is_ok(),
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

    /// Updates the cached close-button ownership for a permanent Header host.
    ///
    /// Rendering a FloatingPartHost must not update FloatingWindowManager: a
    /// reconciliation can notify that host while this manager is already updating.
    pub(super) fn set_header_close_button(
        &self,
        show: bool,
        cx: &mut Context<FloatingWindowManager>,
    ) -> Result<(), String> {
        match self {
            Self::Part(window) => window
                .update(cx, |host, _, host_cx| {
                    if host.set_show_close_button(show) {
                        host_cx.notify();
                    }
                })
                .map_err(|error| error.to_string()),
            Self::Floating(_) | Self::DragHost(_) => Ok(()),
        }
    }

    /// Requests a redraw from the exact GPUI window that owns this host.
    pub(super) fn notify_host(&self, cx: &mut Context<FloatingWindowManager>) -> Result<(), String> {
        match self {
            Self::Floating(window) => window
                .update(cx, |_, _, host_cx| host_cx.notify())
                .map_err(|error| error.to_string()),
            Self::Part(window) => window
                .update(cx, |_, _, host_cx| host_cx.notify())
                .map_err(|error| error.to_string()),
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
                let _ = window.update(cx, |_, window, _| window.remove_window());
            }
            Self::DragHost(window) => {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            }
        }
    }

}

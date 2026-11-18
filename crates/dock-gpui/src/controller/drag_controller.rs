use dock_core::{
    DockMetrics, DragCancelReason, DragSession, DragSessionError, DropSource, DropZoneResolver,
    PanelId,
};
use gpui::WindowId;

use crate::drag_host::DragHostSession;

use super::{FloatingRectCacheKey, RectCacheKey, ResolverCacheKey};

/// State for an active drag gesture.
///
/// DragSession and the optional resolver are transient interaction data only;
/// the layout tree remains exclusively owned by dock_core::Layout.
pub struct DragController {
    pub(super) session: DragSession,
    pub(super) main_resolver: Option<(RectCacheKey, DropSource, DropZoneResolver)>,
    pub(super) floating_resolver: Option<(FloatingRectCacheKey, DropSource, DropZoneResolver)>,
    /// Identifies which host-local resolver is currently active for preview/rendering.
    pub(super) resolver_key: Option<ResolverCacheKey>,
    /// Layout revision captured when the current drag press began.
    pub(super) layout_version: Option<u64>,
    /// Adapter context keeps the original source host separate from its transport host.
    pub(super) host_session: Option<DragHostSession>,
}

impl DragController {
    pub(super) fn new(metrics: &DockMetrics) -> Result<Self, DragSessionError> {
        Ok(Self {
            session: DragSession::new(metrics)?,
            main_resolver: None,
            floating_resolver: None,
            resolver_key: None,
            layout_version: None,
            host_session: None,
        })
    }

    pub fn session(&self) -> &DragSession {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut DragSession {
        &mut self.session
    }

    pub fn resolver(&self) -> Option<&DropZoneResolver> {
        match self.resolver_key? {
            ResolverCacheKey::Main(key) => self
                .main_resolver
                .as_ref()
                .filter(|(cached_key, _, _)| *cached_key == key)
                .map(|(_, _, resolver)| resolver),
            ResolverCacheKey::Floating(key) => self
                .floating_resolver
                .as_ref()
                .filter(|(cached_key, _, _)| *cached_key == key)
                .map(|(_, _, resolver)| resolver),
        }
    }

    /// Returns the adapter's source/transport context for the active content drag.
    pub fn host_session(&self) -> Option<&DragHostSession> {
        self.host_session.as_ref()
    }

    pub(crate) fn begin_host_session(&mut self, session: DragHostSession) {
        self.host_session = Some(session);
    }

    pub(crate) fn attach_transport_host(&mut self, id: crate::DragHostId) -> bool {
        self.host_session
            .as_mut()
            .is_some_and(|session| session.attach_transport_host(id))
    }

    pub(crate) fn clear_transport_host(&mut self, id: crate::DragHostId) -> bool {
        self.host_session
            .as_mut()
            .is_some_and(|session| session.clear_transport_host(id))
    }

    pub(crate) fn can_begin_panel_view_transfer(&self, panel: PanelId) -> bool {
        self.host_session
            .as_ref()
            .is_some_and(|session| session.can_begin_panel_view_transfer(panel))
    }

    pub(crate) fn begin_panel_view_transfer(&mut self, panel: PanelId) -> bool {
        self.host_session
            .as_mut()
            .is_some_and(|session| session.begin_panel_view_transfer(panel))
    }

    pub(crate) fn finish_panel_view_transfer(&mut self, panel: PanelId) -> bool {
        self.host_session
            .as_mut()
            .is_some_and(|session| session.finish_panel_view_transfer(panel))
    }

    pub(crate) fn panel_view_is_transferred(&self, panel: PanelId) -> bool {
        self.host_session
            .as_ref()
            .is_some_and(|session| session.transferred_panels().contains(&panel))
    }

    pub(crate) fn panel_view_source_detached(&self, panel: PanelId) -> bool {
        self.host_session
            .as_ref()
            .is_some_and(|session| session.panel_view_source_detached(panel))
    }

    pub(crate) fn confirm_panel_view_source_detached(
        &mut self,
        panel: PanelId,
        source_window_id: WindowId,
    ) -> bool {
        self.host_session
            .as_mut()
            .is_some_and(|session| {
                session.confirm_panel_view_source_detached(panel, source_window_id)
            })
    }

    /// Finishes a successfully released content drag and discards its adapter context.
    pub(crate) fn finish_host_session(&mut self) {
        self.host_session = None;
    }

    /// Cancels a content drag and discards its adapter context.
    pub(crate) fn cancel_host_session(&mut self) {
        self.host_session = None;
    }

    pub(super) fn set_main_resolver(
        &mut self,
        resolver: DropZoneResolver,
        key: RectCacheKey,
        source: DropSource,
    ) {
        self.main_resolver = Some((key, source, resolver));
        self.resolver_key = Some(ResolverCacheKey::Main(key));
    }

    pub(super) fn set_floating_resolver(
        &mut self,
        resolver: DropZoneResolver,
        key: FloatingRectCacheKey,
        source: DropSource,
    ) {
        self.floating_resolver = Some((key, source, resolver));
        self.resolver_key = Some(ResolverCacheKey::Floating(key));
    }

    pub fn clear_resolver(&mut self) {
        self.main_resolver = None;
        self.floating_resolver = None;
        self.resolver_key = None;
    }

    pub(crate) fn clear_layout_version(&mut self) {
        self.layout_version = None;
    }

    /// Cancels any active gesture and clears its resolver snapshot.
    ///
    /// The adapter selects the reason so Escape, focus loss, and model/viewport
    /// invalidation remain distinguishable in Core's terminal state.
    pub fn cancel_gesture(&mut self) {
        self.cancel_gesture_with_reason(DragCancelReason::Explicit);
    }

    pub(super) fn cancel_gesture_with_reason(&mut self, reason: DragCancelReason) {
        match reason {
            DragCancelReason::Escape => {
                let _ = self.session.escape();
            }
            DragCancelReason::FocusLost => {
                let _ = self.session.focus_lost();
            }
            _ => {
                let _ = self.session.cancel();
            }
        }
        let _ = self.session.reset();
        self.clear_resolver();
        self.clear_layout_version();
    }
}

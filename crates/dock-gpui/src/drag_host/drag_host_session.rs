use dock_core::{NodeId, PanelId};
use gpui::WindowId;

use super::{DockWindowRole, DragHostId, DragSourceHost};

/// Adapter-only context retained for one content drag across host transitions.
///
/// The semantic payload and handle remain the original source values. The
/// optional DragHostId identifies only the temporary presentation surface and
/// never replaces the source host or the Core payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DragHostSession {
    payload: dock_core::DragPayload,
    handle: dock_core::DragHandle,
    source_host: DragSourceHost,
    source_window_id: Option<WindowId>,
    transport_host: Option<DragHostId>,
    transferred_panels: Vec<PanelId>,
    detached_panels: Vec<PanelId>,
}

impl DragHostSession {
    /// Starts runtime tracking for a content drag after Core accepts pointer-down.
    pub(crate) const fn new(
        payload: dock_core::DragPayload,
        handle: dock_core::DragHandle,
        source_host: DragSourceHost,
        source_window_id: Option<WindowId>,
    ) -> Self {
        Self {
            payload,
            handle,
            source_host,
            source_window_id,
            transport_host: None,
            transferred_panels: Vec::new(),
            detached_panels: Vec::new(),
        }
    }

    /// Returns the original semantic object carried by this drag.
    pub const fn payload(&self) -> dock_core::DragPayload { self.payload }

    /// Returns the original interaction surface that initiated this drag.
    pub const fn handle(&self) -> dock_core::DragHandle { self.handle }

    /// Returns the persistent host where the content originated.
    pub const fn source_host(&self) -> DragSourceHost { self.source_host }

    /// Returns the GPUI WindowId of the persistent source host, when recorded.
    pub const fn source_window_id(&self) -> Option<WindowId> {
        self.source_window_id
    }

    /// Returns the temporary DragHost identity currently presenting the content.
    pub const fn transport_host(&self) -> Option<DragHostId> { self.transport_host }

    /// Returns the transport window role without changing the original source.
    pub const fn transport_role(&self) -> Option<DockWindowRole> {
        match self.transport_host {
            Some(id) => Some(DockWindowRole::Dragging { id }),
            None => None,
        }
    }

    /// Returns the cached PanelViews currently transported by DragHost.
    pub fn transferred_panels(&self) -> &[PanelId] {
        &self.transferred_panels
    }

    /// Returns the first transported Panel for single-panel compatibility.
    pub fn transferred_panel(&self) -> Option<PanelId> {
        self.transferred_panels.first().copied()
    }

    /// Returns whether every transferred PanelView has left its source host.
    pub fn all_panel_views_source_detached(&self) -> bool {
        !self.transferred_panels.is_empty()
            && self
                .transferred_panels
                .iter()
                .all(|panel| self.detached_panels.contains(panel))
    }

    pub(crate) fn can_begin_panel_view_transfer(&self, panel: PanelId) -> bool {
        self.handle == dock_core::DragHandle::PanelTab
            && matches!(self.payload, dock_core::DragPayload::Panel { panel: active } if active == panel)
            && self.transferred_panels.is_empty()
    }

    pub(crate) fn can_begin_group_view_transfer(
        &self,
        group: NodeId,
        panels: &[PanelId],
    ) -> bool {
        self.handle == dock_core::DragHandle::GroupGrip
            && matches!(self.payload, dock_core::DragPayload::Group { group: active } if active == group)
            && self.transferred_panels.is_empty()
            && !panels.is_empty()
            && panels
                .iter()
                .enumerate()
                .all(|(index, panel)| !panels[..index].contains(panel))
    }

    pub(crate) fn begin_group_view_transfer(
        &mut self,
        group: NodeId,
        panels: &[PanelId],
    ) -> bool {
        if self.transport_host.is_none() || !self.can_begin_group_view_transfer(group, panels) {
            return false;
        }
        self.transferred_panels.extend_from_slice(panels);
        true
    }

    pub(crate) fn can_begin_dock_area_view_transfer(
        &self,
        side: dock_core::DockSide,
        panels: &[PanelId],
    ) -> bool {
        self.handle == dock_core::DragHandle::DockAreaHeader
            && matches!(
                self.payload,
                dock_core::DragPayload::DockArea { side: active } if active == side
            )
            && self.transferred_panels.is_empty()
            && panels
                .iter()
                .enumerate()
                .all(|(index, panel)| !panels[..index].contains(panel))
    }

    pub(crate) fn begin_dock_area_view_transfer(
        &mut self,
        side: dock_core::DockSide,
        panels: &[PanelId],
    ) -> bool {
        if self.transport_host.is_none()
            || !self.can_begin_dock_area_view_transfer(side, panels)
        {
            return false;
        }
        self.transferred_panels.extend_from_slice(panels);
        true
    }

    pub(crate) fn begin_panel_view_transfer(&mut self, panel: PanelId) -> bool {
        if self.transport_host.is_none() || !self.can_begin_panel_view_transfer(panel) {
            return false;
        }
        self.transferred_panels.push(panel);
        true
    }

    /// Returns whether this PanelView has left its source host.
    pub fn panel_view_source_detached(&self, panel: PanelId) -> bool {
        self.transferred_panels.contains(&panel) && self.detached_panels.contains(&panel)
    }

    pub(crate) fn confirm_panel_view_source_detached(
        &mut self,
        panel: PanelId,
        source_window_id: WindowId,
    ) -> bool {
        if !self.transferred_panels.contains(&panel)
            || self.source_window_id != Some(source_window_id)
            || self.detached_panels.contains(&panel)
        {
            return false;
        }
        self.detached_panels.push(panel);
        true
    }

    pub(crate) fn finish_panel_view_transfer(&mut self, panel: PanelId) -> bool {
        if !self.transferred_panels.contains(&panel) {
            return false;
        }
        self.transferred_panels.retain(|active| *active != panel);
        self.detached_panels.retain(|active| *active != panel);
        true
    }

    pub(crate) fn attach_transport_host(&mut self, id: DragHostId) -> bool {
        match self.transport_host {
            None => { self.transport_host = Some(id); true }
            Some(active) => active == id,
        }
    }

    pub(crate) fn clear_transport_host(&mut self, id: DragHostId) -> bool {
        if self.transport_host != Some(id) { return false; }
        self.transport_host = None;
        true
    }
}

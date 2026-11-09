//! Runtime identity and role types for dock-related native windows.
//!
//! These types describe GPUI adapter windows only. A DragHost is a temporary
//! presentation surface; it is not a Core DockArea, FloatingGroup, or workspace
//! entity and must never be inserted into or serialized with Layout.
//!
//! The future DragHostManager is owned by DockHost as a sibling of
//! FloatingWindowManager. Both managers receive the same DockController entity;
//! neither manager owns or duplicates the Core Layout.

use dock_core::{FloatingGroupId, NodeId, PanelId};
use gpui::WindowId;

/// Runtime identity of one temporary drag-presentation window.
///
/// IDs are allocated by the adapter's DragHost manager. They are deliberately
/// independent from GPUI WindowId and Core FloatingGroupId.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DragHostId(u64);

impl DragHostId {
    /// Wraps an adapter-assigned runtime value.
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// Returns the adapter-assigned runtime value.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Describes the runtime purpose of a native window used by the dock adapter.
///
/// This is not a Core model type. In particular, `Dragging` identifies a
/// temporary transport/presentation window and does not imply that it owns a
/// dock tree or a `FloatingGroup`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DockWindowRole {
    /// The application's persistent main window.
    Main,
    /// A persistent native window hosting one Core floating composition.
    Floating {
        /// Core identity of the composition hosted by this window.
        group: FloatingGroupId,
    },
    /// A temporary window used only while a dock object is being dragged.
    Dragging {
        /// Runtime identity of the temporary drag host.
        id: DragHostId,
    },
}

/// Identifies the persistent host where dragged dock content originated.
///
/// Main is unique within one DockHost. Floating uses its Core group identity,
/// which remains stable even if the panel view later moves to another host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragSourceHost {
    /// Content originated in the persistent main dock window.
    Main,
    /// Content originated in a persistent floating composition.
    Floating {
        /// Core identity of the source floating composition.
        group: FloatingGroupId,
    },
}

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
        self.transferred_panels
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
        _panel: PanelId,
    ) -> bool {
        self.handle == dock_core::DragHandle::GroupGrip
            && matches!(self.payload, dock_core::DragPayload::Group { group: active } if active == group)
            && self.transferred_panels.is_empty()
    }

    pub(crate) fn begin_group_view_transfer(
        &mut self,
        group: NodeId,
        panel: PanelId,
    ) -> bool {
        if self.transport_host.is_none() || !self.can_begin_group_view_transfer(group, panel) {
            return false;
        }
        self.transferred_panels.push(panel);
        true
    }

    pub(crate) fn can_begin_column_view_transfer(
        &self,
        column: NodeId,
        panels: &[PanelId],
    ) -> bool {
        self.handle == dock_core::DragHandle::ColumnHeader
            && matches!(self.payload, dock_core::DragPayload::Column { column: active } if active == column)
            && self.transferred_panels.is_empty()
            && panels.iter().enumerate().all(|(index, panel)| !panels[..index].contains(panel))
    }

    pub(crate) fn begin_column_view_transfer(
        &mut self,
        column: NodeId,
        panels: &[PanelId],
    ) -> bool {
        if self.transport_host.is_none()
            || !self.can_begin_column_view_transfer(column, panels)
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

#[cfg(test)]
mod tests {
    use dock_core::FloatingGroupId;
    use slotmap::Key as _;

    use super::{DockWindowRole, DragHostId};

    #[test]
    fn drag_host_id_is_a_distinct_runtime_identity() {
        let first = DragHostId::from_raw(1);
        let second = DragHostId::from_raw(2);

        assert_ne!(first, second);
        assert_eq!(first.get(), 1);
        assert_eq!(second.get(), 2);
    }

    #[test]
    fn drag_host_session_keeps_original_payload_and_source_separate_from_transport() {
        use dock_core::{DragHandle, DragPayload, PanelId};

        let mut panel_ids = slotmap::SlotMap::<PanelId, ()>::with_key();
        let panel = panel_ids.insert(());
        let other_panel = panel_ids.insert(());
        let source_group = FloatingGroupId::null();
        let mut session = super::DragHostSession::new(
            DragPayload::Panel { panel },
            DragHandle::PanelTab,
            super::DragSourceHost::Floating { group: source_group },
            None,
        );
        let drag_host = DragHostId::from_raw(11);

        assert_eq!(session.payload(), DragPayload::Panel { panel });
        assert_eq!(session.handle(), DragHandle::PanelTab);
        assert_eq!(
            session.source_host(),
            super::DragSourceHost::Floating { group: source_group }
        );
        assert_eq!(session.source_window_id(), None);
        assert_eq!(session.transport_host(), None);
        assert_eq!(session.transferred_panel(), None);
        assert!(session.transferred_panels().is_empty());
        assert!(!session.panel_view_source_detached(panel));
        assert!(session.can_begin_panel_view_transfer(panel));
        assert!(!session.begin_panel_view_transfer(panel));

        assert!(session.attach_transport_host(drag_host));
        assert_eq!(session.transport_host(), Some(drag_host));
        assert_eq!(
            session.transport_role(),
            Some(DockWindowRole::Dragging { id: drag_host })
        );
        assert!(session.begin_panel_view_transfer(panel));
        assert_eq!(session.transferred_panel(), Some(panel));
        assert_eq!(session.transferred_panels(), &[panel]);
        assert!(!session.panel_view_source_detached(panel));
        assert!(!session.begin_panel_view_transfer(panel));
        assert_eq!(session.payload(), DragPayload::Panel { panel });
        assert_eq!(
            session.source_host(),
            super::DragSourceHost::Floating { group: source_group }
        );

        assert!(!session.finish_panel_view_transfer(other_panel));
        assert!(session.finish_panel_view_transfer(panel));
        assert_eq!(session.transferred_panel(), None);
        assert!(session.transferred_panels().is_empty());
        assert!(!session.panel_view_source_detached(panel));
        assert!(!session.clear_transport_host(DragHostId::from_raw(12)));
        assert!(session.clear_transport_host(drag_host));
        assert_eq!(session.transport_host(), None);
    }

    #[test]
    fn group_drag_transfers_only_the_active_panel_view_after_host_is_attached() {
        use dock_core::{DragHandle, DragPayload, NodeId, PanelId};
        use slotmap::SlotMap;

        let mut group_ids = SlotMap::<NodeId, ()>::with_key();
        let group = group_ids.insert(());
        let mut panel_ids = SlotMap::<PanelId, ()>::with_key();
        let panel = panel_ids.insert(());
        let mut session = super::DragHostSession::new(
            DragPayload::Group { group },
            DragHandle::GroupGrip,
            super::DragSourceHost::Main,
            None,
        );
        let drag_host = DragHostId::from_raw(21);

        assert_eq!(session.payload(), DragPayload::Group { group });
        assert!(session.can_begin_group_view_transfer(group, panel));
        assert!(!session.begin_group_view_transfer(group, panel));

        assert!(session.attach_transport_host(drag_host));
        assert!(session.begin_group_view_transfer(group, panel));
        assert_eq!(session.transferred_panel(), Some(panel));
        assert!(!session.can_begin_group_view_transfer(group, panel));
        assert!(!session.begin_group_view_transfer(group, panel));
        assert_eq!(session.payload(), DragPayload::Group { group });
    }

    #[test]
    fn dock_window_roles_keep_dragging_separate_from_floating_groups() {
        let drag_id = DragHostId::from_raw(7);
        let dragging = DockWindowRole::Dragging { id: drag_id };

        assert_eq!(dragging, DockWindowRole::Dragging { id: drag_id });
        assert_ne!(
            dragging,
            DockWindowRole::Floating {
                group: FloatingGroupId::null(),
            }
        );
    }
}

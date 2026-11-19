//! Runtime identity and role types for dock-related native windows.

#[path = "dock_window_role.rs"]
mod dock_window_role;
#[path = "drag_host_id.rs"]
mod drag_host_id;
#[path = "drag_host_session.rs"]
mod drag_host_session;
#[path = "drag_source_host.rs"]
mod drag_source_host;

pub use dock_window_role::DockWindowRole;
pub use drag_host_id::DragHostId;
pub use drag_host_session::DragHostSession;
pub use drag_source_host::DragSourceHost;

#[cfg(test)]
mod tests {
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
    fn new_drag_host_session_has_no_detached_panel_views() {
        use dock_core::{DragHandle, DragPayload, NodeId, PanelId};

        let session = super::DragHostSession::new(
            DragPayload::Panel {
                panel: PanelId::null(),
            },
            DragHandle::PanelTab,
            super::DragSourceHost::Main,
            None,
        );

        assert!(session.transferred_panels().is_empty());
        assert!(!session.all_panel_views_source_detached());
    }

    #[test]
    fn drag_host_session_keeps_original_payload_and_source_separate_from_transport() {
        use dock_core::{DragHandle, DragPayload, PanelId};

        let mut panel_ids = slotmap::SlotMap::<PanelId, ()>::with_key();
        let panel = panel_ids.insert(());
        let other_panel = panel_ids.insert(());
        let source_group = dock_core::FloatingGroupId::null();
        let source_column = NodeId::null();
        let mut session = super::DragHostSession::new(
            DragPayload::Panel { panel },
            DragHandle::PanelTab,
            super::DragSourceHost::FloatingColumn { group: source_group, column: source_column },
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
    fn group_drag_transfers_every_tab_view_with_active_panel_first() {
        use dock_core::{DragHandle, DragPayload, NodeId, PanelId};
        use slotmap::SlotMap;

        let mut group_ids = SlotMap::<NodeId, ()>::with_key();
        let group = group_ids.insert(());
        let mut panel_ids = SlotMap::<PanelId, ()>::with_key();
        let active_panel = panel_ids.insert(());
        let inactive_panel = panel_ids.insert(());
        let panels = [active_panel, inactive_panel];
        let mut session = super::DragHostSession::new(
            DragPayload::Group { group },
            DragHandle::GroupGrip,
            super::DragSourceHost::Main,
            None,
        );
        let drag_host = DragHostId::from_raw(21);

        assert_eq!(session.payload(), DragPayload::Group { group });
        assert!(session.can_begin_group_view_transfer(group, &panels));
        assert!(!session.begin_group_view_transfer(group, &panels));

        assert!(session.attach_transport_host(drag_host));
        assert!(session.begin_group_view_transfer(group, &panels));
        assert_eq!(session.transferred_panel(), Some(active_panel));
        assert_eq!(session.transferred_panels(), &panels);
        assert!(!session.can_begin_group_view_transfer(group, &panels));
        assert!(!session.begin_group_view_transfer(group, &panels));
        assert_eq!(session.payload(), DragPayload::Group { group });
    }

    #[test]
    fn dock_area_drag_transfers_all_active_panel_views_as_one_session() {
        use dock_core::{DockSide, DragHandle, DragPayload, PanelId};
        use slotmap::SlotMap;

        let mut panel_ids = SlotMap::<PanelId, ()>::with_key();
        let first_panel = panel_ids.insert(());
        let second_panel = panel_ids.insert(());
        let panels = [first_panel, second_panel];
        let mut session = super::DragHostSession::new(
            DragPayload::DockArea { side: DockSide::Right },
            DragHandle::DockAreaHeader,
            super::DragSourceHost::Main,
            None,
        );
        let drag_host = DragHostId::from_raw(41);

        assert!(session.can_begin_dock_area_view_transfer(DockSide::Right, &panels));
        assert!(!session.begin_dock_area_view_transfer(DockSide::Right, &panels));
        assert!(session.attach_transport_host(drag_host));
        assert!(session.begin_dock_area_view_transfer(DockSide::Right, &panels));
        assert_eq!(session.transferred_panels(), &panels);
        assert!(!session.all_panel_views_source_detached());
        assert!(!session.can_begin_dock_area_view_transfer(DockSide::Right, &panels));
        assert!(!session.can_begin_dock_area_view_transfer(DockSide::Left, &panels));
        assert_eq!(
            session.payload(),
            DragPayload::DockArea { side: DockSide::Right }
        );
    }

    #[test]
    fn dock_window_roles_keep_dragging_separate_from_floating_groups() {
        let drag_id = DragHostId::from_raw(7);
        let dragging = DockWindowRole::Dragging { id: drag_id };

        assert_eq!(dragging, DockWindowRole::Dragging { id: drag_id });
        assert_ne!(
            dragging,
        );
        assert_ne!(
            dragging,
            DockWindowRole::FloatingColumn {
                group: FloatingGroupId::null(),
                column: dock_core::NodeId::null(),
            }
        );
    }
}

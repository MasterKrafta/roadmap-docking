use std::{collections::HashSet, rc::Rc};

use dock_core::{DockSide, DragPayload, Node, NodeId, PanelId};
use gpui::{Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div};

use crate::{
    components::{
        context_menu::GroupMenuToggle,
        floating::FloatingColumnHost,
        PanelHost,
    },
    dock_components::{collect_group_presentations, DockGroupView},
    DockController, DockTheme,
};

use super::{source_layout_rects, DragGroupPresentation};

pub(crate) struct DragHostSurface {
    pub(super) theme: DockTheme,
    pub(super) controller: Entity<DockController>,
    pub(super) controller_observed: bool,
    pub(super) panel: Option<PanelId>,
    pub(super) group: Option<NodeId>,
    pub(super) dock_area: Option<DockSide>,
    pub(super) render_detached_tab: bool,
    pub(super) handoff_placeholder: bool,
    /// When a GroupGrip or detached PanelTab creates a new Floating Column,
    /// this same HWND becomes its permanent host instead of being replaced.
    pub(super) promoted_column: Option<Entity<FloatingColumnHost>>,
}

impl DragHostSurface {
    pub(crate) fn set_promoted_column_close_button(
        &mut self,
        show: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(host) = self.promoted_column.as_ref() {
            let _ = host.update(cx, |host, host_cx| {
                if host.set_show_close_button(show) {
                    host_cx.notify();
                }
            });
        }
    }
}

impl Render for DragHostSurface {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.controller_observed {
            let controller = self.controller.clone();
            cx.observe(&controller, |_, _, host_cx| host_cx.notify())
                .detach();
            self.controller_observed = true;
        }

        let mut root = div()
            .relative()
            .size_full()
            .overflow_hidden();

        if let Some(promoted_column) = self.promoted_column.clone() {
            return root.child(promoted_column);
        }

        if self.handoff_placeholder || self.dock_area.is_some() {
            // Main DockArea drags are rendered by the temporary Column HWNDs.
            // This hidden DragHost remains only as the pointer/drop-tracking coordinator.
            return root;
        }

        if let Some(group) = self.group
            && let Some(snapshot) = self.controller.update(cx, |controller, _| {
                let (source_host, source_window_id, panel) = {
                    let session = controller.drag().host_session()?;
                    if !matches!(
                        session.payload(),
                        DragPayload::Group { group: active } if active == group
                    ) {
                        return None;
                    }
                    let panel = session.transferred_panel()?;
                    if !session.panel_view_source_detached(panel) {
                        return None;
                    }
                    (session.source_host(), session.source_window_id()?, panel)
                };
                let rects = source_layout_rects(controller, source_host, source_window_id)?;
                let group_rect = rects
                    .groups
                    .iter()
                    .find(|rect| rect.group == group)
                    .copied()?;
                let strip = rects
                    .tab_strips
                    .iter()
                    .find(|strip| strip.group == group)
                    .copied()?;
                let headers = rects
                    .tab_headers
                    .iter()
                    .filter(|header| header.group == group)
                    .copied()
                    .collect::<Vec<_>>();
                let overflow = rects
                    .tab_overflows
                    .iter()
                    .find(|overflow| overflow.group == group)
                    .cloned();
                let ids = HashSet::from([group]);
                let presentations = collect_group_presentations(
                    controller.layout(),
                    controller.registry(),
                    &ids,
                    Some(&rects),
                );
                let presentation = presentations.get(&group)?.clone();
                let panel_view = controller.panel_view_for_drag(panel);
                let panel_rect = rects
                    .panels
                    .iter()
                    .find(|rect| rect.panel == panel)
                    .copied();
                let panel = panel_rect.zip(panel_view);
                let column = controller.layout().nodes().find_map(|(id, node)| {
                    matches!(
                        node,
                        Node::Column(value) if value.groups.contains(&group)
                    )
                    .then_some(id)
                });
                Some(DragGroupPresentation {
                    group: group_rect,
                    strip,
                    headers,
                    overflow,
                    presentation,
                    column,
                    panel,
                    metrics: controller.metrics(),
                })
            })
        {
            // The temporary DragHost presents only the Group. The permanent
            // CompositionHeader is created after a successful Floating drop.
            let group_content_origin = snapshot.group.rect.origin;
            let toggle_menu: GroupMenuToggle = Rc::new(|_, _, _| {});
            root = root.child(DockGroupView::render_core(
                snapshot.column,
                snapshot.group,
                snapshot.strip,
                &snapshot.headers,
                snapshot.overflow,
                &snapshot.presentation,
                group_content_origin,
                snapshot.metrics,
                0.0,
                self.controller.clone(),
                self.theme,
                toggle_menu,
            ));
            if let Some(panel) = snapshot.panel {
                root = root.child(PanelHost::render(
                    vec![(panel.0, Some(panel.1))],
                    group_content_origin,
                    self.theme,
                ));
            }
        } else if let Some(panel) = self.panel
            && let Some((view, title, metrics, detached_group)) = self.controller.update(cx, |controller, _| {
                let view = controller.panel_view_for_drag(panel)?;
                let title = controller
                    .layout()
                    .panel_instance(panel)
                    .and_then(|instance| controller.registry().get(&instance.key))
                    .map(|descriptor| descriptor.title().to_string())
                    .unwrap_or_else(|| format!("Panel {panel:?}"));
                let metrics = controller.metrics();
                let detached_group = if self.render_detached_tab {
                    let group = controller.layout().nodes().find_map(|(id, node)| {
                        let Node::Tabs(tabs) = node else {
                            return None;
                        };
                        tabs.children
                            .iter()
                            .any(|child| {
                                matches!(
                                    controller.layout().node(*child),
                                    Some(Node::Panel(value)) if value.panel_id == panel
                                )
                            })
                            .then_some(id)
                    });
                    group.and_then(|group| {
                        let (source_host, source_window_id) = {
                            let session = controller.drag().host_session()?;
                            (session.source_host(), session.source_window_id()?)
                        };
                        let rects =
                            source_layout_rects(controller, source_host, source_window_id)?;
                        let group_rect = rects
                            .groups
                            .iter()
                            .find(|group_rect| group_rect.group == group)
                            .copied()?;
                        let strip = rects
                            .tab_strips
                            .iter()
                            .find(|strip| strip.group == group)
                            .copied()?;
                        let header = rects
                            .tab_headers
                            .iter()
                            .find(|header| header.group == group && header.panel == panel)
                            .copied()?;
                        let panel_rect = rects
                            .panels
                            .iter()
                            .find(|panel_rect| panel_rect.panel == panel)
                            .copied()?;
                        let column = controller.layout().nodes().find_map(|(id, node)| {
                            matches!(
                                node,
                                Node::Column(value) if value.groups.contains(&group)
                            )
                            .then_some(id)
                        });
                        let mut presentation = collect_group_presentations(
                            controller.layout(),
                            controller.registry(),
                            &HashSet::from([group]),
                            Some(&rects),
                        )
                        .remove(&group)?;
                        presentation.tabs.retain(|tab| tab.panel == panel);
                        presentation.tabs.first()?;
                        presentation.active_panel = Some(panel);
                        presentation.collapsed = false;
                        presentation.model_collapsed = false;
                        presentation.auto_collapsed = false;
                        presentation.auto_expanded = false;
                        Some((column, group_rect, strip, header, panel_rect, presentation))
                    })
                } else {
                    None
                };
                Some((view, title, metrics, detached_group))
            })
        {
            // Reuse the exact Core Group geometry. The permanent CompositionHeader
            // is not part of this DragHost and is created only after the Floating drop.
            if let Some((column, group, strip, header, panel_rect, presentation)) = detached_group {
                let toggle_menu: GroupMenuToggle = Rc::new(|_, _, _| {});
                root = root.child(DockGroupView::render_detached_tab(
                    column,
                    group,
                    strip,
                    header,
                    panel_rect,
                    &presentation,
                    view,
                    metrics,
                    self.controller.clone(),
                    self.theme,
                    toggle_menu,
                ));
            } else {
                // Flyout panels have no source Tabs Group. Keep their existing
                // standalone preview; PanelTab drags take the Core renderer above.
                let tab_surface = div()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col();
                root = root.child(tab_surface.child(DockGroupView::render_active(
                    format!("drag-tab-group-{panel:?}"),
                    title,
                    view,
                    metrics,
                    self.theme,
                )));
            }
        }

        root
    }
}

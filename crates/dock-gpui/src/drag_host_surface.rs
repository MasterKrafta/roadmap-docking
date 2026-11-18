use std::{collections::{HashMap, HashSet}, rc::Rc};

use dock_core::{
    ColumnRect, DockSide, DragPayload, GroupRect, Node, NodeId, PanelId, PanelRect,
    Size, SplitterRect,
    TabHeaderRect, TabOverflowRect, TabStripRect, VerticalOverflow,
};
use gpui::{Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div};

use crate::{
    components::{
        context_menu::GroupMenuToggle,
        floating::FloatingPartHost,
        MainDockView,
        PanelHost,
    },
    dock_components::{collect_group_presentations, DockGroupView},
    DockController, DockTheme, DragSourceHost,
};

use super::{
    dock_area_bounds, source_layout_rects, translate_rect,
    DragDockAreaPresentation, DragGroupPresentation,
};

pub(crate) struct DragHostSurface {
    pub(super) theme: DockTheme,
    pub(super) controller: Entity<DockController>,
    pub(super) controller_observed: bool,
    pub(super) panel: Option<PanelId>,
    pub(super) group: Option<NodeId>,
    pub(super) dock_area: Option<DockSide>,
    pub(super) render_detached_tab: bool,
    pub(super) drag_content_size: Size,
    pub(super) handoff_placeholder: bool,
    /// When a Main-origin GroupGrip or detached PanelTab succeeds, this same
    /// HWND becomes the permanent Floating Group host instead of being replaced.
    pub(super) promoted_part: Option<Entity<FloatingPartHost>>,
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

        if let Some(promoted_part) = self.promoted_part.clone() {
            return root.child(promoted_part);
        }

        if self.handoff_placeholder || self.dock_area.is_some() {
            // Main DockArea drags are rendered by the temporary Header/Group HWNDs.
            // This hidden DragHost remains only as the pointer/drop-tracking coordinator.
            return root;
        }

        if let Some(side) = self.dock_area
            && let Some(snapshot) = self.controller.update(cx, |controller, _| {
                let (root, panels) = {
                    let session = controller.drag().host_session()?;
                    if session.source_host() != DragSourceHost::Main
                        || !matches!(
                            session.payload(),
                            DragPayload::DockArea { side: active } if active == side
                        )
                        || !session.all_panel_views_source_detached()
                    {
                        return None;
                    }
                    (
                        controller.layout().main_area(side)?.root,
                        session.transferred_panels().to_vec(),
                    )
                };

                let mut node_ids = HashSet::new();
                let mut column_ids = HashSet::new();
                let mut column_groups = HashMap::new();
                crate::dock_host::collect_subtree_nodes(
                    controller.layout(),
                    root,
                    &mut node_ids,
                    &mut column_ids,
                    &mut column_groups,
                );

                let rects = controller.geometry().ok()?.clone();
                let source_columns = rects
                    .columns
                    .iter()
                    .filter(|column| column_ids.contains(&column.column))
                    .copied()
                    .collect::<Vec<_>>();
                let area_rect = dock_area_bounds(&source_columns)?;
                let columns = source_columns
                    .iter()
                    .map(|column| {
                        ColumnRect::new(
                            column.column,
                            translate_rect(column.rect, area_rect.origin),
                        )
                    })
                    .collect::<Vec<_>>();
                let group_ids = column_groups
                    .values()
                    .flat_map(|groups| groups.iter().copied())
                    .collect::<HashSet<_>>();
                let groups = rects
                    .groups
                    .iter()
                    .filter(|group| group_ids.contains(&group.group))
                    .map(|group| {
                        GroupRect::new(
                            group.group,
                            translate_rect(group.rect, area_rect.origin),
                        )
                    })
                    .collect::<Vec<_>>();
                let tab_strips = rects
                    .tab_strips
                    .iter()
                    .filter(|strip| group_ids.contains(&strip.group))
                    .map(|strip| {
                        TabStripRect::new(
                            strip.group,
                            translate_rect(strip.rect, area_rect.origin),
                        )
                    })
                    .collect::<Vec<_>>();
                let tab_headers = rects
                    .tab_headers
                    .iter()
                    .filter(|header| group_ids.contains(&header.group))
                    .map(|header| {
                        TabHeaderRect::new(
                            header.group,
                            header.panel,
                            header.index,
                            translate_rect(header.rect, area_rect.origin),
                        )
                    })
                    .collect::<Vec<_>>();
                let tab_overflows = rects
                    .tab_overflows
                    .iter()
                    .filter(|overflow| group_ids.contains(&overflow.group))
                    .map(|overflow| {
                        TabOverflowRect::new(
                            overflow.group,
                            translate_rect(overflow.button_rect, area_rect.origin),
                            overflow.hidden_tabs.clone(),
                        )
                    })
                    .collect::<Vec<_>>();
                let presentations = collect_group_presentations(
                    controller.layout(),
                    controller.registry(),
                    &node_ids,
                    Some(&rects),
                );
                let panel_views = rects
                    .panels
                    .iter()
                    .filter(|panel| panels.contains(&panel.panel))
                    .filter_map(|panel| {
                        let view = controller.panel_view_for_drag(panel.panel)?;
                        Some((
                            PanelRect::new(
                                panel.panel,
                                translate_rect(panel.rect, area_rect.origin),
                            ),
                            Some(view),
                        ))
                    })
                    .collect::<Vec<_>>();
                let column_modes = column_ids
                    .iter()
                    .filter_map(|column| match controller.layout().node(*column) {
                        Some(Node::Column(value)) => Some((*column, value.mode)),
                        _ => None,
                    })
                    .collect::<HashMap<_, _>>();
                let rail_display_orders = column_modes
                    .iter()
                    .filter(|(_, mode)| mode.is_rail())
                    .filter_map(|(column, _)| {
                        controller
                            .layout()
                            .column_display_order(*column)
                            .ok()
                            .map(|order| (*column, order))
                    })
                    .collect::<HashMap<_, _>>();
                let splitters = rects
                    .splitters
                    .iter()
                    .filter(|splitter| node_ids.contains(&splitter.owner))
                    .map(|splitter| {
                        SplitterRect::new(
                            splitter.owner,
                            splitter.boundary_index,
                            splitter.axis,
                            translate_rect(splitter.rect, area_rect.origin),
                        )
                    })
                    .collect::<Vec<_>>();
                let vertical_overflow = rects
                    .vertical_overflow
                    .iter()
                    .filter(|overflow| column_ids.contains(&overflow.owner))
                    .map(|overflow| {
                        VerticalOverflow::new(
                            overflow.owner,
                            translate_rect(overflow.viewport, area_rect.origin),
                            translate_rect(overflow.content_extent, area_rect.origin),
                        )
                    })
                    .collect::<Vec<_>>();
                let node_sides = node_ids
                    .iter()
                    .map(|node| (*node, side))
                    .collect::<HashMap<_, _>>();
                let column_sides = column_ids
                    .iter()
                    .map(|column| (*column, side))
                    .collect::<HashMap<_, _>>();

                Some(DragDockAreaPresentation {
                    columns,
                    groups,
                    panels: panel_views,
                    tab_strips,
                    tab_headers,
                    tab_overflows,
                    presentations,
                    rail_display_orders,
                    node_ids,
                    node_sides,
                    column_sides,
                    column_groups,
                    column_modes,
                    splitters,
                    vertical_overflow,
                    metrics: controller.metrics(),
                })
            })
        {
            let toggle_menu: GroupMenuToggle = Rc::new(|_, _, _| {});
            root = root.child(MainDockView::render(
                &snapshot.columns,
                &snapshot.groups,
                snapshot.panels,
                None,
                None,
                &snapshot.tab_strips,
                &snapshot.tab_headers,
                &snapshot.tab_overflows,
                &snapshot.presentations,
                &snapshot.rail_display_orders,
                None,
                None,
                &[],
                snapshot.metrics,
                None,
                toggle_menu,
                true,
                &snapshot.splitters,
                None,
                &snapshot.vertical_overflow,
                &snapshot.node_ids,
                &snapshot.node_sides,
                &snapshot.column_sides,
                &snapshot.column_groups,
                &snapshot.column_modes,
                &HashMap::new(),
                self.controller.clone(),
                self.theme,
                true,
            ));
        } else if let Some(group) = self.group
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
                        let mut presentation = collect_group_presentations(
                            controller.layout(),
                            controller.registry(),
                            &HashSet::from([group]),
                            None,
                        )
                        .remove(&group)?;
                        presentation.tabs.retain(|tab| tab.panel == panel);
                        presentation.tabs.first()?;
                        let (source_host, source_window_id) = {
                            let session = controller.drag().host_session()?;
                            (session.source_host(), session.source_window_id()?)
                        };
                        let rects =
                            source_layout_rects(controller, source_host, source_window_id)?;
                        let source_header = rects
                            .tab_headers
                            .iter()
                            .find(|header| header.group == group && header.panel == panel)
                            .copied()?;
                        presentation.active_panel = Some(panel);
                        presentation.collapsed = false;
                        presentation.model_collapsed = false;
                        presentation.auto_collapsed = false;
                        presentation.auto_expanded = false;
                        Some((source_header, presentation))
                    })
                } else {
                    None
                };
                Some((view, title, metrics, detached_group))
            })
        {
            // A detached tab uses the shared Group renderer without drawing a
            // CompositionHeader in the temporary DragHost.
            if let Some((source_header, presentation)) = detached_group {
                let toggle_menu: GroupMenuToggle = Rc::new(|_, _, _| {});
                root = root.child(DockGroupView::render_detached_tab(
                    source_header,
                    self.drag_content_size,
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

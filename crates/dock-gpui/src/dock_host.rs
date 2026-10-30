use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use dock_core::{
    ColumnMode, ComputeRectsError, DockSide, HitResult, Layout, Node, NodeId, Point, Rect,
    Size as CoreSize,
};
use gpui::{
    AnyView, App, Bounds, Context, Entity, IntoElement, ParentElement, Pixels, Render, ScrollHandle,
    Styled, Window, canvas, div, px, rgb,
};
use ui::Theme;

use crate::{
    DockController,
    components::{
        GroupMenuToggle, MainDockView, PanelHost, TabOverflowToggle, collect_group_presentations,
    },
    input::scroll_adjusted_hit_test_point,
};

/// Root composition for an application's docking workspace.
///
/// The application's main menu remains outside this view. The toolbar,
/// Options bar, and document canvas are supplied by the application as GPUI
/// views; their visibility follows the current Core chrome mode.
///
/// Layer order is deliberate: canvas, main dock surfaces, flyout, then
/// floating groups in ascending Core z-order. DropOverlay, DragGhost, and
/// context-menu layers are added after the corresponding components land.
pub struct DockHost {
    controller: Entity<DockController>,
    theme: Theme,
    toolbar: Option<AnyView>,
    options_bar: Option<AnyView>,
    canvas: Option<AnyView>,
    /// GPUI scroll positions are transient and keyed by the Core owner NodeId.
    column_scroll_handles: HashMap<NodeId, ScrollHandle>,
    /// Currently opened group menu; menu visibility is adapter-only state.
    open_group_menu: Option<NodeId>,
    /// Currently opened list of tabs hidden by a tab-strip overflow button.
    open_tab_overflow: Option<NodeId>,
    /// Whether middle-clicking a tab closes its panel instance.
    middle_click_closes_tabs: bool,
}

impl DockHost {
    /// Starts a host around the application's single DockController entity.
    pub fn new(controller: Entity<DockController>, theme: Theme) -> Self {
        Self {
            controller,
            theme,
            toolbar: None,
            options_bar: None,
            canvas: None,
            column_scroll_handles: HashMap::new(),
            open_group_menu: None,
            open_tab_overflow: None,
            middle_click_closes_tabs: true,
        }
    }

    /// Supplies the application's toolbar slot.
    ///
    /// The slot is rendered only when Core reports
    /// `ChromeMode::toolbar_visible()`.
    pub fn toolbar(mut self, view: impl Into<AnyView>) -> Self {
        self.toolbar = Some(view.into());
        self
    }

    /// Supplies the application's Options bar slot.
    ///
    /// The slot is rendered only when Core reports
    /// `ChromeMode::options_bar_visible()`.
    pub fn options_bar(mut self, view: impl Into<AnyView>) -> Self {
        self.options_bar = Some(view.into());
        self
    }

    /// Supplies the user-owned document canvas slot.
    ///
    /// When omitted, the canvas region remains a neutral theme surface.
    pub fn canvas(mut self, view: impl Into<AnyView>) -> Self {
        self.canvas = Some(view.into());
        self
    }

    /// Enables or disables closing the clicked panel when a tab receives a middle click.
    ///
    /// Enabled by default, following the common desktop-tab interaction convention.
    pub fn middle_click_closes_tabs(mut self, enabled: bool) -> Self {
        self.middle_click_closes_tabs = enabled;
        self
    }

    /// Returns the controller entity used by this host.
    pub fn controller(&self) -> &Entity<DockController> {
        &self.controller
    }

    /// Returns the theme currently used by the host and its base surfaces.
    pub fn theme(&self) -> Theme {
        self.theme
    }

    /// Maps a pointer point in a scrolled main column back to Core geometry.
    ///
    /// Call this before DockController::hit_test for points inside the given
    /// column. The layout rectangles themselves remain unchanged.
    pub fn scroll_adjusted_hit_test_point(&self, column: NodeId, point: Point) -> Point {
        let offset_y = self
            .column_scroll_handles
            .get(&column)
            .map(|handle| handle.offset().y.as_f32())
            .unwrap_or(0.0);
        scroll_adjusted_hit_test_point(point, offset_y)
    }

    /// Hit-tests a pointer position after translating it out of a scrolled
    /// main-column viewport and back into the immutable Core coordinate space.
    pub fn hit_test(
        &self,
        point: Point,
        app: &mut App,
    ) -> Result<HitResult, ComputeRectsError> {
        let overflow_owner = self.controller.update(app, |controller, _| {
            controller.geometry().map(|rects| {
                rects.vertical_overflow.iter().find_map(|overflow| {
                    (self.column_scroll_handles.contains_key(&overflow.owner)
                        && overflow.viewport.contains(point).unwrap_or(false))
                    .then_some(overflow.owner)
                })
            })
        })?;

        let content_point = overflow_owner
            .map(|owner| self.scroll_adjusted_hit_test_point(owner, point))
            .unwrap_or(point);

        self.controller
            .update(app, |controller, _| controller.hit_test(content_point))
    }
}

impl Render for DockHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // One read transaction produces the geometry snapshot plus the IDs/modes
        // needed by the main-dock presentation layer. The Layout remains Core-owned.
        let (
            chrome,
            rects,
            main_node_ids,
            main_column_ids,
            main_column_groups,
            column_modes,
            group_presentations,
            metrics,
        ) = self.controller.update(cx, |controller, _| {
                let rects = controller.geometry().cloned();
                let layout = controller.layout();
                let chrome = layout.chrome();
                let mut main_node_ids = HashSet::new();
                let mut main_column_ids = HashSet::new();
                let mut main_column_groups = HashMap::new();

                for side in [DockSide::Left, DockSide::Right] {
                    if let Some(area) = layout.main_area(side) {
                        collect_main_nodes(
                            layout,
                            area.root,
                            &mut main_node_ids,
                            &mut main_column_ids,
                            &mut main_column_groups,
                        );
                    }
                }

                let column_modes = main_column_ids
                    .iter()
                    .filter_map(|id| match layout.node(*id) {
                        Some(Node::Column(column)) => Some((*id, column.mode)),
                        _ => None,
                    })
                    .collect::<HashMap<_, _>>();
                let group_presentations =
                    collect_group_presentations(layout, controller.registry(), &main_node_ids);
                let metrics = controller.metrics();

                (
                    chrome,
                    rects,
                    main_node_ids,
                    main_column_ids,
                    main_column_groups,
                    column_modes,
                    group_presentations,
                    metrics,
                )
            });
        let rects = match rects {
            Ok(rects) => Some(rects),
            Err(error) => {
                tracing::warn!(%error, "dock host could not obtain layout geometry");
                None
            }
        };

        // Main-column panels are mounted by ColumnView so their content moves
        // with that column's own scroll handle. The remaining panel rectangles
        // belong to floating compositions and are mounted in workspace space.
        let main_active_panels = main_column_groups
            .values()
            .flat_map(|groups| groups.iter())
            .filter_map(|group| {
                group_presentations
                    .get(group)
                    .and_then(|presentation| presentation.active_panel)
            })
            .collect::<HashSet<_>>();
        let floating_panels = rects
            .as_ref()
            .map(|rects| {
                rects
                    .panels
                    .iter()
                    .filter(|panel| !main_active_panels.contains(&panel.panel))
                    .copied()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // Keep scroll state only while its Column node still owns an overflow
        // region. Removing/replacing the Core node or resolving the overflow
        // drops the associated GPUI handle and its transient scroll position.
        if let Some(rects) = rects.as_ref() {
            let overflow_columns = rects
                .vertical_overflow
                .iter()
                .filter(|overflow| main_column_ids.contains(&overflow.owner))
                .map(|overflow| overflow.owner)
                .collect::<HashSet<_>>();
            self.column_scroll_handles.retain(|owner, _| {
                main_column_ids.contains(owner) && overflow_columns.contains(owner)
            });
            for owner in overflow_columns {
                self.column_scroll_handles
                    .entry(owner)
                    .or_insert_with(ScrollHandle::new);
            }
        }

        if self
            .open_group_menu
            .is_some_and(|group| !group_presentations.contains_key(&group))
        {
            self.open_group_menu = None;
        }
        if self.open_tab_overflow.is_some_and(|group| {
            !group_presentations.contains_key(&group)
                || !rects.as_ref().is_some_and(|rects| {
                    rects.tab_overflows.iter().any(|overflow| overflow.group == group)
                })
        }) {
            self.open_tab_overflow = None;
        }

        let host_entity = cx.entity().downgrade();
        let toggle_group_menu: GroupMenuToggle = Rc::new(move |group, app| {
            if let Some(host) = host_entity.upgrade() {
                let _ = host.update(app, |host, host_cx| {
                    host.open_group_menu = if host.open_group_menu == Some(group) {
                        None
                    } else {
                        host.open_tab_overflow = None;
                        Some(group)
                    };
                    host_cx.notify();
                });
            }
        });

        let overflow_host_entity = cx.entity().downgrade();
        let toggle_tab_overflow: TabOverflowToggle = Rc::new(move |group, app| {
            if let Some(host) = overflow_host_entity.upgrade() {
                let _ = host.update(app, |host, host_cx| {
                    host.open_tab_overflow = if host.open_tab_overflow == Some(group) {
                        None
                    } else {
                        host.open_group_menu = None;
                        Some(group)
                    };
                    host_cx.notify();
                });
            }
        });

        let middle_click_closes_tabs = self.middle_click_closes_tabs;
        let overflow_group = self.open_tab_overflow;
        let theme = self.theme;
        let controller_for_measurement = self.controller.clone();
        let measurement = canvas(
            move |bounds, _window, app| {
                let viewport = viewport_from_bounds(bounds);
                if viewport.validate_non_empty().is_err() {
                    return;
                }

                controller_for_measurement.update(app, |controller, controller_cx| {
                    match controller.set_viewport(viewport) {
                        Ok(true) => {
                            // geometry() performs a fresh valid calculation and updates
                            // Layout::canvas_bounds only when the measured Canvas rect changed.
                            if let Err(error) = controller.geometry() {
                                tracing::warn!(%error, "dock host geometry update failed");
                            }
                            controller_cx.notify();
                        }
                        Ok(false) => {}
                        Err(error) => {
                            tracing::warn!(%error, "dock host rejected measured viewport");
                        }
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme.colors.window_background));

        if chrome.toolbar_visible()
            && let Some(toolbar) = self.toolbar.clone()
        {
            root = root.child(
                div()
                    .w_full()
                    .flex_none()
                    .bg(rgb(theme.colors.toolbar_background))
                    .child(toolbar),
            );
        }

        if chrome.options_bar_visible()
            && let Some(options_bar) = self.options_bar.clone()
        {
            root = root.child(
                div()
                    .w_full()
                    .flex_none()
                    .bg(rgb(theme.colors.toolbar_background))
                    .child(options_bar),
            );
        }

        let mut workspace = div()
            .relative()
            .flex_1()
            .w_full()
            .bg(rgb(theme.colors.window_background))
            .overflow_hidden()
            .child(measurement);

        // Base canvas surface prevents an uninitialized/empty workspace from
        // exposing an undefined background while the first geometry pass runs.
        if let Some(canvas_rect) = rects.as_ref().and_then(|rects| rects.canvas) {
            let mut canvas_surface = div()
                .absolute()
                .left(px(canvas_rect.origin.x))
                .top(px(canvas_rect.origin.y))
                .w(px(canvas_rect.size.width))
                .h(px(canvas_rect.size.height))
                .bg(rgb(theme.colors.canvas_background));
            if let Some(canvas_view) = self.canvas.clone() {
                canvas_surface = canvas_surface.child(
                    div().size_full().child(canvas_view),
                );
            }
            workspace = workspace.child(canvas_surface);
        } else {
            let mut canvas_surface = div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(rgb(theme.colors.canvas_background));
            if let Some(canvas_view) = self.canvas.clone() {
                canvas_surface = canvas_surface.child(
                    div().size_full().child(canvas_view),
                );
            }
            workspace = workspace.child(canvas_surface);
        }

        if chrome.dock_visible()
            && let Some(rects) = rects.as_ref()
        {
            let main_columns = rects
                .columns
                .iter()
                .filter(|column| main_column_ids.contains(&column.column))
                .copied()
                .collect::<Vec<_>>();
            workspace = workspace.child(MainDockView::render(
                &main_columns,
                &rects.groups,
                &rects.panels,
                &rects.tab_strips,
                &rects.tab_headers,
                &rects.tab_overflows,
                &group_presentations,
                metrics,
                self.open_group_menu,
                toggle_group_menu,
                overflow_group,
                toggle_tab_overflow,
                middle_click_closes_tabs,
                &rects.splitters,
                &rects.vertical_overflow,
                &main_node_ids,
                &main_column_groups,
                &column_modes,
                &self.column_scroll_handles,
                self.controller.clone(),
                window,
                cx,
                theme,
            ));

            // Flyout is painted above the Canvas and main dock surfaces.
            if let Some(flyout) = rects.flyout {
                workspace = workspace.child(
                    div()
                        .absolute()
                        .left(px(flyout.rect.origin.x))
                        .top(px(flyout.rect.origin.y))
                        .w(px(flyout.rect.size.width))
                        .h(px(flyout.rect.size.height))
                        .bg(rgb(theme.colors.panel_surface_background)),
                );
            }

            // Later children paint above earlier ones; Core z_order is preserved.
            let mut floating_groups = rects.floating_groups.clone();
            floating_groups.sort_by_key(|group| group.z_order);
            for floating in floating_groups {
                workspace = workspace.child(
                    div()
                        .absolute()
                        .left(px(floating.rect.origin.x))
                        .top(px(floating.rect.origin.y))
                        .w(px(floating.rect.size.width))
                        .h(px(floating.rect.size.height))
                        .bg(rgb(theme.colors.panel_surface_background))
                        .border_1()
                        .border_color(rgb(theme.colors.group_frame_border)),
                );
            }

            // Main panels live in their scrolling columns. Only floating panels
            // remain here, above the floating-window surfaces.
            workspace = workspace.child(PanelHost::render(
                &floating_panels,
                Point::ZERO,
                self.controller.clone(),
                window,
                cx,
            ));
        }

        root.child(workspace)
    }
}

/// Collects only nodes reachable from the left/right main dock roots.
/// Floating compositions are deliberately rendered by their own later layer.
fn collect_main_nodes(
    layout: &Layout,
    id: NodeId,
    nodes: &mut HashSet<NodeId>,
    columns: &mut HashSet<NodeId>,
    column_groups: &mut HashMap<NodeId, Vec<NodeId>>,
) {
    if !nodes.insert(id) {
        return;
    }

    match layout.node(id) {
        Some(Node::Split(split)) => {
            for child in &split.children {
                collect_main_nodes(layout, *child, nodes, columns, column_groups);
            }
        }
        Some(Node::Column(column)) => {
            columns.insert(id);
            column_groups.insert(id, column.groups.clone());
            for group in &column.groups {
                collect_main_nodes(layout, *group, nodes, columns, column_groups);
            }
        }
        Some(Node::Tabs(tabs)) => {
            for child in &tabs.children {
                collect_main_nodes(layout, *child, nodes, columns, column_groups);
            }
        }
        Some(Node::Panel(_)) | None => {}
    }
}

/// Converts GPUI's logical bounds to the host-local Core viewport.
///
/// Core geometry is expressed relative to the DockHost, so the GPUI window
/// origin is intentionally not copied into the viewport origin.
fn viewport_from_bounds(bounds: Bounds<Pixels>) -> Rect {
    Rect::new(
        Point::ZERO,
        CoreSize::new(
            bounds.size.width.as_f32(),
            bounds.size.height.as_f32(),
        ),
    )
}

#[cfg(test)]
mod tests {
    use gpui::{Bounds, Point as GpuiPoint, Size as GpuiSize, px};

    use super::viewport_from_bounds;
    use dock_core::{Point, Rect, Size};

    #[test]
    fn viewport_uses_host_local_origin_and_logical_size() {
        let bounds = Bounds {
            origin: GpuiPoint::new(px(18.0), px(32.0)),
            size: GpuiSize::new(px(960.0), px(640.0)),
        };

        assert_eq!(
            viewport_from_bounds(bounds),
            Rect::new(Point::ZERO, Size::new(960.0, 640.0))
        );
    }
}

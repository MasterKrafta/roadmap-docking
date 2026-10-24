use dock_core::{
    create_initial_layout, DockCommand, DockMetrics, DockSide, DropTarget, DropZone, Layout,
    Node, NodeId, PanelId, Point, Profile, Rect, Size,
};
use gpui::{
    Context, IntoElement, ParentElement, Render, Styled, Window, div, px, rgb,
};
use ui::Theme;

use crate::{DockSurface, NoopHost, dock_metrics_from_theme};
use crate::dock::DockController;

/// Temporary picture-only host window used while the new surface architecture is validated.
///
/// Interaction handling and the command controller arrive in the following roadmap stages.
pub(crate) struct MainWindow {
    theme: Theme,
    dock: DockController,
}

impl MainWindow {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        let (layout, metrics) = build_initial_layout(&theme)?;
        Ok(Self {
            theme,
            dock: DockController::new(layout, metrics),
        })
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let window_size = window.viewport_size();
        let width = window_size.width.as_f32().max(1.0);
        let height = window_size.height.as_f32().max(1.0);
        let menu_height = self.theme.metrics.column_header_height.max(1.0);
        let content_height = (height - menu_height).max(1.0);
        let viewport = Rect::new(Point::ZERO, Size::new(width, content_height));

        let frame = match self.dock.frame(viewport) {
            Ok(frame) => frame,
            Err(error) => {
                return div()
                    .w(px(width))
                    .h(px(height))
                    .bg(rgb(self.theme.colors.window_background))
                    .text_color(rgb(self.theme.colors.primary_text))
                    .child(format!("Could not compute dock geometry: {error:?}"));
            }
        };
        for overflow in &frame.rects().vertical_overflow {
            self.dock
                .scroll_handles_mut()
                .entry(overflow.owner)
                .or_insert_with(gpui::ScrollHandle::new);
        }

        let mut host = NoopHost;
        div()
            .flex()
            .flex_col()
            .w(px(width))
            .h(px(height))
            .bg(rgb(self.theme.colors.window_background))
            .child(
                div()
                    .flex()
                    .items_center()
                    .w(px(width))
                    .h(px(menu_height))
                    .bg(rgb(self.theme.colors.toolbar_background))
                    .border_1()
                    .border_color(rgb(self.theme.colors.border))
                    .text_color(rgb(self.theme.colors.primary_text))
                    .child("Dockers ▾"),
            )
            .child(
                DockSurface::new(
                    self.dock.layout(),
                    &frame,
                    self.dock.scroll_handles(),
                    &self.theme,
                    None,
                )
                .build(&mut host),
            )
    }
}

fn build_initial_layout(theme: &Theme) -> Result<(Layout, DockMetrics), String> {
    let metrics = dock_metrics_from_theme(theme);
    let descriptors = crate::panels::descriptors(theme);
    let mut layout = create_initial_layout(Profile::Photoshop, &descriptors, metrics)
        .map_err(|error| format!("{error:?}"))?;

    let (single_tab_group, multi_tab_group) = {
        let area = layout
            .main_area(DockSide::Right)
            .ok_or_else(|| "Core factory did not create a right dock area".to_owned())?;
        let Some(Node::Column(column)) = layout.node(area.root) else {
            return Err("Core factory right dock root is not a Column".to_owned());
        };
        if column.groups.len() != 2 {
            return Err("Core factory did not create the expected two tab groups".to_owned());
        }
        (column.groups[0], column.groups[1])
    };

    let color_panel = panel_id_for_key(&layout, "color")?;
    let history_panel = panel_id_for_key(&layout, "history")?;

    layout
        .apply(
            DockCommand::MovePanel {
                panel: color_panel,
                target: DropTarget::Tabs {
                    group: multi_tab_group,
                    index: 0,
                },
            },
            &metrics,
        )
        .map_err(|error| format!("Could not place the second tab group: {error:?}"))?;

    layout
        .apply(
            DockCommand::MovePanel {
                panel: history_panel,
                target: DropTarget::GroupEdge {
                    group: multi_tab_group,
                    zone: DropZone::Bottom,
                },
            },
            &metrics,
        )
        .map_err(|error| format!("Could not create the lower group: {error:?}"))?;

    layout
        .apply(
            DockCommand::MoveGroupToMainColumn {
                group: single_tab_group,
                side: DockSide::Right,
                index: 0,
            },
            &metrics,
        )
        .map_err(|error| format!("Could not create the adjacent right column: {error:?}"))?;

    layout
        .validate(&metrics)
        .map_err(|errors| format!("Constructed Core layout is invalid: {errors:?}"))?;

    Ok((layout, metrics))
}

fn panel_id_for_key(layout: &Layout, key: &str) -> Result<PanelId, String> {
    layout
        .panel_instances()
        .find_map(|(id, instance)| (instance.key.as_str() == key).then_some(id))
        .ok_or_else(|| format!("Core layout does not contain the panel '{key}'"))
}

#[cfg(test)]
mod tests {
    use super::build_initial_layout;
    use ui::Theme;

    #[test]
    fn initial_render_layout_is_valid() {
        let (layout, metrics) =
            build_initial_layout(&Theme::default()).expect("initial layout should be valid");
        assert_eq!(layout.validate(&metrics), Ok(()));
        assert_eq!(layout.panel_instances().count(), 4);
    }
}

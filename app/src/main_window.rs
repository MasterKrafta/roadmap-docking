use dock_core::{
    create_initial_layout, DockCommand, DockMetrics, Layout, Point, Profile, Rect, Size,
};
use gpui::{
    App, ClickEvent, Context, IntoElement, ParentElement, Render, Styled, Window, div, px, rgb,
};
use ui::Theme;

use crate::panels::PanelRegistry;

/// The app is the only owner of the live Layout and applies adapter-returned commands.
pub(crate) struct MainWindow {
    layout: Layout,
    theme: Theme,
    metrics: DockMetrics,
    panels: PanelRegistry,
}

impl MainWindow {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        let panels = PanelRegistry::default();
        let metrics = dock_gpui::dock_metrics_from_theme(&theme);
        let descriptors = panels.core_descriptors();
        let layout = create_initial_layout(Profile::Photoshop, &descriptors, metrics)
            .map_err(str::to_owned)?;

        Ok(Self {
            layout,
            theme,
            metrics,
            panels,
        })
    }

    fn apply_command(&mut self, command: DockCommand, cx: &mut Context<Self>) {
        match self.layout.apply(command, &self.metrics) {
            Ok(_) => cx.notify(),
            Err(error) => eprintln!("Dock command rejected by Core: {error:?}"),
        }
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let viewport = Rect::new(
            Point::ZERO,
            Size::new(viewport.width.0.max(1.0), viewport.height.0.max(1.0)),
        );

        let rects = match self.layout.compute_rects(viewport, &self.metrics) {
            Ok(rects) => rects,
            Err(error) => {
                return div()
                    .w(px(viewport.size.width))
                    .h(px(viewport.size.height))
                    .bg(rgb(self.theme.colors.window_background))
                    .text_color(rgb(self.theme.colors.primary_text))
                    .child(format!("Dock geometry error: {error:?}"));
            }
        };

        let panel_registry = &self.panels;
        let theme = &self.theme;
        let weak_window = cx.weak_entity();
        dock_gpui::render(
            &self.layout,
            &rects,
            theme,
            |instance| panel_registry.render(instance, theme),
            |instance| panel_registry.title(instance),
            move |command| {
                let weak_window = weak_window.clone();
                Box::new(move |_event: &ClickEvent, _window: &mut Window, app: &mut App| {
                    let _ = weak_window.update(app, |view, cx| {
                        view.apply_command(command.clone(), cx);
                    });
                }) as dock_gpui::CommandClickHandler
            },
        )
    }
}

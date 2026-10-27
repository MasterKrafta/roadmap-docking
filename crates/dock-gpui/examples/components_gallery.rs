//! Visual gallery for the stateless dock GPUI components.
//!
//! Run on the Windows target with:
//! `cargo run -p dock-gpui --example components_gallery`
//!
//! All callbacks are intentionally no-ops: this window previews component styling,
//! keyboard focus, labels and hit targets without creating a DockController or Layout.

#[cfg(windows)]
mod windows_gallery {
use dock_core::{
    Axis, ColumnMode, FloatingGroupId, NodeId, PanelId, Point, Rect, ResizeEdge, Size,
};
use gpui::{
    App, ClickEvent, Context, IntoElement, InteractiveElement, KeyDownEvent, MouseDownEvent, ParentElement,
    Render, StatefulInteractiveElement, Styled, Window, div, px, rgb,
};
use slotmap::SlotMap;
use ui::Theme;

use dock_gpui::components::{
    DropZone, FloatingTitle, FlyoutHeader, GroupFrame, GroupGrip, GroupMenuButton, IconButton,
    PanelSurface, PanelTab, RailEntry, RectBox, ResizeEdgeHandle, ResizeEdgeKind, SplitterHandle,
    TabOverflowButton, TabStrip, with_focus_ring,
};

const WINDOW_WIDTH: f32 = 1360.0;
const WINDOW_HEIGHT: f32 = 960.0;
const CARD_WIDTH: f32 = 420.0;
const CARD_HEIGHT: f32 = 138.0;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
type KeyDownHandler = Box<dyn Fn(&KeyDownEvent, &mut Window, &mut App) + 'static>;
type MouseDownHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

struct Gallery {
    theme: Theme,
    group: NodeId,
    panel: PanelId,
    floating_group: FloatingGroupId,
}

impl Gallery {
    fn new(theme: Theme) -> Self {
        // Generate valid, stable keys for this gallery entity. The components only
        // use these IDs to make GPUI element identities; no layout tree is required.
        let mut groups = SlotMap::<NodeId, ()>::with_key();
        let group = groups.insert(());

        let mut panels = SlotMap::<PanelId, ()>::with_key();
        let panel = panels.insert(());

        let mut floating_groups = SlotMap::<FloatingGroupId, ()>::with_key();
        let floating_group = floating_groups.insert(());

        Self {
            theme,
            group,
            panel,
            floating_group,
        }
    }
}

impl Render for Gallery {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let group = self.group;
        let panel = self.panel;
        let floating_group = self.floating_group;

        div()
            .relative()
            .w(px(WINDOW_WIDTH))
            .h(px(WINDOW_HEIGHT))
            .bg(rgb(theme.colors.window_background))
            .child(
                div()
                    .absolute()
                    .left(px(16.0))
                    .top(px(4.0))
                    .text_color(rgb(theme.colors.primary_text))
                    .child("Dock components gallery — focus a control with Tab"),
            )
            .child(card(
                "RectBox — background + child",
                16.0,
                24.0,
                theme,
                body(
                    RectBox::new(rect(12.0, 44.0, 200.0, 62.0), theme.colors.active_tab).child(
                        div()
                            .absolute()
                            .left(px(10.0))
                            .top(px(20.0))
                            .text_color(rgb(theme.colors.primary_text))
                            .child("RectBox content"),
                    ),
                ),
            ))
            .child(card(
                "with_focus_ring — keyboard focus",
                452.0,
                24.0,
                theme,
                body(
                    with_focus_ring(
                        div()
                            .absolute()
                            .left(px(12.0))
                            .top(px(48.0))
                            .w(px(260.0))
                            .h(px(32.0))
                            .bg(rgb(theme.colors.toolbar_background))
                            .text_color(rgb(theme.colors.primary_text))
                            .child("Focus me with Tab")
                            .id("gallery-focus-ring"),
                        &theme,
                    )
                    .on_click(noop_click()),
                ),
            ))
            .child(card(
                "IconButton — close glyph",
                888.0,
                24.0,
                theme,
                body(IconButton::new(
                    "gallery-icon-button",
                    rect(12.0, 44.0, 34.0, 30.0),
                    "×",
                    &theme,
                    noop_click(),
                )),
            ))
            .child(card(
                "PanelTab — active, idle and long label",
                16.0,
                180.0,
                theme,
                body(
                    div()
                        .child(PanelTab {
                            id: "gallery-panel-tab-active".to_owned(),
                            rect: rect(12.0, 44.0, 176.0, 30.0),
                            label: "Active tab".to_owned(),
                            active: true,
                            theme,
                            on_activate: noop_click(),
                            on_activate_key: noop_key_down(),
                            on_reorder_key: noop_key_down(),
                            on_collapse_toggle: noop_click(),
                            on_drag_start: noop_mouse_down(),
                        })
                        .child(PanelTab {
                            id: "gallery-panel-tab-idle".to_owned(),
                            rect: rect(196.0, 44.0, 212.0, 30.0),
                            label: "Long properties tab label".to_owned(),
                            active: false,
                            theme,
                            on_activate: noop_click(),
                            on_activate_key: noop_key_down(),
                            on_reorder_key: noop_key_down(),
                            on_collapse_toggle: noop_click(),
                            on_drag_start: noop_mouse_down(),
                        }),
                ),
            ))
            .child(card(
                "TabStrip — background with tabs",
                452.0,
                180.0,
                theme,
                body(
                    div()
                        .child(TabStrip {
                            rect: rect(12.0, 43.0, 396.0, 34.0),
                            theme,
                        })
                        .child(PanelTab {
                            id: "gallery-strip-tab-a".to_owned(),
                            rect: rect(14.0, 45.0, 170.0, 30.0),
                            label: "Layers".to_owned(),
                            active: true,
                            theme,
                            on_activate: noop_click(),
                            on_activate_key: noop_key_down(),
                            on_reorder_key: noop_key_down(),
                            on_collapse_toggle: noop_click(),
                            on_drag_start: noop_mouse_down(),
                        })
                        .child(PanelTab {
                            id: "gallery-strip-tab-b".to_owned(),
                            rect: rect(186.0, 45.0, 170.0, 30.0),
                            label: "History".to_owned(),
                            active: false,
                            theme,
                            on_activate: noop_click(),
                            on_activate_key: noop_key_down(),
                            on_reorder_key: noop_key_down(),
                            on_collapse_toggle: noop_click(),
                            on_drag_start: noop_mouse_down(),
                        })
                        .child(TabOverflowButton {
                            group,
                            rect: rect(366.0, 46.0, 28.0, 28.0),
                            theme,
                            on_open: noop_click(),
                        }),
                ),
            ))
            .child(card(
                "TabOverflowButton — hidden tabs",
                888.0,
                180.0,
                theme,
                body(TabOverflowButton {
                    group,
                    rect: rect(12.0, 44.0, 30.0, 30.0),
                    theme,
                    on_open: noop_click(),
                }),
            ))
            .child(card(
                "GroupMenuButton — menu request",
                16.0,
                336.0,
                theme,
                body(GroupMenuButton {
                    group,
                    panel: Some(panel),
                    rect: rect(12.0, 44.0, 34.0, 30.0),
                    theme,
                    on_open: noop_click(),
                }),
            ))
            .child(card(
                "GroupGrip — drag hit target",
                452.0,
                336.0,
                theme,
                body(
                    div()
                        .child(GroupGrip {
                            group,
                            rect: rect(12.0, 44.0, 396.0, 4.0),
                            theme,
                            on_drag_start: noop_mouse_down(),
                        })
                        .child(
                            div()
                                .absolute()
                                .left(px(12.0))
                                .top(px(57.0))
                                .text_color(rgb(theme.colors.muted_text))
                                .child("4 px group drag grip above"),
                        ),
                ),
            ))
            .child(card(
                "GroupFrame — outer border",
                888.0,
                336.0,
                theme,
                body(GroupFrame {
                    rect: rect(12.0, 43.0, 396.0, 76.0),
                    theme,
                }),
            ))
            .child(card(
                "PanelSurface — content host",
                16.0,
                492.0,
                theme,
                body(PanelSurface {
                    rect: rect(12.0, 43.0, 396.0, 78.0),
                    theme,
                    content: div()
                        .absolute()
                        .left(px(12.0))
                        .top(px(20.0))
                        .text_color(rgb(theme.colors.primary_text))
                        .child("Panel body preview")
                        .into_any_element(),
                }),
            ))
            .child(card(
                "SplitterHandle — resize band",
                452.0,
                492.0,
                theme,
                body(
                    div()
                        .child(SplitterHandle {
                            id: "gallery-splitter".to_owned(),
                            rect: rect(192.0, 43.0, 6.0, 78.0),
                            axis: Axis::X,
                            theme,
                            on_resize_start: Some(noop_mouse_down()),
                        })
                        .child(
                            div()
                                .absolute()
                                .left(px(18.0))
                                .top(px(56.0))
                                .text_color(rgb(theme.colors.muted_text))
                                .child("Drag the vertical splitter band"),
                        ),
                ),
            ))
            .child(card(
                "ResizeEdgeHandle — floating edge",
                888.0,
                492.0,
                theme,
                body(ResizeEdgeHandle {
                    id: "gallery-floating-resize".to_owned(),
                    rect: rect(384.0, 48.0, 18.0, 62.0),
                    edge: ResizeEdgeKind::Floating(ResizeEdge::Left),
                    theme,
                    on_resize_start: noop_mouse_down(),
                }),
            ))
            .child(card(
                "FloatingTitle — move + dock capture + close/collapse",
                16.0,
                648.0,
                theme,
                body(FloatingTitle {
                    group: floating_group,
                    rect: rect(12.0, 44.0, 396.0, 32.0),
                    title: "Floating window — long title preview".to_owned(),
                    theme,
                    on_drag_start: noop_mouse_down(),
                    on_capture_drag_start: Some(noop_mouse_down()),
                    on_close: Some(noop_click()),
                    on_collapse: Some(noop_click()),
                    collapse_label: "−",
                }),
            ))
            .child(card(
                "RailEntry — labels and compact icons",
                452.0,
                648.0,
                theme,
                body(
                    div()
                        .child(RailEntry {
                            id: "gallery-rail-labeled".to_owned(),
                            rect: rect(12.0, 43.0, 396.0, 28.0),
                            mode: ColumnMode::IconsAndLabels,
                            icon: "◉".to_owned(),
                            title: "Color panel with a longer name".to_owned(),
                            active: true,
                            theme,
                            on_open: noop_click(),
                            on_open_key: noop_key_down(),
                        })
                        .child(RailEntry {
                            id: "gallery-rail-icons".to_owned(),
                            rect: rect(12.0, 75.0, 56.0, 28.0),
                            mode: ColumnMode::IconsOnly,
                            icon: "▤".to_owned(),
                            title: "Layers".to_owned(),
                            active: false,
                            theme,
                            on_open: noop_click(),
                            on_open_key: noop_key_down(),
                        }),
                ),
            ))
            .child(card(
                "FlyoutHeader — menu and close",
                888.0,
                648.0,
                theme,
                body(FlyoutHeader {
                    group,
                    panel,
                    rect: rect(12.0, 44.0, 396.0, 32.0),
                    title: "Flyout panel title".to_owned(),
                    theme,
                    on_menu: noop_click(),
                    on_close: noop_click(),
                }),
            ))
            .child(card(
                "DropZone — idle and active target",
                16.0,
                804.0,
                theme,
                body(
                    div()
                        .child(DropZone {
                            rect: rect(12.0, 44.0, 188.0, 72.0),
                            active: false,
                            theme,
                        })
                        .child(DropZone {
                            rect: rect(208.0, 44.0, 200.0, 72.0),
                            active: true,
                            theme,
                        })
                        .child(
                            div()
                                .absolute()
                                .left(px(30.0))
                                .top(px(68.0))
                                .text_color(rgb(theme.colors.primary_text))
                                .child("Idle"),
                        )
                        .child(
                            div()
                                .absolute()
                                .left(px(278.0))
                                .top(px(68.0))
                                .text_color(rgb(theme.colors.primary_text))
                                .child("Active"),
                        ),
                ),
            ))
    }
}

fn card(
    title: &'static str,
    x: f32,
    y: f32,
    theme: Theme,
    content: impl IntoElement,
) -> impl IntoElement {
    div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(CARD_WIDTH))
        .h(px(CARD_HEIGHT))
        .bg(rgb(theme.colors.group_background))
        .border_1()
        .border_color(rgb(theme.colors.border))
        .child(
            div()
                .absolute()
                .left(px(12.0))
                .top(px(8.0))
                .text_color(rgb(theme.colors.primary_text))
                .child(title),
        )
        .child(content)
}

fn body(content: impl IntoElement) -> impl IntoElement {
    div()
        .absolute()
        .left(px(0.0))
        .top(px(0.0))
        .w(px(CARD_WIDTH))
        .h(px(CARD_HEIGHT))
        .child(content)
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect::new(Point::new(x, y), Size::new(width, height))
}

fn noop_click() -> ClickHandler {
    Box::new(|_, _, _| {})
}

fn noop_key_down() -> KeyDownHandler {
    Box::new(|_, _, _| {})
}

fn noop_mouse_down() -> MouseDownHandler {
    Box::new(|_, _, _| {})
}

pub(super) fn run() {
    use gpui::{AppContext, Bounds, WindowBounds, WindowOptions, px, size};
    use gpui_platform::application;

    application().run(|cx: &mut App| {
        let theme = Theme::default();
        let gallery = Gallery::new(theme);
        let bounds = Bounds::centered(None, size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };

        if let Err(error) = cx.open_window(options, move |_window, cx| {
            cx.new(move |_| gallery)
        }) {
            eprintln!("Could not open the components gallery: {error}");
            return;
        }

        cx.activate(true);
    });
}
}

#[cfg(windows)]
fn main() {
    windows_gallery::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("components_gallery currently targets Windows 10 x64 (MSVC).");
}

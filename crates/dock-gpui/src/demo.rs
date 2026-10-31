//! A scrollable gallery of the dock-gpui presentation components.
//!
//! The gallery uses real dock-core layouts for the expanded dock, rail/flyout,
//! and floating-group examples. It is intentionally separate from the normal
//! application entry point.

pub fn run() {
    #[cfg(windows)]
    windows_demo::run();

    #[cfg(not(windows))]
    eprintln!("The dock-gpui component demo currently uses the Windows GPUI backend.");
}

#[cfg(windows)]
mod windows_demo {
    use std::collections::{HashMap, HashSet};
    use std::rc::Rc;

    use dock_core::{
        Axis, ColumnDisplayItem, ColumnMode, ColumnRect, DockCommand, DockMetrics, DockSide,
        DragPayload, DragPreview, DropSource, DropZone, DropZoneRect, FloatingGroupId,
        FloatingGroupRect, FlyoutRect, FlyoutState, GroupRect, Layout, Node, NodeId, PanelConstraints,
        PanelId, PanelKey, PanelRect, Point, Profile, Rect, Size, SplitterRect, TabHeaderRect,
        TabOverflowRect, TabStripRect, VerticalOverflow, create_initial_layout,
    };
    use gpui::{
        AnyView, App, AppContext, Bounds, Context, Entity, IntoElement, Render, ScrollHandle,
        SharedString, TitlebarOptions, Window, WindowBounds, WindowOptions, div, px, rgb, size,
    };
    use gpui::prelude::*;
    use gpui_platform::application;
    use ui::Theme;

    use crate::{
        DockController, IconSource, PanelDescriptor, PanelRegistry,
        components::{
            canvas_slot::CanvasSlot,
            column::ColumnView,
            context_menu::{
                ContextMenu, ContextMenuCallbacks, ContextMenuEntry, ContextMenuItem,
                ContextMenuTarget, GroupMenuToggle,
            },
            drop_overlay::{DragGhost, DropOverlay},
            floating::FloatingWindowView,
            flyout::FlyoutView,
            group::GroupView,
            main_dock::MainDockView,
            panel_host::PanelHost,
            rail::RailView,
            splitter::SplitterView,
            tab_overflow::TabOverflow,
            tab_strip::{GroupPresentation, TabHeaderView, TabStripView, collect_group_presentations},
        },
    };

    const DEMO_WIDTH: f32 = 1100.0;
    const DEMO_HEIGHT: f32 = 480.0;
    const WINDOW_WIDTH: f32 = 1240.0;
    const WINDOW_HEIGHT: f32 = 900.0;

    pub(super) fn run() {
        application().run(|cx: &mut App| {
            let metrics = DockMetrics::default();
            let viewport = Rect::new(
                Point::ZERO,
                Size::new(DEMO_WIDTH, DEMO_HEIGHT),
            );

            let registry = demo_registry();
            let layout = create_initial_layout(
                Profile::Photoshop,
                &registry.descriptors_for_core(),
                metrics,
            )
            .expect("the component demo layout must satisfy the Photoshop baseline");

            let main_column = layout
                .main_area(DockSide::Right)
                .expect("the demo has a right main column")
                .root;
            let groups = match layout.node(main_column) {
                Some(Node::Column(column)) => column.groups.clone(),
                _ => panic!("the demo's right main area must be a Column"),
            };
            let layers_panel = layout
                .panel_instances()
                .find_map(|(panel, instance)| {
                    (instance.key.as_str() == "layers").then_some(panel)
                })
                .expect("the demo layout contains Layers");

            let mut rail_layout = layout.clone();
            rail_layout
                .apply(
                    DockCommand::SetColumnMode {
                        column: main_column,
                        mode: ColumnMode::IconsAndLabels,
                    },
                    &metrics,
                )
                .expect("the rail example switches the column to icon mode");
            rail_layout
                .apply(
                    DockCommand::OpenFlyout {
                        column: main_column,
                        panel: layers_panel,
                    },
                    &metrics,
                )
                .expect("the rail example opens the Layers flyout");

            let mut floating_layout = layout.clone();
            floating_layout
                .apply(
                    DockCommand::Float {
                        node: groups[1],
                        bounds: Rect::new(
                            Point::new(360.0, 88.0),
                            Size::new(470.0, 300.0),
                        ),
                    },
                    &metrics,
                )
                .expect("the floating example detaches the second tab group");

            let main_controller = cx.new(|_| {
                DockController::new(layout, metrics, registry, viewport)
                    .expect("the main demo controller is valid")
            });
            let rail_controller = cx.new(|_| {
                DockController::new(rail_layout, metrics, demo_registry(), viewport)
                    .expect("the rail demo controller is valid")
            });
            let floating_controller = cx.new(|_| {
                DockController::new(floating_layout, metrics, demo_registry(), viewport)
                    .expect("the floating demo controller is valid")
            });

            let bounds = Bounds::centered(
                None,
                size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)),
                cx,
            );
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Dock GPUI — Component demo".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };

            let _window = cx
                .open_window(options, move |_window, cx| {
                    cx.new(|_| {
                        DemoView::new(
                            main_controller,
                            rail_controller,
                            floating_controller,
                        )
                    })
                })
                .expect("the dock component demo window opens");

            cx.activate(true);
        });
    }

    fn demo_registry() -> PanelRegistry {
        let mut registry = PanelRegistry::new();
        let constraints = PanelConstraints::new(
            Size::new(120.0, 80.0),
            Size::new(280.0, 220.0),
            None,
            false,
        );

        for (key, title, color) in [
            ("layers", "Layers", 0x40566F),
            ("color", "Color", 0x65506F),
            ("properties", "Properties", 0x526B5C),
            ("history", "History", 0x725C4B),
        ] {
            let descriptor = PanelDescriptor::new(
                PanelKey::new(key).expect("demo panel keys are valid"),
                title,
                IconSource::None,
                constraints,
                move |_window, cx| {
                    cx.new(|_| DemoPanel { title, color }).into()
                },
            );
            registry
                .register(descriptor)
                .expect("demo panel keys are unique");
        }

        registry
    }

    struct DemoPanel {
        title: &'static str,
        color: u32,
    }

    impl Render for DemoPanel {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(8.0))
                .p(px(16.0))
                .bg(rgb(self.color))
                .text_color(rgb(0xFFFFFF))
                .child(self.title)
                .child("Demo panel content")
        }
    }

    struct DemoView {
        main: Entity<DockController>,
        rail: Entity<DockController>,
        floating: Entity<DockController>,
        scroll: ScrollHandle,
    }

    impl DemoView {
        fn new(
            main: Entity<DockController>,
            rail: Entity<DockController>,
            floating: Entity<DockController>,
        ) -> Self {
            Self {
                main,
                rail,
                floating,
                scroll: ScrollHandle::new(),
            }
        }
    }

    struct DemoSnapshot {
        rects: dock_core::LayoutRects,
        metrics: DockMetrics,
        group_presentations: HashMap<NodeId, GroupPresentation>,
        column_groups: HashMap<NodeId, Vec<NodeId>>,
        main_column_groups: HashMap<NodeId, Vec<NodeId>>,
        column_modes: HashMap<NodeId, ColumnMode>,
        rail_display_orders: HashMap<NodeId, Vec<ColumnDisplayItem>>,
        main_node_ids: HashSet<NodeId>,
        main_columns: Vec<ColumnRect>,
        panel_views: HashMap<PanelId, AnyView>,
        active_flyout: Option<FlyoutState>,
        floating_titles: HashMap<FloatingGroupId, String>,
        floating_root: Option<NodeId>,
        floating_node_ids: HashSet<NodeId>,
        floating_panel_ids: HashSet<PanelId>,
        drop_zone: Option<DropZoneRect>,
        drag_preview: Option<DragPreview>,
    }

    fn capture_snapshot(
        entity: &Entity<DockController>,
        window: &mut Window,
        cx: &mut Context<DemoView>,
    ) -> DemoSnapshot {
        entity.update(cx, |controller, app| {
            let rects = controller
                .geometry()
                .expect("demo geometry is valid")
                .clone();
            let metrics = controller.metrics();

            let (
                group_presentations,
                column_groups,
                main_column_groups,
                column_modes,
                rail_display_orders,
                main_node_ids,
                main_columns,
                active_flyout,
                floating_root,
                floating_node_ids,
                floating_panel_ids,
                drop_zone,
                drag_preview,
            ) = {
                let layout = controller.layout();
                let all_node_ids = layout.nodes().map(|(id, _)| id).collect::<HashSet<_>>();
                let group_presentations =
                    collect_group_presentations(layout, controller.registry(), &all_node_ids);

                let mut main_node_ids = HashSet::new();
                for side in [DockSide::Left, DockSide::Right] {
                    if let Some(area) = layout.main_area(side) {
                        collect_node_ids(layout, area.root, &mut main_node_ids);
                    }
                }

                let mut column_groups = HashMap::new();
                let mut main_column_groups = HashMap::new();
                let mut column_modes = HashMap::new();
                let mut rail_display_orders = HashMap::new();
                for (id, node) in layout.nodes() {
                    if let Node::Column(column) = node {
                        column_groups.insert(id, column.groups.clone());
                        column_modes.insert(id, column.mode);
                        if main_node_ids.contains(&id) {
                            main_column_groups.insert(id, column.groups.clone());
                        }
                        if column.mode.is_rail() {
                            rail_display_orders.insert(
                                id,
                                layout.column_display_order(id).unwrap_or_default(),
                            );
                        }
                    }
                }

                let main_columns = rects
                    .columns
                    .iter()
                    .filter(|column| main_node_ids.contains(&column.column))
                    .copied()
                    .collect::<Vec<_>>();

                let floating_root = rects.floating_groups.first().and_then(|floating| {
                    layout
                        .floating_group(floating.group)
                        .and_then(|group| layout.dock_area(group.area))
                        .map(|area| area.root)
                });
                let mut floating_node_ids = HashSet::new();
                if let Some(root) = floating_root {
                    collect_node_ids(layout, root, &mut floating_node_ids);
                }
                let floating_panel_ids = layout
                    .nodes()
                    .filter_map(|(id, node)| {
                        if !floating_node_ids.contains(&id) {
                            return None;
                        }
                        match node {
                            Node::Panel(panel) => Some(panel.panel_id),
                            _ => None,
                        }
                    })
                    .collect::<HashSet<_>>();

                let drop_choice = layout
                    .panel_instances()
                    .next()
                    .and_then(|(panel, _)| {
                        let resolver = layout
                            .prepare_drop_resolver(
                                DropSource::Panel { panel },
                                &rects,
                                &metrics,
                            )
                            .ok()?;
                        let zone = resolver
                            .zones()
                            .iter()
                            .copied()
                            .find(|zone| {
                                matches!(
                                    zone.zone,
                                    DropZone::Center
                                        | DropZone::Left
                                        | DropZone::Right
                                        | DropZone::Top
                                        | DropZone::Bottom
                                )
                            })?;
                        Some((panel, zone))
                    });

                let (drop_zone, drag_preview) = match drop_choice {
                    Some((panel, zone)) => {
                        let point = Point::new(
                            zone.rect.origin.x + zone.rect.size.width * 0.5,
                            zone.rect.origin.y + zone.rect.size.height * 0.5,
                        );
                        (
                            Some(zone),
                            Some(DragPreview {
                                payload: DragPayload::Panel { panel },
                                point,
                                target: Some(zone.target),
                            }),
                        )
                    }
                    None => (None, None),
                };

                (
                    group_presentations,
                    column_groups,
                    main_column_groups,
                    column_modes,
                    rail_display_orders,
                    main_node_ids,
                    main_columns,
                    layout.flyout(),
                    floating_root,
                    floating_node_ids,
                    floating_panel_ids,
                    drop_zone,
                    drag_preview,
                )
            };

            let panel_ids = controller
                .layout()
                .panel_instances()
                .map(|(panel, _)| panel)
                .collect::<Vec<_>>();
            let panel_views = panel_ids
                .into_iter()
                .filter_map(|panel| {
                    controller
                        .panel_view(panel, window, app)
                        .map(|view| (panel, view))
                })
                .collect();

            let floating_titles = rects
                .floating_groups
                .iter()
                .map(|floating| (floating.group, "Floating group".to_owned()))
                .collect();

            DemoSnapshot {
                rects,
                metrics,
                group_presentations,
                column_groups,
                main_column_groups,
                column_modes,
                rail_display_orders,
                main_node_ids,
                main_columns,
                panel_views,
                active_flyout,
                floating_titles,
                floating_root,
                floating_node_ids,
                floating_panel_ids,
                drop_zone,
                drag_preview,
            }
        })
    }

    fn collect_node_ids(layout: &Layout, root: NodeId, ids: &mut HashSet<NodeId>) {
        if !ids.insert(root) {
            return;
        }

        match layout.node(root) {
            Some(Node::Split(split)) => {
                for child in &split.children {
                    collect_node_ids(layout, *child, ids);
                }
            }
            Some(Node::Tabs(tabs)) => {
                for child in &tabs.children {
                    collect_node_ids(layout, *child, ids);
                }
            }
            Some(Node::Column(column)) => {
                for group in &column.groups {
                    collect_node_ids(layout, *group, ids);
                }
            }
            Some(Node::Panel(_)) | None => {}
        }
    }

    fn panel_views_for(
        snapshot: &DemoSnapshot,
        rects: &[PanelRect],
    ) -> Vec<(PanelRect, AnyView)> {
        rects
            .iter()
            .filter_map(|rect| {
                snapshot
                    .panel_views
                    .get(&rect.panel)
                    .cloned()
                    .map(|view| (*rect, view))
            })
            .collect()
    }

    fn demo_section(
        title: &'static str,
        description: &'static str,
        preview_height: f32,
        preview: impl IntoElement,
        theme: Theme,
    ) -> impl IntoElement {
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(9.0))
            .p(px(14.0))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .bg(rgb(theme.colors.panel_surface_background))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(px(17.0))
                            .text_color(rgb(theme.colors.primary_text))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(theme.colors.muted_text))
                            .child(description),
                    ),
            )
            .child(
                div()
                    .relative()
                    .w(px(DEMO_WIDTH - 28.0))
                    .h(px(preview_height))
                    .overflow_hidden()
                    .bg(rgb(theme.colors.window_background))
                    .child(preview),
            )
    }

    impl Render for DemoView {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let main = capture_snapshot(&self.main, window, cx);
            let rail = capture_snapshot(&self.rail, window, cx);
            let floating = capture_snapshot(&self.floating, window, cx);
            let theme = Theme::default();

            let group_menu: GroupMenuToggle = Rc::new(|_target, _point, _app| {});

            let canvas_rect = main.rects.canvas.unwrap_or(Rect::new(
                Point::ZERO,
                Size::new(DEMO_WIDTH, DEMO_HEIGHT),
            ));
            let mut content = div()
                .w(px(DEMO_WIDTH))
                .flex()
                .flex_col()
                .gap(px(18.0))
                .p(px(18.0));

            content = content.child(demo_section(
                "CanvasSlot",
                "Позиционирование Canvas и область предпросмотра FreeArea.",
                300.0,
                CanvasSlot::render(canvas_rect, None, None, theme),
                theme,
            ));

            let main_dock = MainDockView::render(
                &main.main_columns,
                &main.rects.groups,
                panel_views_for(&main, &main.rects.panels),
                &main.rects.tab_strips,
                &main.rects.tab_headers,
                &main.rects.tab_overflows,
                &main.group_presentations,
                &main.rail_display_orders,
                main.active_flyout,
                main.drag_preview,
                &main.rects.drop_zones,
                main.metrics,
                None,
                group_menu.clone(),
                true,
                &main.rects.splitters,
                &main.rects.vertical_overflow,
                &main.main_node_ids,
                &main.main_column_groups,
                &main.column_modes,
                &HashMap::new(),
                self.main.clone(),
                theme,
            );
            content = content.child(demo_section(
                "MainDockView",
                "Основной dock host: колонки, группы, вкладки и сплиттеры.",
                DEMO_HEIGHT,
                main_dock,
                theme,
            ));

            let column = main
                .main_columns
                .first()
                .copied()
                .expect("the main demo has one visible column");
            let column_group_ids = main
                .main_column_groups
                .get(&column.column)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let column_groups = main
                .rects
                .groups
                .iter()
                .filter(|group| column_group_ids.contains(&group.group))
                .copied()
                .collect::<Vec<_>>();
            let active_panels = column_group_ids
                .iter()
                .filter_map(|group| {
                    main.group_presentations
                        .get(group)
                        .and_then(|presentation| presentation.active_panel)
                })
                .collect::<HashSet<_>>();
            let column_panels = main
                .rects
                .panels
                .iter()
                .filter(|panel| active_panels.contains(&panel.panel))
                .copied()
                .collect::<Vec<_>>();
            let column_panels = panel_views_for(&main, &column_panels);
            let column_splitters = main
                .rects
                .splitters
                .iter()
                .filter(|splitter| splitter.owner == column.column && splitter.axis == Axis::Y)
                .copied()
                .collect::<Vec<_>>();
            let column_mode = main
                .column_modes
                .get(&column.column)
                .copied()
                .unwrap_or(ColumnMode::Expanded);
            let column_view = ColumnView::render(
                column,
                column_mode,
                &column_groups,
                column_panels,
                &main.rects.tab_strips,
                &main.rects.tab_headers,
                &main.rects.tab_overflows,
                &main.group_presentations,
                main.rail_display_orders
                    .get(&column.column)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                main.active_flyout,
                main.drag_preview,
                &main.rects.drop_zones,
                main.metrics,
                None,
                group_menu.clone(),
                true,
                &column_splitters,
                main.rects
                    .vertical_overflow
                    .iter()
                    .find(|overflow| overflow.owner == column.column)
                    .copied(),
                None,
                self.main.clone(),
                theme,
                Point::ZERO,
                None,
            );
            content = content.child(demo_section(
                "ColumnView",
                "Одна колонка с её группами, содержимым панелей и внутренним splitter.",
                DEMO_HEIGHT,
                column_view,
                theme,
            ));

            let first_group = main
                .rects
                .groups
                .first()
                .copied()
                .expect("the main demo has tab groups");
            let first_strip = main
                .rects
                .tab_strips
                .iter()
                .find(|strip| strip.group == first_group.group)
                .copied()
                .expect("the first group has a tab strip");
            let first_headers = main
                .rects
                .tab_headers
                .iter()
                .filter(|header| header.group == first_group.group)
                .copied()
                .collect::<Vec<_>>();
            let first_presentation = main
                .group_presentations
                .get(&first_group.group)
                .expect("the first group has presentation metadata");
            let first_overflow = main
                .rects
                .tab_overflows
                .iter()
                .find(|overflow| overflow.group == first_group.group)
                .cloned();

            let group_view = GroupView::render(
                first_group,
                first_strip,
                &first_headers,
                first_overflow.clone(),
                first_presentation,
                Point::ZERO,
                main.metrics,
                main.metrics.group_control_button_width + 8.0,
                self.main.clone(),
                theme,
                None,
                group_menu.clone(),
                true,
            );
            content = content.child(demo_section(
                "GroupView",
                "Заголовок группы, ручка перетаскивания, tab strip и управляющие кнопки.",
                260.0,
                group_view,
                theme,
            ));

            let tab_strip = div()
                .absolute()
                .left(px(first_group.rect.origin.x))
                .top(px(first_group.rect.origin.y))
                .w(px(first_group.rect.size.width))
                .h(px(first_group.rect.size.height))
                .child(TabStripView::render(
                    first_group.group,
                    first_group.rect,
                    first_strip,
                    &first_headers,
                    first_overflow.clone(),
                    first_presentation,
                    self.main.clone(),
                    theme,
                    None,
                    true,
                ));
            content = content.child(demo_section(
                "TabStripView",
                "Поверхность полосы вкладок с активной и неактивной вкладками.",
                160.0,
                tab_strip,
                theme,
            ));

            let first_header = first_headers
                .first()
                .copied()
                .expect("the first group has a visible tab header");
            let first_tab = first_presentation
                .tabs
                .iter()
                .find(|tab| tab.panel == first_header.panel);
            let tab_title: SharedString = first_tab
                .map(|tab| tab.title.clone())
                .unwrap_or_else(|| "Demo tab".into());
            let tab_icon = first_tab.and_then(|tab| tab.icon.clone());
            let tab_header = div()
                .absolute()
                .left(px(first_strip.rect.origin.x))
                .top(px(first_strip.rect.origin.y))
                .w(px(first_strip.rect.size.width))
                .h(px(first_strip.rect.size.height))
                .child(TabHeaderView::render(
                    first_group.group,
                    first_header,
                    first_strip.rect,
                    tab_title,
                    tab_icon,
                    first_presentation.active_panel == Some(first_header.panel),
                    self.main.clone(),
                    theme,
                    None,
                    true,
                ));
            content = content.child(demo_section(
                "TabHeaderView",
                "Одна вкладка: заголовок, hover и кнопка закрытия.",
                100.0,
                tab_header,
                theme,
            ));

            let overflow_button = Rect::new(
                Point::new(
                    first_strip.rect.origin.x + first_strip.rect.size.width
                        - main.metrics.tab_overflow_button_width,
                    first_strip.rect.origin.y,
                ),
                Size::new(
                    main.metrics.tab_overflow_button_width,
                    first_strip.rect.size.height,
                ),
            );
            let overflow_sample = TabOverflowRect::new(
                first_group.group,
                overflow_button,
                first_headers
                    .iter()
                    .skip(1)
                    .map(|header| header.panel)
                    .collect(),
            );
            let tab_overflow = div()
                .absolute()
                .left(px(first_strip.rect.origin.x))
                .top(px(first_strip.rect.origin.y))
                .w(px(first_strip.rect.size.width))
                .h(px(first_strip.rect.size.height))
                .child(TabOverflow::render(first_strip.rect, overflow_sample, theme));
            content = content.child(demo_section(
                "TabOverflow",
                "Кнопка для доступа к скрытым вкладкам.",
                100.0,
                tab_overflow,
                theme,
            ));

            let panel_host = PanelHost::render(
                panel_views_for(&main, &main.rects.panels),
                Point::ZERO,
            );
            content = content.child(demo_section(
                "PanelHost",
                "Размещение активных GPUI panel views в Core-геометрии.",
                DEMO_HEIGHT,
                panel_host,
                theme,
            ));

            let splitter = main
                .rects
                .splitters
                .first()
                .copied()
                .expect("the two demo groups have a splitter");
            let splitter_view = SplitterView::render(
                splitter,
                Point::ZERO,
                Point::ZERO,
                self.main.clone(),
                main.metrics,
                theme,
                None,
            );
            content = content.child(demo_section(
                "SplitterView",
                "Визуальная ручка разделения групп.",
                DEMO_HEIGHT,
                splitter_view,
                theme,
            ));

            let rail_column = rail
                .main_columns
                .first()
                .copied()
                .expect("the rail example has a main column");
            let rail_mode = rail
                .column_modes
                .get(&rail_column.column)
                .copied()
                .unwrap_or(ColumnMode::IconsAndLabels);
            let rail_items = rail
                .rail_display_orders
                .get(&rail_column.column)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let rail_content = RailView::render(
                rail_column,
                rail_mode,
                rail_items,
                &rail.rects.tab_headers,
                &rail.group_presentations,
                rail_column.rect.origin,
                Point::ZERO,
                rail.active_flyout,
                self.rail.clone(),
                theme,
            );
            let rail_view = div()
                .absolute()
                .left(px(rail_column.rect.origin.x))
                .top(px(rail_column.rect.origin.y))
                .w(px(rail_column.rect.size.width))
                .h(px(rail_column.rect.size.height))
                .overflow_hidden()
                .child(rail_content);
            content = content.child(demo_section(
                "RailView",
                "Колонка в режиме иконок и подписей.",
                DEMO_HEIGHT,
                rail_view,
                theme,
            ));

            let flyout = rail
                .rects
                .flyout
                .expect("the rail example has an open flyout");
            let flyout_title = rail
                .group_presentations
                .get(&flyout.group)
                .and_then(|presentation| {
                    presentation
                        .tabs
                        .iter()
                        .find(|tab| tab.panel == flyout.panel)
                })
                .map(|tab| tab.title.clone())
                .unwrap_or_else(|| "Flyout panel".into());
            let flyout_panel = rail.panel_views.get(&flyout.panel).cloned();
            let flyout_view = FlyoutView::render(
                flyout,
                flyout_title,
                flyout_panel,
                self.rail.clone(),
                theme,
            );
            content = content.child(demo_section(
                "FlyoutView",
                "Всплывающая панель поверх Canvas, с заголовком и resize edge.",
                DEMO_HEIGHT,
                flyout_view,
                theme,
            ));

            let floating_rect = floating
                .rects
                .floating_groups
                .first()
                .copied()
                .expect("the floating example has a floating group");
            let floating_root = floating
                .floating_root
                .expect("the floating group has a root node");
            let floating_group = floating
                .rects
                .groups
                .iter()
                .find(|group| group.group == floating_root)
                .copied()
                .expect("the floating root is a Tabs group");
            let floating_strip = floating
                .rects
                .tab_strips
                .iter()
                .find(|strip| strip.group == floating_root)
                .copied()
                .expect("the floating group has a tab strip");
            let floating_presentation = floating
                .group_presentations
                .get(&floating_root)
                .expect("the floating group has presentation metadata");
            let floating_headers = floating
                .rects
                .tab_headers
                .iter()
                .filter(|header| header.group == floating_root)
                .copied()
                .collect::<Vec<_>>();
            let floating_overflows = floating
                .rects
                .tab_overflows
                .iter()
                .filter(|overflow| overflow.group == floating_root)
                .cloned()
                .collect::<Vec<_>>();
            let floating_panels = floating
                .rects
                .panels
                .iter()
                .filter(|panel| floating.floating_panel_ids.contains(&panel.panel))
                .filter_map(|panel| {
                    floating
                        .panel_views
                        .get(&panel.panel)
                        .cloned()
                        .map(|view| (*panel, view))
                })
                .collect::<Vec<_>>();
            let floating_edges = floating
                .rects
                .floating_resize_edges
                .iter()
                .filter(|edge| edge.group == floating_rect.group)
                .copied()
                .collect::<Vec<_>>();
            let floating_presentations =
                HashMap::from([(floating_root, floating_presentation.clone())]);
            let floating_view = FloatingWindowView::render(
                floating_rect,
                &floating_edges,
                &[],
                std::slice::from_ref(&floating_group),
                floating_panels,
                std::slice::from_ref(&floating_strip),
                &floating_headers,
                &floating_overflows,
                &floating_presentations,
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
                &[],
                &[],
                &HashMap::new(),
                None,
                None,
                &[],
                floating.metrics,
                "Floating group",
                None,
                group_menu.clone(),
                true,
                self.floating.clone(),
                theme,
            );
            content = content.child(demo_section(
                "FloatingWindowView",
                "Floating-композиция внутри окна приложения с заголовком и resize edges.",
                DEMO_HEIGHT,
                floating_view,
                theme,
            ));

            let drop_preview_background = MainDockView::render(
                &main.main_columns,
                &main.rects.groups,
                panel_views_for(&main, &main.rects.panels),
                &main.rects.tab_strips,
                &main.rects.tab_headers,
                &main.rects.tab_overflows,
                &main.group_presentations,
                &main.rail_display_orders,
                main.active_flyout,
                None,
                &[],
                main.metrics,
                None,
                group_menu.clone(),
                true,
                &main.rects.splitters,
                &main.rects.vertical_overflow,
                &main.main_node_ids,
                &main.main_column_groups,
                &main.column_modes,
                &HashMap::new(),
                self.main.clone(),
                theme,
            );
            let drop_overlay = div()
                .relative()
                .w(px(DEMO_WIDTH))
                .h(px(DEMO_HEIGHT))
                .child(drop_preview_background)
                .child(DropOverlay::render(
                    main.drop_zone,
                    main.drag_preview,
                    &main.rects.groups,
                    &main.rects.tab_headers,
                    &main.rects.tab_strips,
                    &main.rects.floating_groups,
                    theme,
                ));
            content = content.child(demo_section(
                "DropOverlay",
                "Подсветка выбранной Core drop-zone.",
                DEMO_HEIGHT,
                drop_overlay,
                theme,
            ));

            let drag_ghost = div()
                .relative()
                .w(px(DEMO_WIDTH))
                .h(px(240.0))
                .child(DragGhost::render(
                    main.drag_preview,
                    &main.group_presentations,
                    &main.floating_titles,
                    main.rects.viewport,
                    theme,
                ));
            content = content.child(demo_section(
                "DragGhost",
                "Визуальная карточка панели, следующая за drag pointer.",
                240.0,
                drag_ghost,
                theme,
            ));

            let context_entries = vec![
                ContextMenuItem::new("Activate tab", |_app| {}).into(),
                ContextMenuItem::new("Close panel", |_app| {})
                    .with_hotkey("Ctrl+W")
                    .into(),
                ContextMenuEntry::separator(),
                ContextMenuEntry::submenu(
                    "Move to",
                    None,
                    vec![
                        ContextMenuItem::new("Left dock", |_app| {}).into(),
                        ContextMenuItem::new("Right dock", |_app| {}).into(),
                    ],
                ),
            ];
            let context_menu = ContextMenu::render(
                "demo-context-menu",
                Rect::new(
                    Point::ZERO,
                    Size::new(DEMO_WIDTH, 230.0),
                ),
                Point::new(DEMO_WIDTH - 440.0, 28.0),
                &context_entries,
                Some(3),
                ContextMenuCallbacks {
                    on_submenu: Rc::new(|_index, _app| {}),
                    on_dismiss: Rc::new(|_app| {}),
                },
                theme,
            );
            content = content.child(demo_section(
                "ContextMenu",
                "Универсальное контекстное меню с раскрытым подменю.",
                250.0,
                context_menu,
                theme,
            ));

            div()
                .size_full()
                .bg(rgb(theme.colors.window_background))
                .child(
                    div()
                        .id("dock-component-demo-scroll")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .child(content),
                )
        }
    }
}

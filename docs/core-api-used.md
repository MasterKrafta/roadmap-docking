# Core API, используемый адаптером dock-gpui

Этот файл фиксирует фактические публичные типы и методы, которые используются текущим `crates/dock-gpui/src/render.rs`, но не описываются справочником команд `doc/dock-core-command-reference.md`. Это сверка по исходникам, а не контракт, выведенный по именам.

## Граница ответственности

- `dock-core` владеет моделью `Layout`, семантическими командами и вычислением геометрии. Этот крейт не зависит от GPUI.
- `dock-gpui/src/render.rs` принимает `&Layout` и уже рассчитанный `&LayoutRects`, создаёт GPUI-элементы и вызывает переданные обработчики. Сам рендерер не применяет команды и не клонирует `Layout`.
- Приложение передаёт фабрику контента панелей и обработчики команд, меню, начала drag и resize.

## Публичные методы `Layout`, используемые рендерером

| API | Фактическая сигнатура | Для чего используется |
| --- | --- | --- |
| `node` | `fn node(&self, id: NodeId) -> Option<&Node>` | Обход дерева, определение типов узлов, поиск вкладок и колонок |
| `panel_instance` | `fn panel_instance(&self, id: PanelId) -> Option<&PanelInstance>` | Получение данных панели по её ID |
| `nodes` | `fn nodes(&self) -> impl Iterator<Item = (NodeId, &Node)>` | Построение индексов узлов для отрисовки |
| `main_area` | `fn main_area(&self, side: DockSide) -> Option<&DockArea>` | Получение левой/правой главной dock-области |
| `dock_area` | `fn dock_area(&self, id: DockAreaId) -> Option<&DockArea>` | Получение корня плавающей или главной области |
| `floating_groups_in_paint_order` | `fn floating_groups_in_paint_order(&self) -> Vec<(FloatingGroupId, &FloatingGroup)>` | Рендер плавающих окон сзади вперёд по `z_order` |
| `column_display_order` | `fn column_display_order(&self, column_id: NodeId) -> Result<Vec<ColumnDisplayItem>, CommandError>` | Канонический порядок панелей в рейке: группы сверху вниз, вкладки слева направо |
| `flyout` | `const fn flyout(&self) -> Option<FlyoutState>` | Текущая временно открытая панель flyout |

Дополнительные методы `Layout`, доступные хозяину состояния и input-слою, но не являющиеся прямыми вызовами текущего `render.rs`:

- `floating_group(id) -> Option<&FloatingGroup>`;
- `panel_instances() -> impl Iterator<Item = (PanelId, &PanelInstance)>`;
- `floating_groups() -> impl Iterator<Item = (FloatingGroupId, &FloatingGroup)>`;
- `floating_groups_in_hit_test_order() -> Vec<(FloatingGroupId, &FloatingGroup)>`;
- `chrome() -> ChromeMode`, `canvas() -> &CanvasState`, `flyout_auto_collapse_enabled() -> bool`.


Точные объявления этих методов находятся в `crates/dock-core/src/layout.rs`; порядок рейки реализован в `crates/dock-core/src/layout/column_display_order.rs`.

## Поля `LayoutRects), используемые для кадра

`LayoutRects` — чистый снимок прямоугольников без GPUI-состояния. Определение находится в `crates/dock-core/src/layout_rects.rs`.

Текущий адаптер читает:

- `viewport`, `canvas`;
- `columns`, `groups`;
- `tab_strips`, `tab_headers`, `tab_overflows`;
- `panels`, `splitters`;
- `floating_groups`, `floating_resize_edges`;
- `flyout`, `vertical_overflow`.

Семантические поля сохраняются в записях геометрии: например, `SplitterRect` имеет `owner`, `boundary_index`, `axis`, `rect`; `FloatingResizeEdgeRect` имеет `group`, `edge`, `rect`; `FlyoutRect` содержит `column`, `group`, `panel`, `rect`, `resize_edge`; `VerticalOverflow` содержит `owner`, `viewport`, `content_extent`.

Рендерер не должен заново вычислять положения вкладок, сплиттеров, flyout или областей прокрутки из размеров UI, если нужный прямоугольник уже есть в снимке.

## Типы Core для жестов и drop

Эти типы не являются вариантами `DockCommand`; они описывают ввод, геометрию и семантику взаимодействия.

### Drag

- `DragHandle`: `PanelTab`, `FlyoutPanel`, `GroupGrip`, `FloatingTitle`, `FloatingCapture`.
- `DragPayload`: `Panel { panel }`, `Group { group }`, `FloatingGroup { group }`.
- `DragPreview`: `payload`, текущая точка `point`, разрешённая цель `target: Option<DropTarget>`.
- `DragSession` — UI-независимая машина состояний. Её публичный цикл:
  1. `DragSession::new(&DockMetrics)`;
  2. `pointer_down(payload, position)` или `pointer_down_with_handle(payload, handle, position)`;
  3. `pointer_move(position)`, который начинает drag при достижении порога из снимка `DockMetrics::drag_threshold`;
  4. `set_drop_target(Option<DropTarget>)` после повторного разрешения цели для текущей точки;
  5. `preview()` возвращает `Some(DragPreview)` только во время активного drag;
  6. `pointer_up(position, resolved_target)` возвращает `Result<Option<DragDropRequest>, DragSessionError>`. Обычный click ниже порога или отпускание без действующей цели не создаёт запроса.
- Отмена: `cancel()`, `escape()`, `focus_lost()`; `reset()` возвращает сессию в исходное состояние.
- `DragSession` не владеет и не меняет `Layout`; применение результата — ответственность хозяина состояния.

### Drop

- `DropTarget` описывает семантическое назначение: `Tabs`, `GroupTabs`, `GroupEdge`, `MainColumn`, `ColumnSlot`, `RailSlot`, `FreeArea`.
- `DropZone` описывает визуально/геометрически обнаруженную часть цели, включая `Center`, `TabInsertion`, `ColumnLeft/Right`, `RailInsertion`, `FloatingCenter/Top/Bottom` и другие варианты.
- `DropZoneRect { target, zone, rect }` связывает цель, тип зоны и её прямоугольник.
- `DragDropRequest` сохраняет `payload`, `handle`, `origin`, `point`, необязательный `target` и необязательные `floating_bounds` для detach в свободную область.
- `Layout::compute_drop_zones(source, rects, metrics) -> Result<Vec<DropZoneRect>, ComputeRectsError>` строит только допустимые цели на переданном снимке геометрии.
- `Layout::resolve_drag_drop(request, metrics) -> Result<DragDropAction, CommandError>` — чистое разрешение завершённого жеста. `DragDropAction::NoOp` означает, что команду отправлять нельзя; `DragDropAction::Command(command)` возвращает ровно одну `DockCommand`. Метод не меняет `Layout`.

Определения и реализация находятся в `crates/dock-core/src/drag_*.rs`, `drop_*.rs`, `crates/dock-core/src/compute_drop_zones.rs` и `crates/dock-core/src/layout/drag_drop.rs`.

### Resize

- `ResizeEdge`: `Left`, `Right`, `Top`, `Bottom`, `TopLeft`, `TopRight`, `BottomLeft`, `BottomRight`.
- `ResizeHandle` — тип адаптера из `dock-gpui/src/render.rs`:
  - `Splitter { owner, boundary_index, axis }`;
  - `OuterColumn { column, side }`;
  - `FloatingEdge { group, edge }`.
- Для resize Core предоставляет геометрию в `LayoutRects::splitters` и `LayoutRects::floating_resize_edges`; UI начинает жест от этих прямоугольников и передаёт семантический handle хозяину.

## Дополнительный публичный API `dock-gpui`

В `crates/dock-gpui/src/lib.rs` экспортируются:

- `render` и `normalize_docked_group_geometry`;
- `ResizeHandle`, `DockMenuRequest`, `DragOverlay`;
- типы обработчиков `CommandClickHandler`, `DragStartHandler`, `ResizeStartHandler`;
- `dock_metrics_from_theme`.

`DockMenuRequest` имеет варианты `TabOverflow { group }` и `PanelGroup { group, panel: Option<PanelId> }`. Это запрос к состоянию меню приложения, а не команда изменения Core.

Фактическая сигнатура `render` принимает `&Layout`, `&LayoutRects`, map `NodeId -> ScrollHandle`, `&Theme`, фабрику контента панели, callbacks заголовка/иконки и фабрики обработчиков команды/меню/начала drag/resize, а также `Option<&DragOverlay>`. Обработчики вызываются UI-элементами; сама функция не вызывает `Layout::apply`.

## Текущие ограничения адаптера, подтверждённые исходником

1. `supported_splitter_handle` в текущем `render.rs` поддерживает Y-границы колонок и X-границы только в корне главной dock-области. X-сплиттер внутри floating-композиции пока не выдаёт resize handle. Это отмеченный пробел адаптера, а не отсутствие команды ядра.
2. Ручки `floating_resize_edges` в текущем рендерере рисуются отдельным циклом после всех плавающих окон, поэтому порядок наложения ручек не привязан к отдельному окну.
3. `normalize_docked_group_geometry(layout, metrics, rects)` явно документирована как преобразование, вызываемое ровно один раз на свежем `LayoutRects`. Один и тот же нормализованный снимок должен использоваться для отрисовки и разрешения drop-целей, иначе визуальные прямоугольники и зоны стыковки могут разойтись.

Эти пункты — отправная точка для этапов 3, 6 и 9; они не меняют API `dock-core`.

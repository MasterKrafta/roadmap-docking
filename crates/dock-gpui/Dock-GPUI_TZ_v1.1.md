# ТЗ: dock-gpui — Dock-система в стиле Photoshop на dock-core + GPUI
Версия: 1.1 · Дата: 2026-10-30
Изменения 1.1: уточнены границы ответственности, запуск приложения и демо, контракт панели, версия GPUI, обработка floating/resize/Undo, кэш геометрии, прокрутка, persistence и приёмка.
Расположение: `dock-gpui/` (рядом с `dock-core/`)
---
## 1. Цель и границы
**Цель.** Создать крейт `dock-gpui` — GPUI-адаптер к UI-независимому ядру `dock-core`, который отрисовывает и обрабатывает ввод для docking-системы в стиле панелей Photoshop: колонки справа/слева, группы вкладок, сворачивание групп, рейки с иконками и flyout, плавающие окна, drag & drop с подсветкой зон, ресайз, сохранение/загрузка рабочей области, Undo/Redo.
**Принцип разделения (ключевой).**
- `dock-core` — единственный источник истины о структуре и геометрии. Вся логика (валидация, минимальные размеры, нормализация, правила drop, ограничения Photoshop-профиля) остаётся в ядре.
- `dock-gpui` — только: (1) рендер `LayoutRects` в GPUI-элементы, (2) трансляция событий GPUI в семантику ядра (`HitResult`, `DragSession`, `DockCommand`), (3) хост для содержимого панелей, (4) тема и ассеты.
- Адаптер **не хранит вторую копию дерева** и не пересчитывает геометрию сам.
**Вне области работ (v1).** Нативные OS-окна для floating-групп (плавающие панели рисуются внутри главного окна поверх канваса); содержимое конкретных панелей (Layers, Color и т.д. — только заглушки/демо); сам Canvas приложения; мульти-мониторное восстановление окон; tear-off в отдельное окно ОС.
**Правило запуска приложения и демо.**
- Обычный `app` запускается с пустым рабочим пространством: главное окно, принадлежащие приложению верхнее меню/toolbar и пустой слот Canvas. Никакие демонстрационные панели автоматически не создаются и старая раскладка не восстанавливается, пока приложение явно не включит persistence.
- `DockHost` не владеет главным меню приложения и не реализует сам Canvas. Он предоставляет слоты `toolbar`, `options_bar` и `canvas`; приложение решает, какие вьюхи передать.
- `photoshop_demo` самостоятельно создаёт начальную раскладку с четырьмя обязательными демонстрационными панелями. Это единственное место, где начальная Photoshop-композиция используется как демонстрационный стартовый экран.
- `generic_demo` создаёт минимальную раскладку `Profile::Generic` и демонстрирует повторное использование адаптера без Photoshop-специфичного поведения.
- Пустой `Layout::new(profile)` является допустимым состоянием. `create_initial_layout` не вызывается автоматически при создании обычного `DockHost`.
---
## 2. Исходные данные: что даёт dock-core
Ядро (≈37 тыс. строк, `#![forbid(unsafe_code)]`, фича `serde`) предоставляет:
| Область | Типы / функции |
|---|---|
| Состояние | `Layout` (`new`, `apply`, `version`, `profile`, `chrome`, `canvas`, `flyout`, `flyout_auto_collapse_enabled`, `node`, `panel_instance`, `dock_area`, `main_area`, `floating_groups*`, `column_display_order`, `last_dock_location`, `update_canvas_bounds`) |
| Дерево | `Node` (Split / Tabs / Panel / Column), `Column` (`width`, `mode`, `groups`, `group_ratios`), `Tabs` (`children`, `active_index`, `collapsed`), `FloatingGroup`, `DockArea`, `DockSide` |
| Идентификаторы | `NodeId`, `PanelId`, `DockAreaId`, `FloatingGroupId` (slotmap-ключи, **рантайм**), `PanelKey` (стабильный ключ вида `layers`, `asset-browser`) |
| Команды | `DockCommand` (≈35 вариантов: Open/Close/Activate/Reorder/Move/Merge/Split/Resize*/Float/Dock/Collapse/SetColumnMode/Flyout*/SetChrome) → `Layout::apply` → `CommandOutcome` / `CommandError` |
| Геометрия | `Layout::compute_rects(viewport, &DockMetrics) -> LayoutRects` (columns, groups, tab_strips, tab_headers, tab_overflows, panels, splitters, floating_groups (+z_order), floating_resize_edges, flyout, canvas, vertical_overflow) |
| Hit-test | `Layout::hit_test(point, &rects, &metrics) -> HitResult` |
| Drag | `DragSession` (`pointer_down[_with_handle]`, `pointer_move`, `set_drop_target`, `preview`, `pointer_up`, `escape`, `focus_lost`, `cancel`, `reset`), `DragPayload`, `DragHandle`, `DragDropRequest` |
| Drop | `Layout::prepare_drop_resolver(DropSource, &rects, &metrics) -> DropZoneResolver` (`zones()`, `resolve(point)`), `compute_drop_zones`, `DropTarget`, `DropZone`, `DropZoneRect`, `Layout::resolve_drag_drop(&request, &metrics) -> DragDropAction` |
| История | `UndoRedoHistory` (`apply`, `apply_sequence`, `undo`, `redo`, `can_undo/redo`) |
| Сохранение | `WorkspaceFile` (`from_layout`, `to_json_pretty`, `load_json`, `into_layout_with_options`), `WorkspaceLoadOptions { visible_client_region }`, `WorkspaceDiagnostic` |
| Метрики | `DockMetrics` (колонка 280/мин. 200, рейка 36, сплиттер 4/hit 8, порог drag 4, высота заголовка/таб-стрипа 28, мин. вкладка 72, `max_columns` 4, мин. канвас 320) |
| Фабрика | `create_initial_layout(profile, &[(PanelKey, PanelConstraints)], metrics)` — требует дескрипторы `layers`, `color`, `properties`, `history` |
**Заметки по ядру, влияющие на ТЗ:**
1. В текущем репозитории `crates/dock-core/Cargo.toml` уже существует. Старое замечание об отсутствующем манифесте относилось к переданному архиву и больше не является задачей. Этап 0 должен проверить актуальный манифест и feature `serde`, а не создавать второй Cargo-проект.
2. `Layout` — `!Sync` (внутренний `Cell` для кэша валидации). Для GPUI это нормально при владении моделью в главном UI-потоке, но один и тот же `Layout` нельзя шарить через `Arc` между потоками.
3. `DockMetrics::default()` помечены в ядре как предварительные (в т.ч. `header_height`/`tab_strip_height`=28 — placeholder). Адаптер обязан брать значения только из `DockMetrics`, не дублировать константы.
4. Все координаты в ядре — **логические пиксели**, `f32`, `Rect = origin + size`. Конверсия в `Pixels` GPUI — только на границе адаптера.
5. Overflow вкладок (`TabOverflowRect`) — реализационное решение ядра, не поведение Photoshop; меню скрытых вкладок рисует адаптер и шлёт `ActivateTab`.
6. На момент версии ТЗ в корневом `Cargo.toml` закреплена ревизия GPUI `9cd14daad768fbfaeeac8789f515f9a5f67669a6` репозитория `zed-industries/zed`; workspace использует Rust `1.99.1` и edition `2024` (это также задано в `rust-toolchain.toml`). `dock-gpui` использует только workspace-зависимость `gpui`, не объявляет собственную плавающую git-зависимость и не обновляет ревизию GPUI самостоятельно.
7. Если корневой `Cargo.toml` или `rust-toolchain.toml` меняют GPUI revision/toolchain, это отдельное согласованное изменение workspace. В рамках реализации ТЗ нельзя молча обновлять GPUI ради исправления локальной ошибки API.
8. `dock-gpui` не вызывает платформенные Win32/Cocoa/X11/Wayland API напрямую. Платформенный backend и создание OS-окна принадлежат GPUI и приложению-хосту.
---
## 3. Структура папки
```
dock-gpui/
├── Cargo.toml
├── TZ.md                      # этот документ
├── README.md
├── src/
│   ├── lib.rs                 # публичный API, re-export нужных типов dock-core
│   ├── dock_host.rs           # DockHost — корневой компонент (Render)
│   ├── controller.rs          # DockController: Layout + History + Metrics + кэш LayoutRects
│   ├── geometry.rs            # конверсии Rect/Point/Size <-> gpui, viewport
│   ├── registry.rs            # PanelRegistry, PanelDescriptor, PanelView
│   ├── theme.rs               # DockTheme (цвета, шрифты, радиусы, иконки)
│   ├── actions.rs             # GPUI actions + keymap по умолчанию
│   ├── input/
│   │   ├── mod.rs
│   │   ├── pointer.rs         # события мыши → HitResult → намерения
│   │   ├── drag.rs            # DragController поверх DragSession + DropZoneResolver
│   │   ├── resize.rs          # splitter / column / floating / flyout resize
│   │   ├── scroll.rs          # прокрутка VerticalOverflow без изменения дерева
│   │   └── keyboard.rs        # Tab / Shift+Tab, Esc, Undo/Redo
│   ├── components/
│   │   ├── mod.rs
│   │   ├── main_dock.rs       # MainDockView (left/right)
│   │   ├── column.rs          # ColumnView (expanded) + вертикальный скролл
│   │   ├── rail.rs            # RailView (IconsOnly / IconsAndLabels)
│   │   ├── group.rs           # GroupView: заголовок, grip, кнопки режима/меню
│   │   ├── tab_strip.rs       # TabStripView + TabHeaderView
│   │   ├── tab_overflow.rs    # меню скрытых вкладок
│   │   ├── panel_host.rs      # PanelHost: содержимое активной панели
│   │   ├── splitter.rs        # SplitterView (горизонт./вертикальный)
│   │   ├── flyout.rs          # FlyoutView поверх канваса
│   │   ├── floating.rs        # FloatingWindowView: title, close, resize-ручки
│   │   ├── drop_overlay.rs    # DropOverlay: подсветка зон и линия вставки
│   │   ├── drag_ghost.rs      # DragGhost: «призрак» вкладки/группы у курсора
│   │   ├── context_menu.rs    # меню группы / вкладки / колонки
│   │   └── canvas_slot.rs     # слот под пользовательский Canvas
│   ├── persistence.rs         # WorkspaceFile ⇄ диск, диагностики
│   └── assets/                # SVG-иконки (grip, collapse, menu, close, режимы колонок)
├── examples/
│   ├── photoshop_demo.rs      # полноценное демо: 4 базовые панели + канвас-заглушка
│   └── generic_demo.rs        # Profile::Generic
└── tests/
    ├── geometry_roundtrip.rs
    ├── input_to_command.rs
    ├── drag_resize_history.rs
    ├── workspace_recovery.rs
    └── snapshots/             # визуальные эталоны (если принято)
```
---
## 4. Архитектура
### 4.1. Поток данных (однонаправленный)
```text
GPUI event
    │
    ▼
input::* ──► координаты в логических пикселях ──► Layout::hit_test
    │                                               │
    ├── обычное действие ──────────────────────────┤
    │                                               ▼
    └── жест ──► DragSession / ResizeInteraction ─► DockCommand
                                                    │
                                      UndoRedoHistory::apply
                                                    │
                                      Layout::apply (dock-core)
                                                    │
                                  Layout.version / состояние
                                                    ▼
                       cache key изменился? ── да ──► compute_rects
                                                    │
                                                    ▼
                                    LayoutRects (производный кэш)
                                                    │
                                                    ▼
                                     cx.notify() → Render
                                                    │
                                                    ▼
                                stateless GPUI components
```
Правила потока:
- `Layout` — единственная изменяемая структурная модель. `LayoutRects` — только производный снимок для рендера и hit-test, а не второй layout.
- Компоненты не принимают решений о допустимости Dock/Drop и не рассчитывают итоговые размеры. Они отображают переданные данные и отправляют намерения контроллеру.
- События GPUI переводятся в типы ядра на границе адаптера. Внутри Core не передаются `gpui::Pixels`, `Window`, `AnyView` или другие UI-типы.
- После успешной команды контроллер обновляет кэш, отправляет событие и вызывает `cx.notify()` через владельца Entity. После ошибки текущий валидный снимок не заменяется частично рассчитанным.
### 4.2. DockController (модель)
`DockController` — GPUI-сущность (`Entity<DockController>` либо эквивалент, который требует закреплённая ревизия GPUI). Она владеет:
- `layout: Layout` — единственная структура Dock;
- `history: UndoRedoHistory`;
- `metrics: DockMetrics`;
- `viewport: Rect` в логических пикселях;
- `rects: Option<LayoutRects>` и `rects_key: Option<RectCacheKey>`;
- `drag: DragController` (внутри `DragSession` и `Option<DropZoneResolver>`);
- `interaction: Option<ResizeInteraction>`;
- `registry: PanelRegistry` и кэшем созданных `AnyView` по `PanelId` (кэш можно вынести в отдельный небольшой модуль, но он не дублирует дерево);
- `last_error: Option<CommandError>` для диагностики.
**Ключ кэша геометрии.** `RectCacheKey` должен учитывать как минимум `layout.version()`, `viewport`, ревизию метрик, а также состояние `chrome` и `flyout`, если эти параметры не гарантированно отражены в `Layout.version()` закреплённой версией Core. `DockMetrics` по умолчанию неизменяемы в течение жизни контроллера; при их замене увеличивается `metrics_revision` и кэш инвалидируется. Повторный `compute_rects` при неизменном ключе запрещён.
`Layout::update_canvas_bounds(rects.canvas)` (в Core это `Option<Rect>`) вызывается только после получения нового валидного снимка и только при изменении границ Canvas. Метод возвращает `Result<bool, GeometryError>` и предназначен для записи host-measured bounds без создания persistent command revision; он не должен добавлять шаг Undo или вызывать повторный полный layout pass сам по себе. Тест должен подтвердить, что повторная передача тех же bounds возвращает `false`, а изменение bounds — `true` без изменения `Layout.version()`.
Публичный API контроллера:
- `dispatch(DockCommand) -> Result<CommandOutcome, CommandError>` — через `history.apply`;
- `dispatch_transient(DockCommand)` — отдельная семантическая точка API для команд flyout/анимационного состояния, но использует тот же `UndoRedoHistory::apply`. Текущий Core не создаёт checkpoint для transient-команд; адаптер не ведёт собственную историю и не обходит фасад без необходимости;
- `dispatch_sequence(Vec<DockCommand>)` — атомарный шаг через `apply_sequence`;
- `undo()`, `redo()`, `can_undo()`, `can_redo()`;
- `open_panel(key)`, `close_panel(id)`, `toggle_panel(key)`;
- `set_viewport(Rect)` и `set_metrics(DockMetrics)`;
- `save_workspace() -> Result<String, ...>`, `load_workspace(&str) -> Result<..., ...>`;
- события наружу: `DockEvent::{LayoutChanged, PanelOpened, PanelClosed, PanelActivated, CommandFailed, WorkspaceLoaded(diagnostics)}`.
**Правила команд и ошибок.**
- Неуспешная команда не должна менять `Layout`, историю или кэш.
- После `undo`, `redo`, `load_workspace` и `reset_workspace` кэш геометрии пересчитывается; при загрузке другой `Layout` история старой сессии очищается/синхронизируется средствами `UndoRedoHistory`.
- `NoOp` и transient-команды не должны создавать запись Undo.
- Команду из UI нельзя выполнять одновременно в компоненте и в контроллере. Один пользовательский жест приводит к одной финальной диспетчеризации (или к одному `dispatch_sequence`).
### 4.3. Идентификаторы
- Внутри сессии адаптер оперирует `NodeId/PanelId/FloatingGroupId` из ядра.
- **В сохранённых файлах и публичном API приложения** панели идентифицируются только `PanelKey` — рантайм-ID не сериализуются (это гарантирует и ядро).
- Состояние содержимого панелей (скролл, выделение и т.п.) принадлежит самим панелям, ядро хранит только `PanelState.last_known_size`.
### 4.4. Реестр панелей
```rust
pub struct PanelDescriptor {
    pub key: PanelKey,
    pub title: SharedString,
    pub icon: IconSource,
    pub constraints: PanelConstraints,
    pub factory: Box<dyn Fn(&mut Window, &mut App) -> AnyView>,
}
pub struct PanelRegistry { /* HashMap<PanelKey, PanelDescriptor> */ }
```
**Контракт панели в v1:**
- `PanelView` — не отдельный обязательный trait с собственным циклом `render/on_resize/on_focus`, а GPUI `AnyView`, возвращаемый `factory`. Конкретная панель реализует обычный GPUI view/`Render` и сама владеет своим прикладным состоянием.
- Размер и фокус доставляются панели обычными средствами GPUI. Отдельные lifecycle callbacks `on_resize`/`on_focus` в обязательный контракт v1 не вводятся: они дублировали бы жизненный цикл GPUI view. Если реальный сценарий потребует дополнительных уведомлений, их можно добавить как необязательный расширяемый hook после прототипа.
- `PanelRegistry::descriptors_for_core()` возвращает `Vec<(PanelKey, PanelConstraints)>` для `create_initial_layout` и `WorkspaceFile::load_json`.
- `PanelKey` уникален в реестре. Повторная регистрация того же ключа возвращает `RegistrationError`; молчаливое перезаписывание существующего дескриптора запрещено.
- Вьюхи создаются лениво при первом показе и кэшируются по runtime `PanelId`. Переключение вкладок, сворачивание группы, перемещение панели и изменение размера не должны без причины пересоздавать view.
- View удаляется из кэша после фактического закрытия `PanelId` (`ClosePanel`, закрытие группы или floating-группы). После undo закрытия новый runtime `PanelId` может отличаться — создаётся новый view.
- `singleton` означает «не более одного открытого экземпляра данного `PanelKey`», но **не** означает запрет закрытия. В v1 панели закрываемы; запрет закрытия нельзя выводить из `singleton`. Если в будущем появится обязательная/незакрываемая панель, это должно быть выражено явным ограничением Core, а не только скрытой кнопкой в UI.
- Неизвестный ключ при загрузке (`UnknownPanelKeyPreserved`) отображается через `MissingPanelPlaceholder` с названием ключа. Layout при этом не переписывается и неизвестная панель не удаляется автоматически.
- Если для активного `PanelId` view ещё нет, вызывается фабрика. Отсутствующий descriptor не должен приводить к panic: показывается `MissingPanelPlaceholder`. Паника внутри самой пользовательской factory не является recoverable-ошибкой адаптера и остаётся ответственностью приложения; в v1 factory имеет синхронную сигнатуру, возвращающую `AnyView`.
- Состояние содержимого (выделение, прокрутка, раскрытые секции) принадлежит панели. Оно не записывается в `dock-core` и не попадает в WorkspaceFile автоматически.
---
## 5. Компоненты
Для каждого компонента указано: источник данных в `LayoutRects`, поведение, события → команды.
### 5.1. `DockHost` (корень)
- Занимает всё окно приложения; раскладывает: левый `MainDockView`, `CanvasSlot`, правый `MainDockView`, `FlyoutView`, слой `FloatingWindowView[]`, слой `DropOverlay`, `DragGhost`, контекст-меню.
- Измеряет себя (через `canvas()`/bounds-callback GPUI) → `controller.set_viewport`.
- Учитывает `Layout::chrome()`: при `DockHidden`/`AllHidden` док не рисуется (ядро уже исключает его геометрию); `toolbar_visible()`/`options_bar_visible()` — отдаются приложению через слоты `toolbar` и `options_bar` в билдере.
- После каждого нового валидного расчёта вызывает `layout.update_canvas_bounds(rects.canvas)` только при изменении прямоугольника; защита от цикла пересчётов описана в §4.2.
- Верхнее меню приложения не является частью DockHost. `toolbar` и `options_bar` — внешние слоты, а `canvas` — пользовательская вьюха приложения. При отсутствии canvas-slot отображается нейтральная пустая поверхность.
- Z-порядок слоёв (снизу вверх): canvas → main docks → flyout → floating (по `z_order`) → drop overlay → drag ghost → контекст-меню/tooltip.
### 5.2. `MainDockView` / `ColumnView`
- Данные: `rects.columns`, `rects.groups`, `rects.splitters`, `rects.vertical_overflow`.
- Режим `ColumnMode::Expanded`: стек `GroupView` с вертикальными сплиттерами между группами; между колонками — горизонтальные сплиттеры.
- Если для колонки есть `VerticalOverflow`: `viewport` используется как область клипа, `content_extent` — как полная высота содержимого. Группы **не сжимаются** ниже минимума. Положение прокрутки хранится как временное UI-состояние адаптера, привязанное к runtime `NodeId`, а не как вторая копия Dock-дерева.
- При hit-test координата указателя преобразуется обратно в координатную систему `LayoutRects` с учётом scroll offset; сами прямоугольники Core не пересчитываются и не изменяются. При удалении/замене узла scroll state очищается.
- Над колонкой — тонкая полоса управления режимом колонки (кнопка «⟨⟨/⟩⟩» → `SetColumnMode`): Expanded ⇄ IconsAndLabels ⇄ IconsOnly.
- Рендер строго по прямоугольникам из `LayoutRects` (абсолютное позиционирование внутри контейнера дока).
### 5.3. `GroupView`
- Заголовок группы (`header_height`): **grip** (область перетаскивания всей группы → `DragPayload::Group`), `TabStripView`, кнопки управления (`group_control_button_width`): меню группы, кнопка свернуть/развернуть.
- Двойной клик по пустому месту таб-стрипа или по вкладке → `CollapseGroup { collapsed: !current }` (распознавание двойного клика — в адаптере, как требует ядро).
- Свёрнутая группа — высота `collapsed_group_height`, контент не рисуется.
- Меню группы (`ContextMenu`): «Закрыть группу» (`CloseGroup`), «Закрыть вкладку» (`ClosePanel`), «Свернуть/Развернуть», «Вынести в плавающее окно» (`Float`).
### 5.4. `TabStripView` / `TabHeaderView` / `TabOverflow`
- Данные: `rects.tab_strips`, `rects.tab_headers` (с `index`), `rects.tab_overflows`.
- Вкладка: иконка + заголовок (усечение с `…`), активная подсвечена (`Tabs.active_index`), кнопка закрытия при hover. `singleton` ограничивает число экземпляров, но само по себе не запрещает закрытие.
- Клик → `ActivateTab`. Средний клик → `ClosePanel` (опция темы/настроек).
- Press на вкладке → `DragSession::pointer_down(DragPayload::Panel)`.
- `TabOverflowRect.hidden_tabs` → кнопка «»» с выпадающим списком; выбор → `ActivateTab`.
### 5.5. `PanelHost`
- Данные: `rects.panels` (только активная панель каждой группы).
- Рендерит `AnyView` панели из реестра в заданный прямоугольник, клипует содержимое.
- Неактивные вкладки не должны одновременно занимать видимую область и получать pointer input. При переключении активной вкладки показывается уже созданный view из кэша либо лениво создаётся новый.
- Фактический размер панели может передаваться для обновления `PanelState.last_known_size` только через предусмотренную команду/метод Core. Адаптер не мутирует внутренние поля Core напрямую.
- Фокус/клавиатура внутри панели остаются под контролем этой панели. Глобальные шорткаты Dock не должны перехватывать ввод текстовых полей и других интерактивных дочерних элементов.
- Отдельный `PanelView` trait и ручной `render/on_resize/on_focus` lifecycle в v1 не вводятся; см. §4.4.
### 5.6. `SplitterView`
- Данные: `rects.splitters` (`owner`, `boundary_index`, `axis`, `rect`). Визуально — линия `splitter_thickness`, hit-зона `splitter_hit_zone`.
- Курсор: `col-resize` / `row-resize` при hover.
- Drag → команды (с троттлингом по кадру):
  - вертикальный сплиттер между группами → `ResizeGroups { column, boundary_index, delta, available_height }`
  - между колонками main-дока → `ResizeColumnBoundary { side, boundary_index, delta }`
  - внешний край дока (у канваса) → `ResizeColumn { column, width }`
  - сплиттер внутри floating-дерева → `ResizeSplitColumnBoundary`
- **Дельта считается от точки нажатия, а не накопительно**, чтобы упор в min-размер не «уплывал». Фактический результат берём из `CommandOutcome.actual_value`.
- Во время интерактивного ресайза один жест = одна запись в историю (см. 7.4).
- Двойной клик по сплиттеру → `ResetColumnRatios` / `ResetMainColumnRatios`.
### 5.7. `RailView` (рейки иконок)
- Режимы `IconsOnly` (ширина `rail_width`) и `IconsAndLabels` (ширина `min_column_width`).
- Порядок элементов — `Layout::column_display_order(column)` (плоский: группы сверху вниз, вкладки слева направо). Дерево не меняется.
- Клик по иконке → `OpenFlyout { column, panel }`; повторный клик по выбранной иконке при `flyout_auto_collapse_enabled` → `CloseFlyout`.
- Drag иконки → `DragPayload::Panel`, цели `RailSlot`/`ColumnSlot`/`FreeArea` и т.д.; линия вставки по `RailInsertion`.
- Разделители между исходными группами — тонкая линия (чисто визуально).
### 5.8. `FlyoutView`
- Данные: `rects.flyout` (`rect`, `resize_edge`, `column`, `group`, `panel`). Рисуется **поверх** канваса без reflow.
- Ресайз за край, обращённый к канвасу → `ResizeFlyout { panel, width }` (transient, без истории).
- Закрытие: клик вне, Esc → `DismissFlyout`; явная кнопка → `CloseFlyout`.
- Drag панели из flyout: `pointer_down_with_handle(Panel, DragHandle::FlyoutPanel)`; flyout остаётся видимым во время жеста; валидный drop → `MovePanelFromFlyout`, для FreeArea → `FloatPanelFromFlyout { bounds }`; отмена оставляет flyout открытым.
- Тень/анимация появления (150–200 мс, отключается через тему/reduce-motion).
### 5.9. `FloatingWindowView`
- Данные: `rects.floating_groups` (с `z_order`), `rects.floating_resize_edges`, `HitResult::{FloatingTitle, FloatingGroup, FloatingResizeEdge}`.
- Внутри окна — то же дерево (`Split/Tabs/Column`), рендерится теми же `GroupView/TabStripView/SplitterView`.
- Title bar: заголовок (по активной вкладке), кнопка закрытия → `CloseFloatingGroup`, drag за title → `MoveFloating` (через `DragHandle::FloatingTitle`; dock-цель **не** принимается). Опциональная «capture»-область для докинга окна целиком (`DragHandle::FloatingCapture` → `DockFloating`).
- 8 ручек ресайза (`ResizeEdge`) → `ResizeFloatingBounds` (атомарно для верхних/левых краёв).
- Любой pointer-down внутри окна → `RaiseFloating`.
- `FloatingWindowView` — визуальная композиция внутри того же главного GPUI окна, а не отдельное OS-окно. Окно клампится ядром в снимке геометрии; адаптер не двигает его за пределы вручную и не использует Win32/Cocoa/X11 API.
### 5.10. `DropOverlay` и `DragGhost`
- `DropOverlay`: источник — `DropZoneResolver::zones()`, подсвечивается зона, соответствующая текущему `DragPreview.target`. Визуалы по `DropZone`:
  - `Center`/`TabInsertion` → рамка группы + вертикальная линия вставки в таб-стрипе;
  - `Left/Right/Top/Bottom` (`GroupEdge`) → полупрозрачная половина группы;
  - `ColumnLeft/Right/End`, `DockEdge`, `MainColumn` → вертикальная/горизонтальная направляющая;
  - `RailInsertion` → горизонтальная линия в рейке;
  - `Floating*` → подсветка плавающего окна;
  - `FreeArea` → контур будущего floating-окна (адаптер задаёт желаемые bounds как часть запроса, а Core проверяет и нормализует их; см. §6.3).
- `DragGhost`: компактная «карточка» вкладки (иконка + название) или миниатюра заголовка группы, следует за курсором со смещением, `pointer-events: none`.
- Во время drag курсор — `grabbing`; запрещённая зона — `not-allowed`.
### 5.11. `ContextMenu`
- Универсальный компонент меню (пункты, разделители, disabled, подменю 1 уровня, hotkey-подписи); закрывается по Esc/клику вне. Используется в меню группы, вкладки, колонки и overflow вкладок.
### 5.12. `CanvasSlot`
- Контейнер для пользовательской вьюхи, получает `rects.canvas`. Адаптер ничего не рисует внутри, кроме собственного оверлея drop-зоны `FreeArea`.
---
## 6. Ввод и жесты
### 6.1. Единый pipeline pointer-событий
1. `mouse_down/move/up`, `mouse_leave`, `scroll` и отмена указателя обрабатываются на корне `DockHost` либо специализированным input-компонентом.
2. При начале drag/resize адаптер запрашивает поддерживаемый закреплённой ревизией GPUI захват/удержание pointer events. События должны продолжать поступать, даже когда курсор выходит за границы исходного элемента; если GPUI не поддерживает capture на данной платформе, поведение деградирует в безопасную отмену, а не в зависший жест.
3. Координаты GPUI переводятся в логические пиксели `dock-core`. Конверсия выполняется один раз на границе; значения `NaN`/`Infinity`/отрицательные размеры отвергаются до вызова Core.
4. Для узлов внутри `VerticalOverflow` к координате hit-test добавляется соответствующий scroll offset. Обратное преобразование относится только к input mapping и не меняет `LayoutRects`.
5. `controller.hit(point)` вызывает `Layout::hit_test(point, &rects, &metrics)`; результат не кэшируется дольше одного события указателя.
6. При потере фокуса окна, потере capture, смене viewport или внешнем изменении `layout.version` текущий drag/resize отменяется с корректным сбросом `DragSession`/interaction state.
7. По `HitResult` выбирается обработчик из таблицы §6.2. Компоненты не определяют hit зоны самостоятельно.
### 6.2. Таблица «HitResult → намерение»
| HitResult | Down | Click | Double-click | Drag |
|---|---|---|---|---|
| `TabHeader` | запомнить кандидата на drag | `ActivateTab` | `CollapseGroup` | `Panel` + `PanelTab` |
| `TabStrip` (пустое место) | кандидат на drag группы (если grip-режим) | — | `CollapseGroup` | `Group` + `GroupGrip` |
| `TabOverflowButton` | — | открыть меню | — | — |
| `Group` (grip/заголовок) | кандидат | — | — | `Group` + `GroupGrip` |
| `Splitter` | старт resize | — | `Reset*Ratios` | resize команды (5.6) |
| `FlyoutPanel` | кандидат | — | — | `Panel` + `FlyoutPanel` |
| `FlyoutResizeEdge` | старт resize | — | — | `ResizeFlyout` |
| `FloatingTitle` | `RaiseFloating` + кандидат | — | — | `FloatingGroup` + `FloatingTitle` |
| `FloatingGroup`/`Panel` | `RaiseFloating` | — | — | — |
| `FloatingResizeEdge` | старт resize | — | — | `ResizeFloatingBounds` |
| `Canvas` | при открытом flyout: `DismissFlyout` | — | — | — |
| `Miss` | игнор | — | — | — |
### 6.3. Drag & Drop (поверх `DragSession`)
1. `pointer_down*` → `DragSession::pointer_down_with_handle(payload, handle, pos)`.
2. До `drag_threshold` состояние — `Pressed`; после порога — `Dragging`. `DropZoneResolver` создаётся один раз при переходе в `Dragging` через `layout.prepare_drop_resolver(DropSource, &rects, &metrics)`. Для `FloatingTitle` resolver не создаётся.
3. Каждое движение → `resolver.resolve(point)` → `session.set_drop_target(target)` → обновление `DropOverlay` и `DragGhost`. Рендер preview не меняет основную структуру `Layout`.
4. `pointer_up(pos, resolved_target)` → `Option<DragDropRequest>`. При отсутствии запроса или при `DragDropAction::NoOp` состояние Dock не изменяется.
5. Для `target == FreeArea` адаптер формирует **желаемые** `floating_bounds`: предпочтительный размер берётся из `PanelConstraints.preferred_size` либо текущего размера группы; позиция сохраняет разумное смещение курсора относительно перетаскиваемого элемента. Эти bounds — входные данные команды, а не окончательное решение адаптера. Core проверяет ограничения и выполняет нормализацию/кламп относительно viewport.
6. `layout.resolve_drag_drop(&request, &metrics)` возвращает `NoOp` либо `Command(cmd)`. Полученная команда отправляется в `controller.dispatch(cmd)` ровно один раз.
7. Для `FloatingTitle` движение визуально предварительно отображается через ghost/контур. По `pointer_up` отправляется один `MoveFloating`. Док-цели для `FloatingTitle` не принимаются; целиком вернуть группу в Dock можно только через явно обозначенную capture-зону (`FloatingCapture` → `DockFloating`), если эта возможность включена.
8. Esc → `session.escape()`. Потеря фокуса/capture → `focus_lost()`/`cancel()`. После любого завершения вызывается `session.reset()`, resolver освобождается, pointer capture снимается.
9. Если за время жеста изменился `layout.version`, viewport или исходный runtime ID перестал существовать, жест отменяется (`DragCancelReason::Explicit` или подходящая причина Core). Resolver не используется со старым снимком геометрии.
10. Drag из flyout не закрывает flyout при старте. После успешного переноса Core меняет состояние; при отмене flyout остаётся открытым.
### 6.3.1. Правила разрешения drop
- Единственный список допустимых зон берётся из `DropZoneResolver::zones()`. UI не создаёт дополнительные зоны и не подсвечивает цель, которой нет в resolver.
- `DropZone` определяет вид превью, а `DropTarget` — семантическую цель. Нельзя выводить тип команды из одной только визуальной позиции.
- Запрещённая цель отображает `not-allowed` и не приводит к команде. `NoOp` не считается ошибкой и не создаёт историю.
- Если `resolve_drag_drop` отклоняет запрос, ошибка передаётся контроллеру/логу, исходный layout остаётся валидным.
### 6.3.2. Ресайз и интерактивные жесты
- На `mouse_down` сохраняются исходная точка, исходные размеры/параметры и идентификатор целевого splitter/column/floating/flyout.
- На `pointer_move` адаптер рассчитывает дельту относительно **исходной точки нажатия**, а не относительно предыдущего кадра. Это исключает накопление ошибки при упоре в min/max.
- В v1 во время drag показывается preview (линия/контур будущего размера); `Layout` не мутируется на каждом движении. На `mouse_up` отправляется одна итоговая resize-команда с полной дельтой. Это выбранное решение для одного Undo-шагa на жест и не требует временной второй копии Dock-дерева.
- `CommandOutcome.actual_value` используется для обновления UI/диагностики после применения. Итоговые размеры и min/max остаются решением Core.
- Для resize нескольких параметров, который логически является одной операцией, используется `dispatch_sequence`/`UndoRedoHistory::apply_sequence`, чтобы сохранить атомарность и один checkpoint.
- Если позже потребуется live-resize с изменением Layout на каждом кадре, сначала расширяется API `dock-core` явной транзакцией `begin/preview/commit/cancel`. Временная копия Layout в адаптере не вводится.
### 6.3.3. Прокрутка `VerticalOverflow`
- `VerticalOverflow.viewport` — фиксированная видимая область, `content_extent` — высота полного содержимого. GPUI scroll handle отвечает только за смещение отображения содержимого.
- Scroll offset хранится отдельно от Dock-дерева и привязывается к runtime `NodeId`; он не сериализуется в WorkspaceFile v1.
- При прокрутке не посылаются `ResizeGroups` и не меняются group ratios. На input offset преобразует позицию указателя обратно в систему координат прямоугольников Core.
- При закрытии/пересоздании владельца overflow соответствующий scroll state удаляется. Если Core в будущем получит собственную persistent scroll model, адаптер должен перейти на неё, а не дублировать значения.
### 6.4. Клавиатура и actions
Actions GPUI (`actions!(dock, [...])`) и keymap по умолчанию (переопределяется приложением):
| Action | Клавиши | Реализация |
|---|---|---|
| `ToggleChromeAll` | `Tab` (только если фокус не в текстовом поле/интерактивной панели и действие не перехвачено приложением) | `SetChrome { mode: chrome.after_tab_shortcut() }` |
| `ToggleChromeDock` | `Shift+Tab` (если приложение/поле ввода не использует сочетание) | `SetChrome { mode: chrome.after_shift_tab_shortcut() }` |
| `Undo` / `Redo` | `Cmd/Ctrl+Z`, `Cmd/Ctrl+Shift+Z` (и `Ctrl+Y` на Win/Linux) | `controller.undo()/redo()` |
| `CancelInteraction` | `Esc` | сначала отменить активный drag/resize; иначе закрыть flyout; иначе закрыть context menu |
| `ResetWorkspace` | настраивается | запросить подтверждение на уровне приложения (если есть несохранённые изменения), затем восстановить раскладку текущего профиля |
---
## 7. Состояние, история, сохранение
### 7.1. Профили
- `Profile::Photoshop` — основной (колонки, группы, сворачивание, рейки, flyout). `Profile::Generic` — поддерживается рендером без Photoshop-специфики; демо `generic_demo`.
- Профиль задаётся при создании и хранится в `WorkspaceFile`.
### 7.2. Начальная раскладка
- Для `photoshop_demo`: `create_initial_layout(Profile::Photoshop, descriptors, metrics)`; обязательные панели-дескрипторы: `layers`, `color`, `properties`, `history`. Все четыре descriptor должны быть зарегистрированы до создания layout.
- Для обычного приложения допустим пустой `Layout::new(Profile::Photoshop)`; автоматического открытия демонстрационных панелей нет.
- `Profile::Generic` может начинаться с пустого Layout либо с явно заданной демонстрационной композиции.
- Остальные зарегистрированные панели открываются через `OpenPanel` (с `target: None` — по `last_dock_location`/профильному умолчанию ядра). Если профиль/раскладка не может определить целевую область, команда должна вернуть контролируемую ошибку/NoOp; адаптер не создаёт самодельную колонку.
### 7.3. Сохранение / загрузка рабочей области
- `dock-gpui` получает `workspace_path` от приложения-хоста. Определение OS config directory и имени приложения относится к приложению, а не к переиспользуемой библиотеке.
- Сохранение: `WorkspaceFile::from_layout(&layout, &metrics)?.to_json_pretty()`. Запись — во временный файл в том же каталоге, `flush`/`sync_all`, затем атомарная замена основного файла средствами, совместимыми с целевой ОС. Нельзя сначала удалять единственную исправную копию основного файла.
- Автосохранение запускается с debounce 1–2 секунды после **persistent** изменения layout. Повторные изменения сбрасывают debounce. Transient flyout/hover/drag-preview не вызывают сохранение.
- При закрытии окна приложение по возможности выполняет финальный flush ожидающего сохранения. Ошибка сохранения логируется и не блокирует закрытие навсегда.
- Загрузка: `WorkspaceFile::load_json(json, descriptors, metrics, WorkspaceLoadOptions::new(Some(visible_client_region)))`. `visible_client_region` передаётся в логических координатах текущего клиентского viewport.
- `WorkspaceDiagnostic` → `tracing` + необязательное событие/тост («Рабочая область восстановлена с исправлениями»). Diagnostics не превращаются в фатальные ошибки, если Core вернул валидный `Layout`.
- При ошибке синтаксиса/валидации текущий файл сначала сохраняется как `workspace.json.bak`, затем приложение откатывается на базовую раскладку. Исходный файл не затирается пустой раскладкой до успешной загрузки или явного решения пользователя.
- Файл с версией формата новее поддерживаемой не считается обычным повреждённым файлом: его нельзя автоматически перезаписывать или переносить в `.bak` как будто он битый. Показывается ошибка несовместимости и сохраняется возможность восстановления после обновления приложения.
- WorkspaceFile хранит только сериализуемую структуру Dock и стабильные `PanelKey`. Runtime ID, pointer state, hover, открытые меню и scroll offset v1 не сериализуются.
- После успешной загрузки кэш `AnyView` сверяется с новыми runtime `PanelId`; устаревшие views удаляются, недостающие создаются лениво. История старого layout сбрасывается/синхронизируется средствами Core.
- Именованные рабочие области (Essentials, Painting, …) — v1.1+ (отдельное расширение после базового файла `workspace.json`); в v1 не требуется API `save_as(name)`/`load(name)`.
### 7.4. История (Undo/Redo)
- Все пользовательские persistent структурные команды проходят через `UndoRedoHistory::apply`.
- Текущая реализация Core фиксирует checkpoint только после успешного persistent изменения версии Layout; NoOp, ошибка и transient-команды не должны добавлять запись. Это поведение проверяется тестами адаптера.
- **Интерактивные жесты (resize и перемещение floating-группы) — один шаг истории на жест.** В v1 Layout не меняется на каждом pointer_move: во время drag показывается preview, на mouse_up применяется одна итоговая команда. Для составной операции используется `apply_sequence` как одна атомарная запись.
- Если жест отменён (`Esc`, потеря фокуса/capture, исчезновение source ID), команда не отправляется и история не меняется.
- `OpenFlyout`, `CloseFlyout`, `DismissFlyout`, `ResizeFlyout`, `SetFlyoutAutoCollapse` относятся к transient-поведению и не засоряют Undo. Они проходят через историю/фасад контроллера в соответствии с семантикой Core; контроллер не создаёт собственный стек истории.
- `undo/redo` делегируются `UndoRedoHistory`; после успешного восстановления увеличившийся `Layout.version` инвалидирует кэш геометрии и инициирует render.
- Лимит — `DEFAULT_UNDO_LIMIT`, настраивается через конструктор history. Изменение лимита не меняет сериализуемый WorkspaceFile.
---
## 8. Визуальный стиль и тема
`DockTheme` (структура, задаётся приложением; по умолчанию — тёмная тема «Photoshop-like»):
- Цвета: фон дока, фон заголовка группы, активная/неактивная вкладка, hover, разделитель, акцент drop-зоны (полупрозрачный), текст/иконки (primary/secondary/disabled), тень flyout/floating, цвет сплиттера (normal/hover/active).
- Типографика: размер/вес шрифта вкладок, заголовков окон.
- Радиусы, толщина рамок, отступы; размеры берутся **из `DockMetrics`**, тема только добавляет косметику.
- Иконки: SVG-набор (grip, chevron, close, menu, режимы колонки, 4 базовые панели); загрузка через ассет-систему GPUI.
- Светлая тема и переключение на лету — желательно (v1.1), архитектурно закладывается (тема — глобал GPUI).
- Анимации: появление flyout, collapse/expand группы (около 200 мс), подсветка drop-зон; уважать системный reduce-motion. Если закреплённая версия GPUI не предоставляет надёжный системный флаг, тема содержит явный `reduce_motion` override; отсутствие системного API не должно ломать работу интерфейса.
---
## 9. Нефункциональные требования
- **Производительность.** Целевой сценарий — 60 FPS на reference machine при взаимодействии с ≥ 12 панелями в 3 колонках. До объявления performance DoD фиксируются конфигурация reference machine и release-профиль измерений. `compute_rects` вызывается только при смене cache key (§4.2); `hit_test` — не чаще одного раза на событие; `DropZoneResolver` — один на жест. Никаких аллокаций в горячем пути рендера, кроме неизбежных. Снимки `LayoutRects` и список render-items не пересоздаются без изменения входов.
- **Корректность.** Адаптер не должен паниковать на `CommandError`/`ComputeRectsError`/`DragSessionError`: ошибки логируются (`tracing`), UI остаётся в последнем валидном состоянии. Узкое окно (Canvas < `min_canvas_width`) → `ComputeRectsError` обрабатывается: показывается последний валидный снимок либо режим «окно слишком узкое», без краша и без записи невалидной геометрии в persistence.
- **Адаптивность и DPI.** Работа в логических пикселях; корректная отрисовка на Retina/HiDPI (1 px линии), смена scale factor на лету.
- **Доступность.** Фокус-кольца, навигация по вкладкам клавиатурой (Left/Right в таб-стрипе, Enter/Space — активировать, Del — закрыть), подписи для иконок-кнопок (tooltip).
- **Платформы.** Код `dock-gpui` не содержит OS-specific API. Поддерживаются только те комбинации macOS, Linux (Wayland/X11) и Windows, которые реально поддерживает закреплённая ревизия GPUI и backend приложения. Совместимость каждой целевой комбинации подтверждается отдельной сборкой/ручным smoke-test; нельзя заявлять поддержку ОС только по отсутствию `cfg`-ограничений в адаптере.
- **Безопасность.** `#![forbid(unsafe_code)]` в `dock-gpui`; без сетевых вызовов; чтение/запись только в каталог конфигурации.
- **Зависимости.** `dock-core` (path, feature `serde`), `gpui` из workspace с закреплённой ревизией `9cd14daad768fbfaeeac8789f515f9a5f67669a6` (на дату v1.1), `tracing`, `thiserror`; `serde_json` подключается к `dock-gpui` напрямую только если адаптер реально использует его API, иначе достаточно serde API `dock-core`. Без лишних UI-крейтов и без повторного объявления GPUI в манифесте `dock-gpui`. Если ревизия меняется, это фиксируется в корневом workspace и проверяется на сборке всех примеров.
- **Качество кода.** `cargo fmt --all -- --check`, `cargo clippy -p dock-gpui --all-targets -- -D warnings` (и workspace checks), тесты; целевой размер файла ≤ ~400 строк. Если файл превышает лимит, разделять по ответственности, а не создавать параллельную реализацию. Все публичные типы/методы имеют rustdoc.
---
## 10. Тестирование
1. **Юнит-тесты без открытия OS-окна:**
   - конверсии `Point`/`Rect`/`Size`, проверка конечности и round-trip с учётом scale factor;
   - таблица `HitResult → намерение`;
   - `DragController`: `Pressed → Dragging → Drop/Cancel`, порог, Esc, focus_lost, capture lost, FreeArea bounds, flyout source;
   - расчёт дельт resize от исходной точки нажатия, min/max и отсутствие накопительного drift;
   - cache key: одинаковые входы не вызывают повторный `compute_rects`, изменение viewport/metrics/version инвалидирует снимок;
   - PanelRegistry: уникальность ключей, descriptor lookup, неизвестная панель, жизненный цикл кэша по `PanelId`;
   - transient-команды не создают Undo checkpoint; один resize/drag жест создаёт один checkpoint.
2. **Интеграционные тесты с GPUI:** по возможности использовать headless-фикстуры `TestAppContext` поддерживаемой ревизии GPUI. Проверить: клик по вкладке → `ActivateTab`; двойной клик → `CollapseGroup`; drag между группами/колонками; floating и dock-back; flyout open/dismiss/resize/drag; отмена жеста; Undo/Redo после каждого сценария; save → load → эквивалентные layout-состояние и `LayoutRects`.
3. **Если pinned GPUI не позволяет запускать часть UI-тестов headless**, такие сценарии не маскируются фиктивными тестами: логика Core/controller тестируется без окна, а UI-события покрываются минимальным ручным smoke checklist. Ограничение тестового harness фиксируется в README.
4. **Property/fuzz-тесты:** детерминированная генерация случайных последовательностей команд/жестов с фиксируемым seed. После каждой успешной операции — `layout.validate(&metrics)`; после ошибки исходный layout остаётся валидным и не меняется. Цель DoD — не менее 10 000 шагов на CI seed set и возможность локально повторить конкретный seed.
5. **Тесты persistence:** атомарная запись; чтение прежней версии WorkspaceFile; повреждённый JSON → `.bak` + fallback; неизвестный PanelKey сохраняется; неподдерживаемая будущая версия не перезаписывается; загрузка не сохраняет runtime IDs.
6. **Визуальные тесты:** опциональные snapshot-эталоны для expanded, rail, flyout, drag/drop overlay и floating. Не сравнивать сырые пиксели между разными OS/scale factor без нормализации.
7. **Ручной чек-лист** паритета с Photoshop — приложение A. Любой ручной smoke-test сопровождается указанием OS, backend, scale factor и ревизии GPUI.
---
## 11. Этапы и приёмка
| Этап | Содержание | Критерий приёмки |
|---|---|---|
| **0. Подготовка** | Проверить существующий `crates/dock-core/Cargo.toml` и feature `serde`; `cargo test -p dock-core --features serde`; подтвердить Rust `1.99.1`, edition `2024` и GPUI revision `9cd14daad768fbfaeeac8789f515f9a5f67669a6`; минимальный GPUI window smoke-test; CI fmt/clippy/test | Core tests зелёные; зависимости берутся из workspace; закреплённая GPUI ревизия собирается; пустое окно запускается. Не создавать второй crate и не обновлять GPUI без необходимости |
| **1. Каркас** | `DockController`, `geometry`, `PanelRegistry`, `DockHost`, `CanvasSlot`; пустой layout для приложения; статический рендер колонок/групп/вкладок из `LayoutRects`; отдельный `photoshop_demo` с четырьмя панелями | Обычное приложение показывает пустое рабочее пространство с внешним toolbar/menu slot и Canvas slot; Photoshop demo показывает 4 заглушки; resize главного окна обновляет геометрию; нет второго дерева и локального расчёта dock-layout |
| **2. Вкладки и группы** | Tab strip, активация, закрытие, overflow-меню, collapse (двойной клик), context menu группы; кэш view по PanelId | Все команды вкладок работают; singleton не путается с запретом закрытия; view не пересоздаётся при простом переключении; `Undo/Redo` откатывает |
| **3. Ресайз и overflow** | Сплиттеры групп/колонок, внешний край, vertical overflow со скроллом, reset по двойному клику; preview и финальная команда на mouse_up | Мин-размеры соблюдаются; hit-test корректен при прокрутке; нет «уплывания»; preview не меняет Layout; один жест = один undo-шаг |
| **4. Drag & Drop** | `DragController`, `DropOverlay`, `DragGhost`, все типы целей (`Tabs`, `GroupEdge`, `MainColumn`, `ColumnSlot`, `FreeArea`, rail/flyout/floating) | Любая допустимая вкладка/группа переносится по legal zones; запрещённые зоны не подсвечиваются; pointer capture/cancel корректны; Esc отменяет; команда применяется ровно один раз |
| **5. Рейки и flyout** | `RailView` (2 режима), `FlyoutView`, auto-collapse, resize и drag из flyout | Переключение Expanded⇄Icons сохраняет структуру вкладок; flyout не двигает Canvas; transient действия не засоряют Undo |
| **6. Floating** | `FloatingWindowView` внутри главного окна, z-order, resize 8 ручками, перемещение и dock-back | Floating не создаёт OS-окон; границы и размеры нормализует Core; фокус/подъём работают; floating закрывается/докится обратно |
| **7. Сохранение и полировка** | Workspace save/load + diagnostics/backup; chrome-режимы (Tab/Shift+Tab); тема, иконки, анимации, a11y, README, `generic_demo`, fuzz/performance | Перезапуск приложения восстанавливает рабочую область; повреждённый файл безопасно обрабатывается; приложение A пройдено; clippy/tests чистые |
**Правило перехода между этапами:** следующий этап начинается после прохождения критериев текущего этапа. Не создавать заранее пустые файлы/модули под все будущие возможности. Не объединять несколько этапов в одну большую переделку без отдельной причины и проверки промежуточного результата.
**Definition of Done (общая):** сценарии приложения A проходят; `clippy -D warnings` чисто; fuzz-тест из §10.4 стабилен на ≥ 10 000 шагов; README содержит пример интеграции (≤ 50 строк кода); все публичные типы документированы; `photoshop_demo` и `generic_demo` собираются; обычное приложение по умолчанию стартует с пустым layout.
---
## 12. Риски и оставшиеся проверки
1. **GPUI API и ревизия.** Для текущего workspace зафиксирована ревизия `9cd14daad768fbfaeeac8789f515f9a5f67669a6` и Rust `1.99.1`. Перед реализацией каждого UI-слоя сверять точные сигнатуры pinned GPUI; не переносить API из другой версии Zed/GPUI. Обновление ревизии не входит в обычную реализацию этапов.
2. **Floating внутри окна.** Для v1 решение окончательное: `FloatingWindowView` — абсолютный слой поверх Canvas внутри главного окна GPUI. OS-native окна, tear-off, `open_window` для каждой панели и платформенные window hacks запрещены в рамках этого ТЗ.
3. **История интерактивных жестов.** Для v1 принято: preview во время pointer_move, одна финальная команда на mouse_up. Live-мутирование Layout на каждом кадре и расширение Core API откладываются до подтверждённой необходимости.
4. **Метрики `dock-core`.** `DockMetrics::default()` содержит предварительные значения. Их нужно визуально сверить с reference Photoshop на этапе 1/3. Числа нельзя компенсировать локальными константами в GPUI; при необходимости изменяются метрики в Core и тесты Core.
5. **Манифест `dock-core`.** В текущем репозитории `crates/dock-core/Cargo.toml` есть. Этап 0 проверяет его совместимость с workspace и `serde`; добавлять дублирующий манифест/проект не нужно.
6. **Вертикальная прокрутка.** В Core есть `VerticalOverflow { viewport, content_extent }`; scroll offset в v1 — временное UI-состояние, привязанное к NodeId. Обязательно проверить соответствие hit-test после scroll offset и очистку состояния после изменения дерева. Если такого преобразования недостаточно для корректного UX, сначала расширяется контракт Core.
7. **PanelView lifecycle.** В v1 фабрика панели возвращает `AnyView`; отдельный обязательный trait с `render/on_resize/on_focus` не вводится. GPUI view отвечает за свой внутренний lifecycle. Специальные hooks добавлять только при подтверждённой необходимости конкретного интеграционного сценария.
8. **Кэш геометрии.** До начала массовой реализации проверить реальное поведение `Layout.version()` и `Layout::update_canvas_bounds()` в закреплённой версии. Убедиться, что обновление Canvas bounds не вызывает бесконечную последовательность compute/render.
9. **Производительность.** Цель 60 FPS требует зафиксированной reference machine, release-профиля и воспроизводимого тестового layout. До фиксации reference machine это целевой ориентир, а не универсальная гарантия для любого устройства.
10. **Поддержка платформ.** Наличие общего GPUI-кода не является доказательством поддержки каждой ОС. Платформы подтверждаются сборками и smoke-test с pinned backend.
11. **Файловая атомарность.** Поведение замены файла различается на ОС. Persistence должен использовать корректную схему временный файл → sync → atomic replace/backup, не удаляя заранее единственную рабочую копию.
12. **Тестовый harness GPUI.** Возможности `TestAppContext` проверяются именно на pinned revision. Если headless-сценарий недоступен, тестируется логика контроллера отдельно и фиксируется ручной UI smoke-test; фиктивная интеграционная проверка не допускается.
---
## Приложение A. Чек-лист паритета с Photoshop
- [ ] Две стороны дока (лево/право), до `max_columns` колонок на сторону; колонки меняют ширину; канвас не сжимается ниже `min_canvas_width`.
- [ ] Группы вкладок: переключение, перестановка, закрытие, перенос между группами и колонками. `singleton` не мешает закрыть единственный экземпляр.
- [ ] Двойной клик по вкладке/пустому месту таб-стрипа сворачивает группу до высоты заголовка; перенос вкладки в свёрнутую группу разворачивает её.
- [ ] Колонка переключается: полная ⇄ иконки+подписи ⇄ только иконки; структура групп сохраняется.
- [ ] Клик по иконке рейки открывает flyout поверх канваса; клик вне/Esc закрывает; ширина flyout запоминается.
- [ ] Перетаскивание группы за grip; слияние групп в одну (Center), раскладка по краям, новая колонка у края дока.
- [ ] Вынос вкладки/группы на свободное место канваса → плавающее представление внутри главного окна; оно двигается, ресайзится, поднимается кликом, закрывается и докится обратно. Дополнительное OS-окно не создаётся.
- [ ] Tab — скрыть всё; Shift+Tab — скрыть только док; повтор возвращает.
- [ ] Undo/Redo структурных изменений; один resize/drag-жест = один Undo-шаг; перезапуск восстанавливает раскладку, а повреждённый файл сохраняется в backup перед fallback.
- [ ] Узкое окно / очень высокие панели — вертикальный скролл колонки, корректный hit-test при прокрутке, без сжатия групп ниже минимума и без краша.
- [ ] Обычный `app` стартует с пустым Layout; четыре демо-панели автоматически появляются только в `photoshop_demo`.
- [ ] Неизвестный `PanelKey` после загрузки показывает placeholder и не удаляется из Layout.
## Приложение B. Минимальный пример интеграции (целевой API)
Пример ниже показывает целевую интеграцию приложения с четырьмя демонстрационными панелями. Это схема публичного API, а не утверждение, что код уже компилируется до реализации этапов 0–1: точные типы closure и GPUI builder должны быть подтверждены на закреплённой ревизии.
```rust
let registry = PanelRegistry::new()
    .register(PanelDescriptor::new(
        "layers", "Layers", icons::LAYERS, layers_constraints,
        |window, cx| cx.new(|cx| LayersPanel::new(window, cx)).into(),
    ))
    .register(/* color, properties, history */);
let layout = create_initial_layout(
    Profile::Photoshop,
    &registry.descriptors_for_core(),
    &DockMetrics::default(),
)?;
let controller = DockController::new(layout, DockMetrics::default(), registry)?;
DockHost::new(controller)
    .theme(DockTheme::photoshop_dark())
    .toolbar(|_, _| my_toolbar())
    .options_bar(|_, _| my_options_bar())
    .canvas(|_, cx| cx.new(|cx| MyCanvas::new(cx)).into())
```
Обычное приложение может вместо фабрики начальной раскладки создать пустое состояние:
```rust
let layout = Layout::new(Profile::Photoshop);
let controller = DockController::new(layout, metrics, registry)?;
let host = DockHost::new(controller).canvas(|_, _| my_empty_workspace());
```
## Приложение C. Зафиксированные решения v1.1
| Вопрос | Решение |
|---|---|
| Старт обычного приложения | Пустой Layout; верхнее меню и Canvas принадлежат приложению |
| Photoshop demo | Четыре начальные панели `layers`, `color`, `properties`, `history` |
| Generic demo | Отдельный пример с `Profile::Generic` |
| Дерево и геометрия | Только `dock-core`; GPUI не хранит копию дерева и не пересчитывает layout |
| Panel view | Фабрика возвращает `AnyView`; отдельный lifecycle trait не обязателен в v1 |
| `singleton` | Запрещает дублировать экземпляр, но не запрещает закрыть панель |
| Floating | Внутри главного окна приложения; нативные OS-окна вне v1 |
| Drag/resize + Undo | Preview во время жеста, одна команда и один history step на mouse_up |
| Scroll | Offset — временное состояние адаптера, `VerticalOverflow` и прямоугольники — из Core |
| Workspace path | Передаётся приложением; библиотека не выбирает имя приложения/config directory |
| GPUI/Rust | GPUI revision `9cd14daad768fbfaeeac8789f515f9a5f67669a6`, Rust `1.99.1`, edition `2024` на дату версии документа |
Эти решения считаются базовым контрактом v1. Изменять их можно только осознанно: вместе с причиной, влиянием на границы Core/GPUI и обновлением тестов/критериев приёмки.
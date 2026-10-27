# ТЗ: Dock-система в стиле Photoshop на dock-core + GPUI

Oct 24, 2026 · @master

## Текущая точка продолжения

- **Активная точка:** этап 11 — завершена реализация маршрутизации drop-запросов и внесена матрица интеграционных тестов; осталось подтвердить её Cargo-прогоном перед переходом к следующему этапу.
- **Этап 10:** обработчики `mouse down/move/up`, reusable resolver, target/preview и drag ghost внесены.
- **Преобразование запроса:** `Layout::resolve_drag_drop(&DragDropRequest, &DockMetrics)` возвращает готовый `DragDropAction::Command(DockCommand)` либо `NoOp`. `dock-gpui/src/dock/drop_commit.rs` остаётся тонким адаптером; историю и единственный коммит изменения контролирует `DockController`.
- **Матрица этапа 11:** проверяются `MovePanel` для `GroupEdge`/`ColumnSlot`/`RailSlot`/`MainColumn`, перестановка вкладки и отделение вкладки в группу по краю; `SplitGroup`, `MergeGroup`, `MoveGroup` для обычного и rail-слота, `MoveGroupToMainColumn`; отрыв панели/группы в `FreeArea`; перенос из flyout. Для успешных перемещений проверяются одна запись Undo и циклы Undo/Redo, а для несовместимых комбинаций целей — отсутствие изменений дерева и истории.
- **FreeArea / floating bounds:** адаптер получает прямоугольник источника из актуальной геометрии, сохраняет смещение захвата, учитывает title bar и ограничивает границы viewport; затем заполняет `floating_bounds` до вызова Core. Политика находится в `dock-gpui/src/dock/floating_policy.rs`.
- **Контракт коммита:** `Command` применяется один раз через историю; `NoOp` не меняет дерево, версию или историю и сохраняет ветку Redo. Ошибки подготовки/разрешения пишутся в `last_error`; успешный `NoOp` очищает старую ошибку. `Layout::apply` транзакционен: рабочее дерево заменяется только после успешной нормализации и валидации.
- **Контракт устаревшей геометрии:** если в активном drag меняются версия `Layout`, viewport или `DockMetrics`, контроллер отменяет жест и возвращает `StaleFrameGeometry`, не используя прежний resolver. Устаревший frame на момент пересечения порога тоже отменяет pending-жест; `MainWindow` перерисовывает поверхность после ошибки обновления.
- **FloatingTitle vs FloatingCapture:** `FloatingTitle` формирует `MoveFloating` без drop-target. Отдельный `FloatingCapture` исключает `FreeArea` и собственное дерево, а Core повторно проверяет цель и формирует `DockFloating`. Захватная кнопка скрыта для корневых `Column`/`Split` в Photoshop-профиле, поскольку текущий mapping не стыкует эти корни одной командой.
- **Проверки добавлены, Cargo ещё не запускался:** кроме предыдущего покрытия добавлены матрицы GroupGrip, PanelTab и FlyoutPanel для GroupEdge/ColumnSlot/RailSlot/MainColumn; проверены точные `SplitGroup`, `MoveGroupToMainColumn`, `MoveGroup` на ColumnSlot/RailSlot и `MovePanel` для панелей из dock/flyout; перестановка вкладки и отделение вкладки по краю собственной группы; стыковка плавающего окна с корнем `Panel`; отклонение несовместимых сочетаний `Panel → GroupTabs` / `Group → Tabs`.
- **Очистка warning из последнего лога:** `DragUpdateError::Core/Geometry` реализует `Display` и `Error`; `ResizeSession::pointer_start` оставлен только под `cfg(test)`; удалён `localize_tree_rects); `RailEntry` использует `rail_entry_label`; неиспользуемые привязки `id` и `side` устранены.
- **Статус компиляции:** новый и прежний тестовый код не подтверждён компилятором. По договорённости Cargo не запускался; до реального прогона не считать тесты зелёными.


## 1. Что такое render.rs и почему он главный

`render.rs` — это готовый адаптер «ядро → GPUI» (судя по `use ui::Theme` и вашему `dock_gpui::dock_metrics_from_theme`, он лежит в крейте `dock-gpui`). Он **ничего не хранит и не меняет**: берёт `&Layout` и свежие `LayoutRects` от ядра и рисует всё как абсолютно позиционированные прямоугольники. Любое действие пользователя он превращает не в изменение дерева, а в *запрос*: `DockCommand`, `DockMenuRequest`, `DragPayload + DragHandle`, `ResizeHandle`. Выполняет эти запросы приложение.

Это ровно та архитектура, к которой я вёл аудит `main_window.rs`: там геометрия (`floating_drag_bounds`, `group_drag_bounds`, `floating_size_for_detach`), запросы к дереву (`group_has_header`, `subtree_contains_node`) и жесты жили в самом окне и дублировали ядро. В `render.rs` всё это уже отдано ядру: рейка и flyout (`column_display_order`, `layout.flyout()`), прокрутка колонок (`vertical_overflow`), переполнение вкладок (`tab_overflows`), зоны стыковки (`DropZoneRect`), ручки ресайза плавающих окон (`floating_resize_edges`), порядок наложения окон (`floating_groups_in_paint_order`).

**Вывод:** `main_window.rs` в текущем виде выбрасывается как рендерер и остаётся только как тонкий «хозяин»: хранит состояние, исполняет команды и жесты. Он переезжает из `app` в `dock-gpui`, а в `app` остаётся лишь запуск. Всё, что рисует, живёт в `dock-gpui` и собирается из компонентов.

Ссылка на правду об API: часть типов из `render.rs` (`DragHandle`, `DragPreview`, `DropZoneRect`, `ResizeEdge`, `DockMenuRequest`, `column_display_order`, `flyout()`) в справочнике `dock-core-command-reference.md` не описана, потому что он описывает только `DockCommand`. Поэтому в разделе 11 я перечислил, какие файлы ядра нужно прислать для следующего шага.

## 2. Цели, принципы и границы

Цель: dock-система как у Photoshop (колонки, группы вкладок, рейка с flyout, плавающие окна, стыковка перетаскиванием, Undo/Redo), где **любая изменяющая операция — это `DockCommand`**, а любой элемент интерфейса — маленький переиспользуемый компонент GPUI.

### Слои и кто за что отвечает

| Слой | Где живёт | Владеет | Не делает |
| --- | --- | --- | --- |
| Ядро | `dock-core` | `Layout`, `DockCommand`, `Layout::apply`, `compute_rects`, `compute_drop_zones`, `DragSession`, `UndoRedoHistory`, валидация, workspace (serde) | Ничего не знает про GPUI, мышь, клавиатуру, цвета |
| Адаптер | `dock-gpui` (`components/`, `layers/`, `surface.rs`, `frame.rs`) | Компоненты, `DockSurface`, перевод геометрии ядра в элементы, подгонка геометрии под Photoshop (`normalize_docked_group_geometry`) | Не хранит `Layout`, не вызывает `apply` напрямую, не клонирует `Layout`, не импортирует `dock/`, `menus/`, `panels/` |
| Хозяин | `dock-gpui` (`dock/`, `menus/`, `panels/`, `main_window.rs`) | Состояние (`DockController`), история, сессии жестов, меню, горячие клавиши, реестр панелей и их содержимое, workspace-файл | Не считает геометрию и не правит дерево вручную |
| Запуск | `app` (`main.rs`) | Только вызов `dock_gpui::run(...)` и платформенные ресурсы сборки | Никакой dock-логики |

### Правила (нарушение = дефект при ревью)

1. **Один владелец `Layout`** — `DockController`. Рендер получает `&Layout`, компоненты — только данные из `LayoutRects`.
2. **Компоненты без состояния и без знания о `Layout`.** Компонент получает прямоугольник, текст, цвета и колбэки (`on_click`, `on_drag_start`). Типы `Node`/`Layout` в его полях не появляются; допустимы только идентификаторы (`NodeId`, `PanelId`) для стабильных `ElementId`.
3. **Дерево меняется только через `DockCommand`.** В коде приложения не должно быть вызовов `layout.apply` вне `DockController`, и все они идут через `UndoRedoHistory::apply`, если действие должно отменяться.
4. **Временное UI-состояние не попадает в историю**: flyout, `ResizeFlyout`, `SetFlyoutAutoCollapse` — только через команды ядра, но без записи в Undo (ядро само это решает).
5. **Жест = сессия**. Начало (mouse down) → обновления (mouse move) → коммит или отмена (mouse up / Esc / потеря фокуса). В конце жеста в историю попадает **одна** команда.
6. **Ровно один слушатель `on_drag_move`/`mouse_move` на корне `DockSurface`**, не по одному на каждую ручку (в текущем коде это была проблема).
7. **Никаких разовых `Layout::clone()` в кадре.** Снимок делает сессия один раз при старте жеста (или это делает `DragSession`/`UndoRedoHistory` внутри ядра).
8. **Ответственность важнее размера файла**: ~300 строк — ориентир для ревью, а не повод автоматически дробить код. Выделяйте модуль по цельной предметной подсистеме и связанным операциям; не создавайте production-файл для одной функции по умолчанию. Небольшие помощники остаются рядом с вызывающей логикой, если у них нет самостоятельной ответственности, отдельного типа/API, повторного использования или независимого алгоритма. Обычно цельный модуль в 150–400 строк предпочтительнее множества файлов по 20–80 строк; больший файл можно оставить целым, если разделение ухудшает навигацию. Перед разбиением перечислите границы и назначение будущих модулей. Функция — около 60 строк как ориентир; более 5 аргументов = структура параметров (`Props`/context).

### Как будут оформлены компоненты

Каждый — `#[derive(IntoElement)] struct X { props }` + `impl RenderOnce for X`. Поля: `rect: Rect`, данные подписи/иконки, `theme: &Theme` (или набор цветов), колбэки (`CommandClickHandler`, `DragStartHandler`, `ResizeStartHandler` — те же типы, что уже объявлены в `render.rs`). Вызывать `layout.*` из `render()` компонента нельзя.

## 3. Целевая структура файлов

**Главное правило раздела:** вся логика dock-системы живёт в одном крейте — `dock-gpui`. Крейт `app` содержит только запуск (`main.rs` и ресурсы сборки). `dock-core` и `ui` не меняются вообще: ни файлов, ни `Cargo.toml`, ни тестов.

Пометки: «уже есть» — готово (`dock-core`, `ui`, `metrics.rs`, `render.rs`), `NEW` — создаётся, «из render.rs» — выделяется из готового `render.rs`. В скобках — номер этапа из разделов 4–8, где файл создаётся.

```text
crates/
├─ dock-core/                      НЕ ТРОГАЕМ. Только читаем и вызываем.
├─ ui/                             НЕ ТРОГАЕМ (Theme, colors, metrics)
│
├─ dock-gpui/                      ВСЁ про dock: адаптер + состояние + меню + панели + главное окно
│  ├─ Cargo.toml                   зависимости: dock-core, ui, gpui, serde, serde_json   (уже есть, этап 0 — только сверка)
│  ├─ examples/
│  │  └─ components_gallery.rs     галерея компонентов в разных состояниях               (NEW, этап 4)
│  ├─ tests/
│  │  ├─ frame_geometry.rs         FrameGeometry::compute идемпотентна, зоны = рисунок   (NEW, этап 3)
│  │  ├─ drop_commit.rs            таблица «цель → команды» без GPUI                     (NEW, этап 11)
│  │  └─ workspace_roundtrip.rs    save → load → validate                                (NEW, этап 15)
│  └─ src/
│     │
│     │  ── Точка входа (то, что вызывает app) ─────────────────────────────────────────
│     ├─ lib.rs                    публичный API: run, RunOptions, DockSurface, DockHost, типы запросов (NEW)
│     ├─ entry.rs                  run(RunOptions): Application, init темы, открыть окно, подключить assets  (NEW, этап 6)
│     ├─ main_window.rs           тонкий Render: DockController + DockSurface + меню-полоса  (NEW, этапы 6–7)
│     │
│     │  ── Адаптер ядро → GPUI, БЕЗ состояния ─────────────────────────────────────────
│     ├─ render.rs                 готовый адаптер; его содержимое раскладывается по файлам ниже (уже есть)
│     ├─ metrics.rs                dock_metrics_from_theme                               (уже есть)
│     ├─ host.rs                   trait DockHost + CommandClickHandler, DragStartHandler, ResizeStartHandler, NoopHost (NEW, этап 5)
│     ├─ requests.rs               DockMenuRequest, ResizeHandle, DragOverlay            (из render.rs, этап 5)
│     ├─ frame.rs                  FrameGeometry: compute_rects + normalize + кэши       (NEW, этап 3)
│     ├─ chrome_geometry.rs        normalize_docked_group_geometry                       (из render.rs, этап 3)
│     ├─ geometry.rs               translate / localize / scroll_rect / union_rects      (из render.rs, этап 2)
│     ├─ color.rs                  blend_rgb                                             (из render.rs, этап 2)
│     ├─ labels.rs                 rail_entry_label, tab_header_label                    (из render.rs, этап 2)
│     ├─ tree_query.rs             subtree_nodes, active_panel, main_column_ids …        (из render.rs, этап 2)
│     ├─ surface.rs                DockSurface — сборщик слоёв                           (из render.rs, этап 6)
│     ├─ layers/                   крупные слои кадра (один слой = одна функция)
│     │  ├─ mod.rs
│     │  ├─ main_areas.rs          главные dock-области + scroll-колонки
│     │  ├─ outer_edges.rs         ручки внешнего края колонок
│     │  ├─ floating.rs            плавающие окна + их ручки ресайза (по окну!)
│     │  ├─ flyout.rs              оверлей flyout
│     │  ├─ drop_overlay.rs        зоны стыковки + маркер
│     │  └─ tree.rs                tree_elements, разбитый на подслои
│     ├─ components/               маленькие stateless-компоненты (этап 4)
│     │  ├─ mod.rs
│     │  ├─ rect_box.rs            абсолютный прямоугольник (rect_element из render.rs)
│     │  ├─ focus_ring.rs          общий focusable + focus_visible
│     │  ├─ icon_button.rs         ☰, ⋯, ×, >>  (один компонент вместо трёх)
│     │  ├─ panel_tab.rs           одна вкладка
│     │  ├─ tab_strip.rs           фон полосы вкладок
│     │  ├─ tab_overflow_button.rs
│     │  ├─ group_menu_button.rs
│     │  ├─ group_grip.rs          тонкая ручка перетаскивания группы (4 px)
│     │  ├─ group_frame.rs         фон + рамка группы
│     │  ├─ panel_surface.rs       контейнер содержимого панели
│     │  ├─ splitter_handle.rs     полоса + hit-зона сплиттера
│     │  ├─ resize_edge_handle.rs  ручка края/угла плавающего окна
│     │  ├─ floating_title.rs      заголовок окна (+ кнопки close/collapse)
│     │  ├─ rail_entry.rs          пункт рейки (иконка / иконка+подпись)
│     │  ├─ flyout_header.rs
│     │  ├─ drop_zone.rs           одна зона стыковки (active / idle)
│     │  ├─ column_mode_button.rs  кнопка >> / <<                                    (NEW, этап 8)
│     │  ├─ drag_ghost.rs          «призрак» под курсором                            (NEW, этап 10)
│     │  ├─ flyout_resize_handle.rs ручка края flyout (или переиспользовать resize_edge_handle) (NEW, этап 14)
│     │  └─ popup_menu.rs          общая оболочка выпадающего меню                   (NEW, этап 13)
│     │
│     │  ── Хозяин: состояние, жесты, команды  ─────────────────────────
│     ├─ panels/
│     │  ├─ mod.rs                реестр PanelDef { key, title, icon, constraints, content }  (NEW, этап 1)
│     │  └─ content.rs            фабрики содержимого панелей                        (NEW, этап 1)
│     ├─ dock/
│     │  ├─ mod.rs
│     │  ├─ controller.rs         DockController: Layout + History + Session + apply  (NEW, этап 7)
│     │  ├─ host_impl.rs          impl DockHost (колбэки → cx.listener)               (NEW, этап 7)
│     │  ├─ commands.rs           готовые команды-хелперы (close_panel, close_floating …)  (NEW, этап 8)
│     │  ├─ resize_session.rs     сессия ресайза                                      (NEW, этап 9)
│     │  ├─ drag_session.rs       обёртка над dock_core::DragSession                  (NEW, этап 10)
│     │  ├─ drop_commit.rs        DragDropAction -> Vec<DockCommand>                  (NEW, этап 11)
│     │  ├─ floating_policy.rs    raise, начальные bounds, клампы                     (NEW, этап 12)
│     │  ├─ rail_policy.rs        пороги ширины -> SetColumnMode, flyout              (NEW, этап 14)
│     │  ├─ keyboard.rs           Esc, Tab, Ctrl+Z / Ctrl+Shift+Z                     (NEW, этап 14)
│     │  └─ workspace.rs          загрузка/сохранение раскладки (serde)               (NEW, этап 15)
│     └─ menus/
│        ├─ mod.rs                MenuState: кто открыт, где, закрытие               (NEW, этап 13)
│        ├─ dockers_menu.rs       меню «Dockers» в верхней полосе
│        ├─ group_menu.rs         ☰ меню группы / панели
│        └─ overflow_menu.rs      ⋯ список скрытых вкладок
│
└─ app/                            ТОЛЬКО ЗАПУСК
   ├─ Cargo.toml                   зависимости: dock-gpui, ui, gpui (больше ничего)  (уже есть)
   ├─ build.rs / manifest / icon   платформенные ресурсы сборки
   ├─ assets/                      иконки, шрифты
   └─ src/
      └─ main.rs                  ~20–30 строк: собрать RunOptions и вызвать dock_gpui::run   (NEW, этап 6)
```

Почему так: папка `components/` — «кирпичи», которые можно разрабатывать и смотреть по одному; `layers/` — «слои кадра», один слой = одна функция, которая собирает компоненты для своей части `LayoutRects`; `surface.rs` лишь упорядочивает слои (z-order). Папки `dock/`, `menus/`, `panels/` и `main_window.rs` — «хозяин», который владеет состоянием и исполняет запросы адаптера.

### 3.1. Что остаётся в `app`

Весь `main.rs` — это сборка параметров и один вызов:

```rust
// app/src/main.rs  (эскиз; точные типы — из ваших ui и gpui)
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    dock_gpui::run(dock_gpui::RunOptions {
        title: "Dock".into(),
        assets: Some(Box::new(Assets)),   // источник иконок/шрифтов из app/assets, если нужен
        workspace_path: None,             // по умолчанию — путь из конфигурации
    });
}
```

- `cfg(windows)` допускается только здесь и в `build.rs`: подсистема окна, иконка, манифест.
- В `app` нет `use dock_core`, нет `Layout`, нет `DockCommand`, нет GPUI-компонентов. Если для запуска понадобилась такая строка, значит, она должна жить в `dock-gpui`.
- `app/src` содержит только `main.rs`. Реализация `Assets` (`AssetSource`), если нужна, лежит там же рядом с `main()`.

### 3.2. Публичный API `dock-gpui` (то, что видит `app`)

| Символ | Назначение |
| --- | --- |
| `run(RunOptions) -> !` | Создаёт `Application`, инициализирует тему из `ui`, регистрирует assets, открывает `MainWindow`, запускает цикл событий |
| `RunOptions { title, assets, workspace_path }` | Все параметры запуска. Только простые типы, без `Layout` |
| `DockSurface`, `DockHost`, `NoopHost`, `FrameGeometry` | Остаются публичными для галереи, тестов и возможного встраивания в другие окна |
| `components::*` | Публичны ради `examples/components_gallery.rs` |

Всё остальное (`dock::*`, `menus::*`, `panels::*`, `main_window`) помечается `pub(crate)`. Так `app` физически не сможет залезть в состояние dock-системы.

### 3.3. Правила зависимостей внутри `dock-gpui`

Один крейт не должен превратиться в кашу, поэтому слои различаются **по папкам**, а не по крейтам. Стрелка «→» означает «может импортировать».

```text
main_window.rs, entry.rs  →  dock/, menus/, panels/, surface.rs, frame.rs, host.rs
dock/, menus/, panels/    →  host.rs, requests.rs, frame.rs, components/popup_menu.rs, dock-core
surface.rs, layers/       →  components/, frame.rs, host.rs, requests.rs, geometry.rs, tree_query.rs, dock-core (только чтение)
components/               →  ui::Theme, requests.rs, host.rs (только типы колбэков), gpui. НЕ dock-core::Layout/Node
```

Запрещено (проверяется в CI):

1. `components/`, `layers/`, `surface.rs`, `frame.rs`, `geometry.rs`, `tree_query.rs` **не импортируют** `crate::dock`, `crate::menus`, `crate::panels`, `crate::main_window`. Проверка: `grep -rnE "crate::(dock|menus|panels|main_window)" src/components src/layers src/surface.rs src/frame.rs` — пусто.
2. Файлы без GPUI: `dock/resize_session.rs`, `dock/drop_commit.rs`, `dock/floating_policy.rs`, `dock/rail_policy.rs`, `dock/commands.rs`, `dock/workspace.rs`, а также `geometry.rs`, `color.rs`, `tree_query.rs`, `labels.rs`. Проверка: `grep -n "gpui" ` по этим файлам — пусто. Тогда их тесты не требуют окна.
3. `layout.apply` и `UndoRedoHistory::apply` вызываются только в `dock/controller.rs` (правило 3 из раздела 2 остаётся в силе).
4. `components/` не импортируют `dock-core::Layout` и `Node` (допустимы `NodeId`, `PanelId`).
5. `dock-core/` и `ui/` в ходе работ не меняются. Проверка в CI: `git diff --exit-code origin/master -- crates/dock-core crates/ui`. Если нужна правка ядра (например, `CloseFloating`), она оформляется отдельным PR и согласуется заранее.

### 3.4. `Cargo.toml`

`dock-gpui/Cargo.toml` (эскиз, версии и имена — как в вашем workspace):

```toml
[dependencies]
dock-core  = { path = "../dock-core" }
ui         = { path = "../ui" }
gpui       = { workspace = true }
serde      = { workspace = true }       # workspace.rs; derive нужен только если типы сериализуются здесь
serde_json = { workspace = true }
anyhow     = { workspace = true }       # если уже используется в проекте
```

`app/Cargo.toml`:

```toml
[dependencies]
dock-gpui = { path = "../dock-gpui" }
ui        = { path = "../ui" }          # только если Assets/тема инициализируются в main.rs
gpui      = { workspace = true }
```

В `app` убираются прямые зависимости от `dock-core` и всего, что нужно только dock-системе. `dock-core/Cargo.toml` не редактируется.

## 4. Этапы 0–3: подготовка и фундамент

Каждый этап должен заканчиваться собирающимся проектом (`cargo build`, `cargo test`, `cargo clippy -D warnings`). Не начинайте следующий этап, пока предыдущий не прошёл критерии приёмки.

### Этап 0. Базовая линия и сверка API

- **Цель:** зафиксировать, что именно предоставляет ядро, прежде чем писать код.
- **Файлы:** кода ядра не трогаем. Обновить `dock-core-command-reference.md`.
- **Шаги:**
  1. Ветка `refactor/dock-surface`, тег на текущем состоянии.
  2. Сверить варианты `DockCommand` в исходнике с справочником: старый `main_window.rs` использует `DockCommand::ResizeSplitColumnBoundary`, которого **нет** в 34 вариантах справочника. Справочник устарел или вариант добавили позже; исправить справочник.
  3. Выписать в `docs/core-api-used.md` типы и методы, которые тянет `render.rs`, но которых нет в справочнике (см. раздел 11) и пометить прочитанные в исходниках ядра.
  4. Вынести платформенный `cfg(windows)` только на то, что действительно платформо-зависимо (`app/src/main.rs`, `build.rs`): тогда `cargo test` будет работать на Linux-CI.
- **Приёмка:** все варианты `DockCommand` в исходнике совпадают со справочником; тесты `dock-core` зелёные; `git diff -- crates/dock-core crates/ui` пуст.

### Этап 1. Реестр панелей

- **Цель:** одно место, где описаны панели. Сейчас ключи `layers/color/properties/history` и подписи «Вкладка 1…4» размазаны по `build_core_layout`, `panel_label`, `panel_open_target` и меню.
- **Файлы:** `dock-gpui/src/panels/mod.rs` (NEW), `dock-gpui/src/panels/content.rs` (MOVE из `tabs_group/content.rs`).
- **Содержимое `panels/mod.rs`:**
  - `pub struct PanelDef { key: &'static str, title: SharedString, icon: &'static str, constraints: fn(&Theme) -> PanelConstraints, content: fn(&PanelInstance) -> AnyElement }`
  - `pub const PANELS: &[PanelDef]` — четыре панели.
  - `pub fn def_for(instance: &PanelInstance) -> Option<&'static PanelDef>`; `pub fn descriptors(theme) -> Vec<(PanelKey, PanelConstraints)>` для `create_initial_layout`.
- **Нет в реестре:** размещения панелей (`panel_open_target`) — это решает ядро (этап 8).
- **Приёмка:** `panel_title`, `panel_icon`, `render_panel` для `render.rs` берутся из реестра; неизвестный ключ даёт заметный fallback («Missing panel»), не панику.

### Этап 2. Перенос чистых функций из `render.rs`

- **Цель:** вытащить из одного файла то, что не знает про GPUI, и покрыть тестами.
- **Файлы (`dock-gpui/src/`):**
  - `geometry.rs`: `translate_rect_to_local`, `localize_tree_rects`, `scroll_rect_in_viewport`, `union_rects`.
  - `color.rs`: `blend_rgb`.
  - `tree_query.rs`: `subtree_nodes`, `active_panel`, `first_panel_in_subtree`, `main_column_ids`, `floating_title_groups_in_tree`, `drop_target_node`.
  - `labels.rs`: `rail_entry_label`, `tab_header_label`.
- **Шаги:** перенести функции без изменений логики, перенести соответствующие юнит-тесты (`rail_modes_have_distinct…`, `collapsed_stack_marker…`, `scrolled_drop_zones…`, `floating_descendant_rects…`), добавить тест для `union_rects`.
- **Улучшения сразу:** `subtree_nodes` для каждой области вызывать ровно один раз за кадр (кеш в `FrameGeometry`, этап 3).
- **Приёмка:** `render.rs` импортирует эти функции; поведение бит-в-бит такое же; тесты проходят без GPUI-окна.

### Этап 3. `FrameGeometry`: один раз за кадр

- **Цель:** `normalize_docked_group_geometry` должна вызываться ровно один раз на свежих `LayoutRects` (так написано в её документации), и те же прямоугольники должны получать и рендер, и расчёт зон стыковки (иначе hit-таргеты расходятся с рисунком). Сейчас это условие держится только на соглашении.
- **Файлы:** `frame.rs` (NEW), `chrome_geometry.rs` (MOVE из `render.rs`).
- **API `frame.rs`:**
  - `pub struct FrameGeometry { rects: LayoutRects, nodes_by_area: HashMap<AreaKey, HashSet<NodeId>>, rail_groups: HashSet<NodeId>, panel_nodes: HashMap<PanelId, NodeId> }`
  - `FrameGeometry::compute(layout: &Layout, viewport: Rect, metrics: &DockMetrics) -> Result<Self, GeometryError>` — внутри вызывает `layout.compute_rects` и `normalize_docked_group_geometry` ровно один раз.
  - Поля приватные, доступ только через `&self`-методы. Доступа к `&mut LayoutRects` наружу нет, поэтому повторная нормализация типом невозможна.
  - Методы для drop-зон: `FrameGeometry::drop_zones(&self, layout, payload, pointer)` оборачивает `compute_drop_zones` на **этой же** геометрии (точная сигнатура — из исходников ядра, раздел 11).
- **Важное решение для обсуждения:** `normalize_docked_group_geometry` — это заплатка «генеричный заголовок + вкладки → Photoshop-анатомия» поверх готовой геометрии. В долгосрочной перспективе её лучше перенести в профиль `Profile::Photoshop` внутри `dock-core` (параметры `DockMetrics`: высота грипа, ширина меню), чтобы геометрия не существовала в двух версиях. На этом этапе делать это не нужно.
- **Приёмка:** тесты из `render.rs` (`collapsed_floating_root_tabs…`, `docked_tabs_replace_header_row…`) переехали и зелёные; новый тест: «два вызова `FrameGeometry::compute` на одном `Layout` дают идентичный результат».

## 5. Этапы 4–6: компоненты и сборка кадра

На этих трёх этапах интерактивности ещё нет: цель — чтобы картинка была та же, что у `render.rs`, но собрана из компонентов, а колбэки пока пустые.

### Этап 4. Компоненты (`dock-gpui/src/components/`)

- **Цель:** каждый кусок из `tree_elements`/`render` — отдельный файл-компонент.
- **Порядок создания** (от низа к верху, каждый — с маленьким тестом на стили/состояния, где это возможно):

| № | Файл | Что рисует / поля (props) | Заменяет | Событие → запрос |
| --- | --- | --- | --- | --- |
| 1 | `rect_box.rs` | `RectBox { rect, color }` + `.child()` — абсолютный прямоугольник | `rect_element` | — |
| 2 | `focus_ring.rs` | функция `with_focus_ring(el, theme)` (`.focusable()` + `focus_visible`) | повторяющийся блок в 6 местах | — |
| 3 | `icon_button.rs` | `IconButton { id, rect, glyph, theme, on_click }` | «☰», «⋯», «×» в flyout/вкладках, старые `close_button/collapse_button/hamburger_button` | click → `CommandClickHandler` |
| 4 | `panel_tab.rs` | `PanelTab { id, rect, label, active, theme, on_activate, on_collapse_toggle, on_drag_start }` | блок «вкладка» в `tree_elements` + `tabs_group/tab.rs` | click → `ActivateTab`; двойной click → `CollapseGroup`; mouse down → `DragPayload::Panel` + `DragHandle::PanelTab` |
| 5 | `tab_strip.rs` | `TabStrip { rect, theme }` — фон полосы | блок «strip background» | — |
| 6 | `tab_overflow_button.rs` | `TabOverflowButton { group, rect, theme, on_open }` | блок `tab_overflows` | click → `DockMenuRequest::TabOverflow` |
| 7 | `group_menu_button.rs` | `GroupMenuButton { group, panel, rect, theme, on_open }` | блок «☰» в стрипе | click → `DockMenuRequest::PanelGroup` |
| 8 | `group_grip.rs` | `GroupGrip { group, rect, theme, on_drag_start }` | блок «4 px grip» + `tabs_group/group_drag_zone.rs` | mouse down → `DragPayload::Group` + `DragHandle::GroupGrip` |
| 9 | `group_frame.rs` | `GroupFrame { rect, theme }` — фон + рамка группы | блок «группы» | — |
| 10 | `panel_surface.rs` | `PanelSurface { rect, theme, content: AnyElement }` | блок `rects.panels` + `tabs_group/content.rs` | — |
| 11 | `splitter_handle.rs` | `SplitterHandle { id, rect, axis, theme, on_resize_start: Option<..> }` | блок `rects.splitters` | mouse down → `ResizeHandle::Splitter` |
| 12 | `resize_edge_handle.rs` | `ResizeEdgeHandle { id, rect, edge, theme, on_resize_start }` | цикл `floating_resize_edges` + внешняя ручка колонки | mouse down → `ResizeHandle::{OuterColumn, FloatingEdge}` |
| 13 | `floating_title.rs` | `FloatingTitle { group, rect, title, theme, on_drag_start, on_close, on_collapse }` | блок «title» в плавающем окне | mouse down → `DragPayload::FloatingGroup` + `DragHandle::FloatingTitle` |
| 14 | `rail_entry.rs` | `RailEntry { id, rect, mode, icon, title, active, theme, on_open }` | цикл рейки в `tree_elements` | click → `OpenFlyout` |
| 15 | `flyout_header.rs` | `FlyoutHeader { rect, title, theme, on_menu, on_close }` | шапка flyout | → `PanelGroup` menu, `CloseFlyout` |
| 16 | `drop_zone.rs` | `DropZone { rect, active, theme }` — полупрозрачная зона | цикл `overlay.zones` | — |

- **Правило для всех:** `ElementId` строится из типа и id узла без `format!` на каждый кадр (кортеж `("dock-tab", id)`, если ваша версия GPUI такое позволяет — иначе `SharedString`).
- **Правило для всех:** рамки `border_1()` только на внешних рамках (`GroupFrame`, плавающее окно, flyout), не на каждом вложенном элементе (иначе линии удваиваются).
- **Работа визуально:** создать `dock-gpui/examples/components_gallery.rs` — окно, где каждый компонент нарисован в разных состояниях (active/idle, focus, длинный текст). Это ускорит подгонку стиля.
- **Приёмка:** все 16 файлов собираются; галерея запускается; ни один компонент не импортирует `Layout`/`Node` (проверяется `grep -r "dock_core::Layout\|Node" components/` — пусто).

### Этап 5. Контракт «адаптер ↔ приложение»: `DockHost`

- **Цель:** вместо `render<F, T, I, H, M, D, R>` с семью дженериками и передачей 11 аргументов в каждую внутреннюю функцию — один trait.
- **Файлы:** `host.rs` (NEW), `requests.rs` (MOVE: `DockMenuRequest`, `ResizeHandle`, `DragOverlay`).
- **Содержимое `host.rs`:**
  - типы `CommandClickHandler`, `DragStartHandler`, `ResizeStartHandler` (как в `render.rs`);
  - `pub trait DockHost { fn panel_title(&self, &PanelInstance) -> SharedString; fn panel_icon(&self, &PanelInstance) -> &'static str; fn render_panel(&mut self, &PanelInstance) -> AnyElement; fn command(&self, DockCommand) -> CommandClickHandler; fn menu(&self, DockMenuRequest) -> CommandClickHandler; fn drag_start(&self, DragPayload, DragHandle) -> DragStartHandler; fn resize_start(&self, ResizeHandle) -> ResizeStartHandler; }`
  - `NoopHost` — реализация с пустыми обработчиками для галереи и тестов (этап 6).
- **Приёмка:** `cargo doc` показывает один публичный trait вместо семи дженериков.

### Этап 6. `DockSurface` и слои кадра

- **Цель:** разрезать `render()` на слои и полностью заменить старый рендер `main_window.rs`.
- **Файлы:** `surface.rs`, `layers/{mod,main_areas,outer_edges,floating,flyout,drop_overlay,tree}.rs` (всё NEW/REWORK из `render.rs`); `dock-gpui/src/main_window.rs` (REWORK).
- **Структура:** верхнеуровневый `DockSurface<'a> { layout: &'a Layout, frame: &'a FrameGeometry, scroll: &'a ScrollHandles, theme: &'a Theme, overlay: Option<&'a DragOverlay> }` + `fn build(self, host: &mut dyn DockHost) -> Div`. Мелкие компоненты — `RenderOnce` с владением данными; сам `DockSurface` держит ссылки, потому что `RenderOnce` требует `'static` (проверьте в вашей версии GPUI; если нет — можно и сам корень сделать `RenderOnce`).
- **Порядок слоёв (z-order, снизу вверх):**
  1. canvas (`rects.canvas`);
  2. `main_areas` — области Left/Right, непрокручиваемая часть + scroll-колонки (`vertical_overflow`);
  3. `outer_edges` — ручки внешнего края колонок;
  4. `floating` — по одному окну в порядке `floating_groups_in_paint_order`: контейнер → его дерево → title → **его** ручки ресайза;
  5. `flyout`;
  6. `drop_overlay`;
  7. меню (приложение, этап 13) — вне `DockSurface`.
- **Исправление против `render.rs`:** в оригинале все `floating_resize_edges` рисуются после **всех** окон, так что ручки нижнего окна ляжут поверх верхнего. Рисуйте ручки внутри цикла по окнам, фильтруя `rects.floating_resize_edges` по `group`.
- **Временный `main_window.rs`:** держит `Layout` и `FrameGeometry`, каждый кадр строит `DockSurface` с `NoopHost`. Работает только картинка.
- **Удалить после приёмки:** `main_window/tabs_group.rs` и все `tabs_group/*.rs`.
- **Приёмка:** визуальный паритет с текущим интерфейсом на трёх раскладках (начальная, со свёрнутой группой, с рейкой в режиме `IconsOnly`); нет роста времени кадра более чем на 10 % (замер `Instant` в debug-overlay).

## 6. Этапы 7–9: контроллер, простые команды, resize

Здесь картинка оживает. Все действия проходят через одну точку — `DockController::apply`.

### Этап 7. `DockController` и реальный `DockHost`

- **Цель:** один владелец `Layout`, одна точка применения команд, одна история.
- **Файлы:** `dock-gpui/src/dock/mod.rs`, `controller.rs`, `host_impl.rs` (NEW); `main_window.rs` (REWORK).
- **`controller.rs`:**
  - `pub struct DockController { layout: Layout, metrics: DockMetrics, history: UndoRedoHistory, scroll: HashMap<NodeId, ScrollHandle>, session: Session, menu: MenuState, last_error: Option<CommandError> }`
  - `pub enum Session { Idle, Resize(ResizeSession), Drag(DragState) }` — в один момент активна не более одной сессии. Это замена четырёх независимых `Option` из старого `MainWindow`.
  - `pub fn apply(&mut self, cmd: DockCommand) -> Result<CommandOutcome, CommandError>` — внутри `self.history.apply(&mut self.layout, cmd, &self.metrics)`; на ошибке пишется в `last_error` и лог, дерево не меняется (транзакционность гарантирует ядро).
  - `undo()`, `redo()`; `layout(&self) -> &Layout` (только чтение наружу).
  - `frame(&self, viewport) -> Result<FrameGeometry, _>` — пересчёт каждый кадр (сначала без кэша). Кэш по `new_version` вводится только после замера: flyout версию не меняет, но геометрию меняет.
- **`host_impl.rs`:** `struct WindowHost<'a, 'cx> { controller: &'a mut DockController, cx: &'a mut Context<'cx, MainWindow> }` и `impl DockHost`. Каждый обработчик — `cx.listener(move |this, ev, window, cx| { this.dock.apply(cmd.clone()); cx.notify(); })`. Требуется `DockCommand: Clone` (проверить).
- **`main_window.rs` после этапа:** `struct MainWindow { theme, dock: DockController }`; `render()` берёт `window.viewport_size()`, строит `FrameGeometry`, `DockSurface`, добавляет меню-полосу. Десятки строк, а не тысячи.
- **Приёмка:** `grep -rn "\.apply(" dock-gpui/src` находит вызов только в `controller.rs` (и в превью-пути сессий — см. этап 9); ошибка команды не роняет приложение.

### Этап 8. Простые команды «кликом»

- **Цель:** подключить всё, что не требует жестов — на них быстро видно, что контроллер работает.
- **Файлы:** `dock-gpui/src/dock/commands.rs` (NEW); новый компонент `dock-gpui/src/components/column_mode_button.rs` (NEW).
- **Работы:**
  1. `ActivateTab` — click на `PanelTab` (уже в `render.rs`), двойной click → `CollapseGroup { collapsed: !current }`.
  2. `ClosePanel` / `CloseGroup` — из меню группы (этап 13); команды уже готовы в `commands.rs`: `close_panel(panel)`, `close_group(group)`.
  3. **Закрыть всё плавающее окно:** в ядре отдельной команды нет (справочник, раздел «Известные отсутствующие команды»). Рекомендация: добавить `DockCommand::CloseFloating { group }` в `dock-core`. Если нельзя — `commands::close_floating(group)` проходит все `Tabs` дерева окна и вызывает `CloseGroup` для каждой **в одной пакетной записи истории** (если `UndoRedoHistory` такое позволяет; иначе Undo будет по шагу на группу).
  4. `toggle_panel(key)` для меню «Dockers»: если панель открыта → `ClosePanel`; иначе `OpenPanel { key, constraints, target: None }`. **Не повторять старый `panel_open_target`**: при `target: None` ядро само вернёт панель на последнее известное место или на место по умолчанию профиля. Если профиль Photoshop даёт неудовлетворительное место по умолчанию — просить правку в ядре, а не в приложении.
  5. **Режим колонки** (кнопка `>>` / `<<`): `SetColumnMode { column, mode }`, цикл `Expanded → IconsAndLabels → IconsOnly → Expanded` (в Photoshop эти стрелки переключают режим колонки, а не сворачивают одну группу). Решение по месту: кнопка стоит в стрипе первой группы каждой колонки левее `☰`, нужно зарезервировать её ширину рядом с `menu_width` в `normalize_docked_group_geometry`.
  6. **Рейка:** `OpenFlyout` (уже в `render.rs`), `CloseFlyout`, `DismissFlyout` — см. этап 14.
- **Приёмка:** по каждому пункту — ручной сценарий + юнит-тест «команда → инварианты `Layout::validate`». После `ClosePanel` и повторного `OpenPanel { target: None }` панель возвращается на прежнее место, даже если была плавающей.

### Этап 9. Ресайз через `ResizeSession`

- **Цель:** один общий механизм для всех типов `ResizeHandle`.
- **Файлы:** `dock-gpui/src/dock/resize_session.rs` (NEW). Правка `render.rs`: `supported_splitter_handle` должен поддерживать и X-сплиттеры внутри плавающих композиций (сейчас там только главные области; в старом `main_window.rs` это было, так что при переходе это регрессия).
- **Структура:** `ResizeSession { handle: ResizeHandle, baseline: Layout, pointer_start: Point, start_geometry: StartGeometry }`. `baseline` клонируется **один раз** в `begin`. Ваш приём «снимок + абсолютная дельта, а не накопление» сохраняем.
- **Цикл жеста:**
  1. `begin(handle, pointer)` — выставляет `Session::Resize`.
  2. `update(pointer)` (один слушатель `mouse_move` на корне): `candidate = baseline.clone(); candidate.apply(cmd(total_delta))`; при успехе подменяем `layout` без записи в историю; при ошибке — остаёмся на предыдущем валидном. Не отбрасывать случай `|delta| < 0.01` (было в старом коде — возврат в исходную точку должен восстанавливать baseline).
  3. `commit(pointer)` — вернуть `layout = baseline` и применить итоговую команду через `history.apply` — в истории одна запись на весь жест.
  4. `cancel()` — `layout = baseline`, `Session::Idle`.
- **Какой `ResizeHandle` → какая команда:**

| `ResizeHandle` | Команда ядра | Как считаем |
| --- | --- | --- |
| `Splitter` по `Axis::Y`, `owner` = Column | `ResizeGroups { column, boundary_index, delta: dy, available_height }` | `available_height` = высота колонки − `splitter_thickness` × (число групп − 1); вычислять в `begin`, не на каждом move |
| `Splitter` по `Axis::X`, `owner` = корень Left/Right области | `ResizeColumnBoundary { side, boundary_index, delta: dx }` | `side` определяется по `layout.main_area(side).root == owner` (**не зашивать `Right`**, как было в старом коде) |
| `Splitter` по `Axis::X` внутри плавающего `Split` | `ResizeSplitColumnBoundary { split, boundary_index, delta }` | команда была в старом коде, но не в справочнике — сверить с ядром |
| `OuterColumn { column, side }` | `ResizeColumn { column, width }` | `width = start_width ± dx` (Left: `+dx`, Right: `−dx`); `start_width` — **фактическая** ширина из `rects.columns`, не `column.width`; потолок — `min_canvas_width` из `DockMetrics` |
| `FloatingEdge { group, edge }` | `ResizeFloatingBounds { group, bounds }` | границы в момент `begin` + сдвиг соответствующих сторон по `ResizeEdge`; проверить: если ядро ограничило размер минимумом, окно не «едет» при тяге левого/верхнего края (если «едет» — брать `outcome.actual_value`) |
| двойной click по `Splitter` Y | `ResetColumnRatios { column, available_height }` | опционально |
| двойной click по `Splitter` X (главная область) | `ResetMainColumnRatios { side }` | опционально |

- **Слушатели в `surface.rs` (один набор на корне):** `on_mouse_move`, `on_mouse_up(Left)`, `on_mouse_up_out(Left)`. В `render.rs` ручки стартуют по `on_mouse_down`, а не GPUI-`on_drag`, поэтому `DragMoveEvent` здесь не нужен. Если в `mouse_move` приходит событие без нажатой левой кнопки — сессия завершается как `commit` (защита от потерянного `mouse_up`). При потере фокуса окна — `cancel`.
- **Приёмка:** (1) Ctrl+Z отменяет весь ресайз одним шагом; (2) Esc возвращает исходный размер; (3) при быстром движении и выходе курсора за окно нет «залипшей» сессии; (4) все четыре типа ручек работают и в главной области, и в плавающем окне.

## 7. Этапы 10–12: drag and drop и плавающие окна

Самая сложная часть. В старом коде всё было самописным: свои `*DragPayload`, свои превью, своя геометрия плавающего окна, и выходом было только `Float`. Теперь используем существующий API ядра: `DragSession`, `DragPayload`, `DragHandle`, `DragPreview`, `Layout::prepare_drop_resolver`, `DropZoneResolver::resolve`, `DropTarget`, `DragDropRequest`, `DragDropAction`. Сигнатуры сверены с исходниками `dock-core`; интеграция должна опираться на этот API, а не дублировать правила разрешения целей в GPUI.

### Этап 10. Сессия перетаскивания

- **Файлы:** `dock-gpui/src/dock/drag_session.rs` (NEW), правки `dock-gpui/src/dock/controller.rs`, `dock-gpui/src/dock/host_impl.rs`, `dock-gpui/src/main_window.rs`, `dock-gpui/src/surface.rs`, `dock-gpui/src/layers/drop_overlay.rs` и `dock-gpui/src/layers/tree.rs`; при необходимости — узкая корректировка API resolver в `dock-core`.
- **Состояние адаптера:** хранить источник и handle, исходную позицию, признак превышения drag threshold, ядровую `DragSession`, текущий preview и reusable resolver. Обычный click по вкладке до превышения порога должен оставаться обычным click.
- **Подготовка resolver:** после превышения порога один раз подготовить resolver от того же `FrameGeometry`, который использовался для рендера. На каждом движении вызывать `DropZoneResolver::resolve(pointer)`; не строить список зон и индекс дерева повторно при каждом mouse move.
- **Контракт reusable resolver:** `DropZoneResolver` теперь владеет подготовленным снимком: разрешёнными зонами, индексом дерева и данными корней плавающих областей. Поэтому его можно безопасно хранить между GPUI-событиями без самоссылочных структур и `unsafe`. Этот снимок не обновляется автоматически: адаптер обязан отбросить resolver перед использованием после изменения layout, нормализованной геометрии/viewport или метрик.
- **Инвалидация:** resolver действителен только для исходного снимка `Layout`, нормализованной геометрии и метрик. Если версия layout, viewport или метрики изменились до завершения жеста, отменить сессию либо безопасно пересоздать resolver от нового согласованного снимка до следующего разрешения цели.
- **Цикл жеста:**
  1. **mouse down** на поддерживаемой поверхности → `host.drag_start(payload, handle)` → `controller.begin_drag(...)`; сохранить исходную позицию, но не начинать drag до порога.
  2. **mouse move**: после достижения порога создать ядровую `DragSession`, подготовить reusable resolver и разрешить текущую цель. Далее на каждом move обновлять ядровую сессию, цель, `DragPreview` и `DragOverlay`, затем вызывать `cx.notify()`.
  3. **Дерево во время preview не меняется**. Команда применяется только на этапе 11. Resolver не переживает изменение layout, геометрии или метрик.
  4. **Визуальная обратная связь:** `DockSurface` отображает актуальные зоны и активную подсветку через `DragOverlay`; призрак `DragGhost` добавлять в рамках этого этапа, если он нужен для завершённого сценария preview, не дублируя дерево.
  5. **Завершение:** mouse up передаёт актуальную точку этапу 11. Esc, потеря фокуса окна и потерянный mouse up (движение пришло без нажатой кнопки) отменяют или корректно завершают жест по единому правилу.
- **Автопрокрутка:** при драге у края scroll-колонки (`vertical_overflow`) — отдельная отложенная задача после основной машины.
- **Приёмка:** click без движения не запускает drag; после порога зоны и маркер обновляются при движении; цель разрешается через один и тот же resolver; Esc и потеря фокуса очищают preview; при отмене `Layout::version` и состояние layout не меняются.

### Этап 11. Завершение жеста: от `DropTarget` к `DockCommand`

- **Файл:** `dock-gpui/src/dock/drop_commit.rs` — намеренно тонкий адаптер: Core уже реализует `Layout::resolve_drag_drop(&DragDropRequest, &DockMetrics)` и возвращает готовый `DragDropAction::Command(DockCommand)` или `NoOp`. Не дублировать таблицу разрешения целей в GPUI.
- **Таблица фактического поведения** сверена с `crates/dock-core/src/layout/drop_handling.rs`:

| Источник и ручка | Цель | Результат Core |
| --- | --- | --- |
| `PanelTab` / `Panel` | слот в `Tabs` своей группы | `NoOp` на текущем слоте; иначе `MovePanel { panel, target }` (перестановка вкладки) |
| `PanelTab` / `Panel` | `Tabs` другой группы | `MovePanel { panel, target }` |
| `PanelTab` / `Panel` | `GroupEdge`, `ColumnSlot`, `RailSlot`, `MainColumn` | `MovePanel { panel, target }` |
| `PanelTab` / `Panel` | `FreeArea` с рассчитанными границами | `Float { node: panel_node, bounds }` |
| `FlyoutPanel` / `Panel` | любая допустимая цель dock | `MovePanelFromFlyout { panel, target }` |
| `FlyoutPanel` / `Panel` | `FreeArea` | `FloatPanelFromFlyout { panel, bounds }` |
| `GroupGrip` / `Group` | `GroupTabs` другой группы | `MergeGroup { group, target_group, index }` |
| `GroupGrip` / `Group` | `GroupEdge { zone }` | `SplitGroup { group, target_group, zone, ratio: 0.5 }` |
| `GroupGrip` / `Group` | `ColumnSlot`, `RailSlot` | `MoveGroup { group, column, index }`; для рейки Core переводит slot index в индекс групп |
| `GroupGrip` / `Group` | `MainColumn { side, index }` | `MoveGroupToMainColumn { group, side, index }` |
| `GroupGrip` / `Group` | `FreeArea` с рассчитанными границами | `Float { node: group, bounds }` |
| `FloatingTitle` / `FloatingGroup` | target отсутствует | `MoveFloating { group, origin }` без overlay |
| `FloatingCapture` / `FloatingGroup` | цель dock вне собственного дерева | `DockFloating { group, target }` (подготовка для этапа 12 внесена ранее) |

- **Недопустимые сочетания:** `Panel → GroupTabs` и `Group → Tabs` отклоняются; `FloatingCapture → FreeArea` отклоняется. Цель внутри собственного floating tree для `FloatingCapture` даёт `NoOp`.
- **FreeArea / bounds:** `drop_commit.rs` находит прямоугольник источника из актуального `LayoutRects` (с fallback для flyout), затем `floating_policy::initial_float_bounds` сохраняет смещение захвата, учитывает title bar и ограничивает окно viewport. Bounds передаются в Core до resolve.
- **История:** один успешный drop коммитит одну команду и одну запись Undo; `NoOp` не меняет дерево/версию/историю и сохраняет Redo-ветку. Отклонённая команда не оставляет частично изменённого layout.
- **Покрытие:** тесты проверяют `MovePanel` и `MovePanelFromFlyout` для четырёх основных типов целей, перестановку вкладки и отделение по краю той же группы; `SplitGroup`, `MergeGroup`, `MoveGroup` для ColumnSlot/RailSlot, `MoveGroupToMainColumn`; `FreeArea` для панели/группы/flyout; `DockFloating` для корней `Tabs` и `Panel`; stale snapshot, отмену и отпускание вне зон; Undo/Redo и ошибки разрешения.
- **Приёмка:** код и матрица тестов внесены. Cargo-прогон ещё не выполнен; этап формально не считать прошедшим компиляцию/тесты до отдельной проверки.


### Этап 12. Плавающие окна

- **Файл:** `dock-gpui/src/dock/floating_policy.rs` (NEW).
- **Работы:**
  1. **Raise.** mouse down любой точки плавающего окна → `RaiseFloating { group }` (контейнер окна ловит событие в фазе capture, не съедая дальнейшие обработчики). Старый код этого не делал. Применять без записи в Undo (проверить, что записывает ли её `UndoRedoHistory` — если да, просить ядро исключить).
  2. **Перемещение окна** по `FloatingTitle`: снимок + абсолютная дельта, на отпускании одна запись `MoveFloating { group, origin }`; эта область заголовка не показывает drop-зоны. Для стыковки используется отдельная ручка `FloatingCapture` и `DockFloating`. В Photoshop-профиле текущий resolver поддерживает обратную стыковку корневых `Tabs`/`Panel`; для корней `Column`/`Split` захватная ручка скрыта, пока Core не предложит единый корректный mapping.
  3. **Кнопки в `FloatingTitle`:** закрыть (этап 8, пункт 3) и свернуть (`CollapseGroup` для корневой группы окна).
  4. **Политика позиции:** не давать заголовку уйти за верхний край окна и меню-бар (кламп `origin.y ≥ 0` и хотя бы N px заголовка видно по горизонтали); сверить, что уже делает ядро (в старом коде есть упоминание клампа видимого прямоугольника в `compute_rects`).
- **Удалить после этапа:** весь старый код жестов из `main_window.rs` (`TabDragPayload`, `GroupDragPayload`, `TabDragPreviewState`, `GroupDragPreviewState`, `SplitterDragState`, `update_*_from_drag`, `finish_*_drag`, `*_drag_bounds`, `docked_*_float_command`).
- **Приёмка:** окно поднимается по клику по любой его части; два окна с перекрытием не путают ручки ресайза; `grep` по удалённым именам пуст.

## 8. Этапы 13–15: меню, рейка, клавиатура, workspace, уборка

### Этап 13. Меню

- **Цель:** выпадающие меню рисуются поверх кадра как оверлей и не влияют на размеры dock (в старом коде меню «Dockers» сдвигало и сжимало весь док).
- **Файлы:** `dock-gpui/src/components/popup_menu.rs` (NEW); `dock-gpui/src/menus/{mod,dockers_menu,group_menu,overflow_menu}.rs` (NEW).
- **`popup_menu.rs`:** `PopupMenu { anchor: Point, items: Vec<MenuItem>, theme, on_dismiss }`, `MenuItem { id, label, checked, enabled, on_select }`. Рисуется поверх всего через `absolute` в корне `MainWindow`, за пределами `DockSurface`; над ним — прозрачный «щит», клик по которому закрывает меню. Кламп позиции к вьюпорту; Esc закрывает; стрелки вверх/вниз и Enter (фокус-обход — как в `render.rs`: `.focusable()` + `focus_visible`).
- **`MenuState` (`menus/mod.rs`):** `enum MenuState { Closed, Dockers, Overflow { group, anchor }, PanelGroup { group, panel, anchor } }`. Вход — `DockMenuRequest` из `render.rs` (якорь берём из `ClickEvent::position()`).
- **Содержимое меню:**

| Меню | Пункты | Команда |
| --- | --- | --- |
| Dockers | каждая панель из `PANELS`, галка «открыта» | `OpenPanel { target: None }` / `ClosePanel` |
| ☰ группы | Закрыть вкладку | `ClosePanel { panel }` |
| ☰ группы | Закрыть группу | `CloseGroup { group }` |
| ☰ группы | Свернуть / развернуть | `CollapseGroup { group, collapsed }` |
| ☰ группы | Отделить в плавающее окно | `FloatGroupAsColumn { group, bounds }` (быстрые bounds из `floating_policy`) |
| ☰ группы | Режим колонки (3 пункта) | `SetColumnMode { column, mode }` |
| ☰ группы | Вернуть пропорции групп | `ResetColumnRatios { column, available_height }` |
| ⋯ переполнение | список скрытых вкладок (из `layout`, не из `tab_headers`) | `ActivateTab { group, panel }` |
| ⋯ flyout | то же, что у ☰, но для панели flyout | — |

- **Приёмка:** меню открывается в нужном месте, закрывается кликом снаружи и Esc; при открытом меню размеры dock не меняются; пункты блокируются (`enabled = false`), если команда недопустима (например, свернуть уже свёрнутую).

### Этап 14. Рейка, flyout, клавиатура

- **Файлы:** `dock-gpui/src/dock/rail_policy.rs`, `dock-gpui/src/dock/keyboard.rs` (NEW); `dock-gpui/src/components/flyout_resize_handle.rs` (NEW, можно переиспользовать `resize_edge_handle.rs`).
- **Flyout:**
  1. Вход — click по `RailEntry` → `OpenFlyout { column, panel }` (уже в `render.rs`). Повторный click по той же иконке при включённом auto-collapse закрывает flyout — это делает ядро, приложение только шлёт `OpenFlyout`.
  2. Выход: клик снаружи flyout и Esc → `DismissFlyout`; кнопка × → `CloseFlyout`. «Клик снаружи» — `on_mouse_down_out` на контейнере flyout (в `layers/flyout.rs`).
  3. Тумблер «автосворачивание» в меню ☰ → `SetFlyoutAutoCollapse { enabled }` (временная настройка, без Undo).
  4. Ширина: ручка на левом/правом краю flyout → новый `ResizeHandle::Flyout { panel }` → `ResizeFlyout { panel, width }` (временная, без Undo; сессия та же, но `commit` не пишет в историю).
  5. Тащить панель из flyout — уже есть `DragHandle::FlyoutPanel` (этап 11).
- **Автоматический режим колонки по ширине** («сужаем колонку до порога → уходит в рейку»): в ядре такого нет — справочник явно пишет, что пороги должны быть выше уровня `SetColumnMode`. Это `rail_policy.rs`: в `ResizeSession::update` для `OuterColumn`/`Splitter X` после каждого шага сравнить текущую ширину с порогами (константы в `DockMetrics`/теме) с **гистерезисом** (вход и выход по разным порогам, иначе будет дребезг), и отправить `SetColumnMode`. В истории весь жест (ресайз + смена режима) — одна запись.
- **Клавиатура (`keyboard.rs`, актионы GPUI + карта клавиш):**

| Клавиша | Действие |
| --- | --- |
| Esc | приоритет: отменить активную сессию (drag/resize) → закрыть меню → `DismissFlyout` |
| Tab / Shift+Tab | `SetChrome { mode }` — переключение видимости chrome по циклу `ChromeMode`. Только когда фокус не в поле ввода и не в меню |
| Ctrl+Z / Ctrl+Shift+Z (Ctrl+Y) | `controller.undo()` / `redo()` |

- **Chrome (`SetChrome`):** то, что не входит в `LayoutRects`/дерево (меню-бар, статус), прячется/показывается в `main_window.rs` по полю `layout.chrome_mode()` (точное имя доступного геттера — из ядра).
- **Приёмка:** проходит ручной сценарий: сузить колонку → уходит в рейку → открыть flyout → клик снаружи закрывает → потянуть панель из flyout в плавающее окно → Ctrl+Z.

### Этап 15. Workspace и уборка

- **Файл:** `dock-gpui/src/dock/workspace.rs` (NEW).
- **Workspace:** включить feature `serde` у `dock-core`; `WorkspaceFile` / `WorkspaceLayout` (из справочника). Путь файла — каталог данных приложения (`dirs`/`directories` крейт). Загрузка при старте; если файл повреждён или `validate` не проходит — уведомление и `create_initial_layout`. Сохранение: по закрытию окна + дебаунс после изменения `new_version`. Запись — во временный файл + `rename` (атомарно).
- **Важно:** история Undo и «последнее место панели» в workspace не попадают (справочник); значит, после перезапуска «повторное открытие на прежнее место» даст место по умолчанию профиля.
- **Уборка:**
  1. Удалить всё помеченное `DELETE` в разделе 3.
  2. `cargo clippy --all-targets -- -D warnings` без предупреждений.
  3. Вынести `cfg(windows)` только на `main.rs`/платформенные вызовы; модули `dock/*`, `drop_commit`, `resize_session` (без GPUI) — с тестами на Linux-CI.
  4. `eprintln!` → `tracing`; `Result<_, String>` → `thiserror`.
  5. README `docs/architecture.md` — одна страница с диаграммой слоёв и таблицей «жест → команда» из раздела 9.

## 9. Карта покрытия: каждая команда ядра → где и как вызывается

Проверочная таблица: когда все строки закрыты — работа по справочнику завершена. Колонка «Этап» — когда команда подключается.

| Команда `DockCommand` | Жест / источник в UI | Компонент / модуль | Этап | В Undo? |
| --- | --- | --- | --- | --- |
| `OpenPanel` | меню Dockers | `menus/dockers_menu.rs` | 8, 13 | да |
| `ClosePanel` | ☰ → «Закрыть», Dockers | `menus/group_menu.rs` | 8, 13 | да |
| `CloseGroup` | ☰ → «Закрыть группу» | `menus/group_menu.rs` | 13 | да |
| `ActivateTab` | click по вкладке, выбор в меню ⋯ | `PanelTab`, `overflow_menu.rs` | 8, 13 | да (уточнить в ядре) |
| `MovePanel` | drag вкладки: перестановка или перенос в dock | `drop_commit.rs` / Core resolver | 11 | да |
| `MovePanelFromFlyout` | drag панели из flyout на допустимую цель dock | `drop_commit.rs` / Core resolver | 11, 14 | да |
| `FloatPanelFromFlyout` | drag панели из flyout в свободную область | `drop_commit.rs` / Core resolver | 11, 14 | да |
| `MoveGroup` | drag группы на `ColumnSlot` / `RailSlot` | `drop_commit.rs` / Core resolver | 11 | да |
| `MoveGroupToMainColumn` | drag группы на `MainColumn` | `drop_commit.rs` / Core resolver | 11 | да |
| `MergeGroup` | drag группы за grip в `GroupTabs` другой группы | `drop_commit.rs` / Core resolver | 11 | да |
| `SplitGroup` | drag группы за grip на `GroupEdge` | `drop_commit.rs` / Core resolver | 11 | да |
| `ResizeGroups` | сплиттер Y | `resize_session.rs` | 9 | да, одна запись на жест |
| `ResizeColumn` | внешняя ручка колонки | `resize_session.rs` | 9 | да |
| `ResizeColumnBoundary` | сплиттер X главной области | `resize_session.rs` | 9 | да |
| `ResizeSplitColumnBoundary` *(нет в справочнике)* | сплиттер X в плавающем окне | `resize_session.rs` | 9 | да |
| `ResetColumnRatios` | ☰ → «Вернуть пропорции», double-click по сплиттеру Y | `group_menu.rs`, `resize_session.rs` | 9, 13 | да |
| `ResetMainColumnRatios` | double-click по сплиттеру X | `resize_session.rs` | 9 | да |
| `RestoreGroupRatios` | только внутри ядра/истории; из UI **не вызывать** | — | — | — |
| `Float` | отрыв вкладки/группы на `FreeArea` по текущему Core resolver | `drop_commit.rs` / Core resolver | 11 | да |
| `FloatGroupAsColumn` | отдельная команда ядра; не формируется текущим drag resolver | `group_menu.rs` (если используется) | 13 | да |
| `FloatPanelAsGroup` | отдельная команда ядра; не формируется текущим drag resolver | UI-команда, если используется | — | да |
| `MoveFloating` | drag по основной области `FloatingTitle` без drop-target | `floating_policy.rs` / Core resolver | 11–12 | да, одна запись |
| `ResizeFloatingBounds` | ручки края/угла окна | `resize_session.rs` | 9 | да |
| `ResizeFloating` | резерв: только размер (программно) | `commands.rs` | 8 | да |
| `RaiseFloating` | mouse down по окну | `layers/floating.rs` | 12 | проверить в ядре |
| `DockFloating` | drag за отдельную захватную ручку `FloatingCapture` на цель dock | `drop_commit.rs` / Core resolver | 11–12 | да |
| `CollapseGroup` | двойной click по вкладке, ☰, кнопка в `FloatingTitle` | `PanelTab`, `group_menu.rs` | 8, 13 | да |
| `SetColumnMode` | `>>` / `<<`, ☰, пороги ширины | `column_mode_button.rs`, `rail_policy.rs` | 8, 14 | да |
| `OpenFlyout` | click по `RailEntry` | `RailEntry` | 14 | **нет** (временное) |
| `CloseFlyout` | × в flyout | `flyout_header.rs` | 14 | нет |
| `DismissFlyout` | клик снаружи, Esc | `layers/flyout.rs`, `keyboard.rs` | 14 | нет |
| `SetFlyoutAutoCollapse` | пункт меню | `group_menu.rs` | 14 | нет |
| `ResizeFlyout` | ручка края flyout | `resize_session.rs` | 14 | нет |
| `SetChrome` | Tab / Shift+Tab | `keyboard.rs` | 14 | проверить в ядре |

Не входят в `DockCommand`, но нужны приложению: `UndoRedoHistory::{apply, undo, redo}` (этапы 7, 14), `compute_rects` (этап 3), `compute_drop_zones` + `DragSession` (этапы 10–11), `Layout::validate` (тесты и workspace), `WorkspaceFile` (этап 15).

## 10. Замечания к `render.rs`, которые нужно учесть при переносе

`render.rs` — хорошая основа, но в нём есть места, которые лучше исправить во время разрезания, а не тащить в новые файлы. Всё чтением кода, без сборки.

| № | Место в `render.rs` | Проблема | Где чиним |
| --- | --- | --- | --- |
| 1 | цикл «`floating_resize_edges`» после цикла окон | Все ручки ресайза лежат поверх **всех** окон, в том числе перекрытых верхними | этап 6 (`layers/floating.rs`) |
| 2 | `tree_elements` — 11 аргументов, «too many arguments», каждый раз протаскиваются 7 дженериков | читаемость, невозможно тестировать | этапы 5–6 (`DockHost`, `layers/tree.rs`) |
| 3 | `localize_tree_rects` клонирует все `LayoutRects` на каждое плавающее окно и на каждую scroll-колонку в каждом кадре | лишние аллокации | в слоях рендерить в координатах окна «в лету»: передавать `origin` в компонент и вычитать при рисовании (`RectBox::at(rect, origin)`) |
| 4 | `panel_node_ids` — полный обход всех узлов в каждом вызове `tree_elements`; `subtree_nodes` и `rail_groups` считаются по несколько раз | линейный обход × число областей | кэш в `FrameGeometry` (этап 3) |
| 5 | `rects.groups.iter().any(...)` внутри цикла плавающих окон в `normalize_docked_group_geometry` | квадратичный поиск | индекс `HashMap<NodeId, usize>` внутри нормализатора |
| 6 | `on_mouse_down` на `PanelTab` начинает жест сразу, а `on_click` на том же элементе активирует вкладку | без порога «просто клик» создаёт ложные drag-сессии | порог `DRAG_SLOP_PX` в этапе 10 |
| 7 | `on_click` + `click_count() >= 2` → `CollapseGroup`: первый click сначала активирует вкладку, второй сворачивает | нормально, но нужен тест: не должно быть двух `ActivateTab` подряд в истории | этап 8 |
| 8 | `format!("dock-tab-{:?}-{:?}", ..)` на каждый элемент каждый кадр | аллокации строк | составной `ElementId` |
| 9 | `supported_splitter_handle` для `Axis::X` разрешает только корень главной области | в плавающих композициях с двумя колонками разделитель не тянется (в старом `main_window.rs` тянулся) | этап 9 |
| 10 | заголовок плавающего окна — текст первой панели, без кнопок | нельзя закрыть окно | этапы 4, 8, 12 (`FloatingTitle`) |
| 11 | `DragOverlay` всём рисует только зоны и маркер 10×10 | нет «призрака» под курсором, нет приглушения источника | этап 10 |
| 12 | `tab_overflows` + расчёт ширины вкладок «поровну» в `normalize_docked_group_geometry` | равная ширина всех видимых вкладок — осознанный выбор для Photoshop-стиля; оставить, но покрыть тестом на 1, 2, 5 вкладок | этап 3 |
| 13 | `focusable()` без явного `tab_index`/порядка обхода | обход клавиатурой будет в порядке отрисовки, а не логическом | после этапа 14 |

**Порядок правок не важен**: каждую правку делать в том этапе, который указан в последней колонке, и отмечать в коммите.

## 11. Что нужно уточнить в ядре и что мне прислать

До этапа 10 (drag and drop) точные сигнатуры неизвестны, а без них нельзя написать `drag_session.rs` и `drop_commit.rs`. Чтобы я довёл такие файлы до рабочего кода (а не гадал), пришлите из `dock-core`:

1. **Файл с `DragSession`, `DragPayload`, `DragHandle`, `DragPreview`, `DragDropRequest`, `DragDropAction`** — важнее всего.
2. **Файл с `compute_drop_zones`, `DropZone`, `DropZoneRect`, `DropTarget`.**
3. **Файл с `UndoRedoHistory`** (нужны публичные методы и поведение на временных командах).
4. **`LayoutRects` и сопутствующие структуры** (`FloatingGroupRect`, `ResizeEdge`, записи flyout, overflow).
5. **Публичные методы `Layout`** (особенно `flyout()`, `column_display_order`, `floating_groups_in_paint_order`, `main_area`, `dock_area`, доступ к chrome-режиму).
6. `dock-gpui/src/lib.rs` и `ui::Theme` (цвета и метрики) — чтобы компоненты были собраны на реальных полях.

### Вопросы к ядру (ответы меняют реализацию)

| № | Вопрос | Что меняется в плане |
| --- | --- | --- |
| 1 | Пишет ли `UndoRedoHistory::apply` в историю временные команды (`OpenFlyout`, `ResizeFlyout`, `RaiseFloating`, `SetChrome`)? | этапы 7, 12, 14: один вход `apply` или два (`apply` / `apply_transient`) |
| 2 | Есть ли пакетная запись (несколько команд = один шаг Undo)? | этапы 8, 11, 14 (закрытие окна, ресайз + смена режима) |
| 3 | `DragDropAction` — это готовые `DockCommand` или намерение, которое нужно развернуть (таблица этапа 11)? | объём `drop_commit.rs` |
| 4 | Какие реальные варианты `DragPayload`? Есть ли «вся колонка/область»? (`render.rs` знает `Panel`, `Group`, `FloatingGroup`; старый `main_window.rs` умел тянуть всю колонку за первый заголовок — в новом дизайне этого нет) | решение: теряем функцию или просим `DragPayload::Column` |
| 5 | `compute_drop_zones` работает на сырых `LayoutRects` или принимает нормализованные? | этап 3: если сырые, зоны будут смещены относительно нарисованного на высоту заголовка — нужна вторая нормализация зон или правка профиля |
| 6 | Есть ли `DockCommand::ResizeSplitColumnBoundary` и другие команды, не входящие в справочник? `DockCommand: Clone`? | этапы 0, 7, 9 |
| 7 | Работает ли в ядре кламп позиции `MoveFloating` и минимума `ResizeFloatingBounds`? Что в `outcome.actual_value`? | этапы 9, 12 |
| 8 | Знак и смысл `delta` у `ResizeColumnBoundary` для левой и правой стороны | этап 9 (таблица «как считаем») |
| 9 | Даёт ли ядро готовый `available_height` для колонки, или считать его нужно в приложении | этап 9 |
| 10 | Считается ли `ActivateTab` за постоянное изменение для Undo? (в Photoshop переключение вкладки не отменяется) | этап 8 |

### Для чего такой порядок этапов

Первые шесть этапов — это только картинка, и их можно делать **сейчас**, без ответов на вопросы выше. Этапы 7–9 требуют ответов № 1, 2, 6, 8, 9. Этапы 10–12 — № 3, 4, 5, 7. Вот почему я бы присылал файлы ядра к тому моменту, когда вы закончите этап 6.

## 12. Тема: цвета и метрики

Правило: компоненты берут цвета только из `ThemeColors`, размеры — только из `ThemeMetrics` (без чисел `28.0`, `4.0`, `30.0` и без `rgb(0x…)` внутри компонентов). Выполняется на этапе 4.

### Цвета

Уже используются: `window_background`, `canvas_background`, `dock_background`, `group_background`, `border`, `splitter`, `primary_text`, `muted_text`, `active_tab`, `focus`, `drop_target`, `toolbar_background`. `hover` пока не используется — подключить в вкладках, кнопках, пунктах меню/рейки и сплиттере.

| Новое поле | Зачем | Этап |
| --- | --- | --- |
| `scrollbar_thumb` (+ опц. `scrollbar_track`) | полоса прокрутки колонок с `vertical_overflow` | 6 |
| `shadow` | тень меню, flyout и «призрака» при перетаскивании | 10, 13 |
| `danger` | наведение на ×, опасные пункты меню | 8, 13 |
| `disabled_text` | пункты меню с `enabled = false` | 13 |

Опционально: `inactive_title` (заголовок неактивного плавающего окна), `pressed`. `focus` и `drop_target` должны заметно отличаться: активная зона стыковки рисуется границей `focus` поверх заливки `drop_target`.

### Метрики

Разбор текущего `ThemeMetrics` по полям:

| Поле | Статус | Действие |
| --- | --- | --- |
| `column_header_height` | дублирует `DockMetrics::header_height` | `render.rs` читает оба (заголовок окна и flyout — из темы, геометрия — из `DockMetrics`). Выбрать один источник: компоненты получают высоту из `FrameGeometry` |
| `tab_strip_height` | ок | та же проверка на совпадение с `DockMetrics::tab_strip_height` |
| `group_control_button_width` | ок, но не используется | в `render.rs` вместо константы `GROUP_MENU_BUTTON_WIDTH = 28.0` |
| `tab_drag_zone_width` | **устарело** (старый `group_drag_zone`) | удалить после этапа 6 |
| `column_tab_width` | **устарело** («простая колонка», в `render.rs` вкладки делят ширину поровну) | но если ядро решает переполнение вкладок по ширине вкладки, переименовать в `tab_min_width` (проверить в `DockMetrics`) |
| `button_size`, `button_gap`, `content_padding` | не видно, где используются | использовать в `IconButton` и `PanelSurface`, иначе удалить |
| `splitter_thickness`, `splitter_hit_zone` | ок | `SplitterHandle`: полоса = thickness, зона клика = hit\_zone (в `render.rs` сейчас заливается вся hit-зона — полоса больше задуманного) |
| `edge_drop_zone_width` и `drop_edge_zone` | **два поля на одно** | оставить `drop_edge_zone`, остальное удалить |
| `drop_indicator_thickness`, `drop_outline_thickness` | ок, но не используются | этап 10: линия вставки и разделения — толщиной `drop_indicator_thickness`, контур целевой группы/окна — `drop_outline_thickness`; заменяют маркер 10×10 |
| `drag_start_threshold` | ок, но не используется | **это и есть порог `DRAG_SLOP_PX` из этапа 10** — не заводить новую константу |
| `default_column_width`, `min_column_width`, `rail_width`, `min_group_height`, `collapsed_group_height`, `max_columns`, `default_column_split_ratio` | работают на уровне `DockMetrics` | не читать из UI напрямую — только через `dock_metrics_from_theme` |
| `min_document_width` | в ядре называется `min_canvas_width` | проверить, что `dock_metrics_from_theme` его мапит |

**Что добавить в `ThemeMetrics`:**

| Новое поле | Зачем | Этап |
| --- | --- | --- |
| `group_grip_height` (4.0) | сейчас константа `GROUP_DRAG_GRIP_HEIGHT` в `render.rs` и в нормализаторе | 3 |
| `menu_item_height` (30.0), `menu_min_width`, `menu_padding` | в старом коде `30.0` было магическим числом | 13 |
| `scrollbar_width` | полоса прокрутки колонки | 6 |
| `border_width` (1.0), `focus_ring_width` (2.0) | в `render.rs` зашиты `border_1()` и `border_2()` | 4 |
| `corner_radius` | единый радиус меню, flyout, призрака (можно 0) | 4 |
| `icon_size`, `tab_font_size`, `title_font_size` | в теме сейчас нет **типографики**: размеры текста остаются по умолчанию GPUI | 4 |
| `ghost_offset` | смещение «призрака» от курсора | 10 |
| `rail_collapse_width`, `rail_expand_width` | пороги автопереключения режима колонки с гистерезисом (два разных порога) | 14 |

### Как метрики доходят до ядра (`dock_metrics_from_theme`)

Уточнение по фактической функции маппинга; оно исправляет две строки таблицы выше.

- `column_header_height` → `header_height` мапится напрямую, расхождения нет. Высоту в UI всё равно читать одним способом (из `FrameGeometry`/`DockMetrics`).
- `drop_edge_zone` → `edge_drop_width`, а `edge_drop_zone_width` не мапится никуда: **удаляем**.
- `drag_start_threshold` → `drag_threshold` в ядре. Если `DragSession` сам применяет этот порог, не дублируйте его в `drag_session.rs`; иначе используйте то же значение.
- Не доходят до ядра: `default_column_split_ratio`, `group_control_button_width`, `drop_indicator_thickness`, `drop_outline_thickness`, `tab_drag_zone_width`, `column_tab_width`, `button_*`, `content_padding`. Для `default_column_split_ratio` проверить, есть ли аналог в `DockMetrics`: если есть, поле теряется молча.
- `..DockMetrics::default()` скрывает поля, которых тема не задаёт: `tab_overflow_button_width` (его читает `normalize_docked_group_geometry`), размеры flyout, минимум плавающего окна, толщина ручек resize. Выписать их все и решить по каждому: выставить из темы или осознанно оставить по умолчанию.
- **Резерв под кнопки в полосе вкладок.** UI резервирует ширину под ☰ и ⋯ уже после расчёта ядра (`normalize_docked_group_geometry`), а ядро решает, сколько вкладок помещается, считая всю ширину доступной. Из-за этого переполнение срабатывает позже, чем нужно, и вкладки сжимаются. Если в `DockMetrics` есть поле под резерв в конце полосы, передать туда `group_control_button_width`; если нет, попросить добавить.
- **Защита от дрейфа:** добавить тест, который деструктурирует `ThemeMetrics` без `..`. При добавлении нового поля тест перестанет компилироваться, и поле придётся явно отнести к «мапится в ядро» или «только UI».

**Уточнение после просмотра `DockMetrics` (15 полей):**

- Тема не задаёт только два поля ядра: `min_tab_width` и `tab_overflow_button_width`. Вывести их в `ThemeMetrics`. Устаревшее `column_tab_width` переименовать в `tab_min_width` и передать в `min_tab_width`.
- Размеры flyout и минимум плавающего окна в `DockMetrics` нет: они берутся из `PanelConstraints` панелей (реестр из этапа 1). Не добавлять их в тему.
- `default_column_split_ratio` в ядре нет — поле темы мёртвое. Удалить или применить в приложении при сборке начальной раскладки.
- Резерв под ☰ в конце полосы вкладок в `DockMetrics` **нет**. Ядро решает переполнение по `min_tab_width`, не зная об этих 28 px, а UI вычитает их потом. Просим у ядра поле `tab_strip_trailing_reserved`; пока — увеличить `min_tab_width` на запас и покрыть тестом «после `normalize_docked_group_geometry` ширина каждой видимой вкладки ≥ `min_tab_width`» на 2, 4, 8 вкладках и разных ширинах колонки.
- `DockMetrics` имеет `serde(default)`. Если workspace-файл сохраняет метрики, при загрузке игнорировать их и брать из темы, иначе смена темы не влияет на старые раскладки (проверить состав `WorkspaceLayout`).
- Инварианты для `ThemeMetrics::validated()` и теста: `min_column_width ≤ default_column_width`; `rail_width < min_column_width`; `splitter_hit_zone ≥ splitter_thickness`; `header_height` и `tab_strip_height` > 0; `min_tab_width > 0`; `min_canvas_width > 0`; `max_columns ≥ 1`.

**Проверка входных значений:** в старом коде повсюду `.max(1.0)`. Вынести в `ThemeMetrics::validated(self) -> Self` (клампит отрицательные/NaN) и вызывать один раз при загрузке темы.

## 13. Тесты и критерии приёмки

### Уровни тестов

1. **Чистые юнит-тесты без GPUI** (быстрые, идут на Linux-CI): `geometry`, `tree_query`, `labels`, `chrome_geometry`, `frame`, `drop_commit`, маппинг `ResizeHandle` → `DockCommand`, `rail_policy` (гистерезис), `floating_policy` (`initial_float_bounds`).
2. **Тесты инвариантов ядра:** после любой команды из таблицы раздела 9 — `layout.validate(&metrics) == Ok(())` и `compute_rects` не падает. Предлагаю один параметризованный тест на все варианты.
3. **Два теста на Undo:** «команда → undo → дерево равно исходному» и «жест ресайза = один шаг истории».
4. **Визуальные:** `examples/components_gallery.rs` для компонентов; скриншоты трёх эталонных раскладок до и после этапа 6.

### Критерии «готово» для любого этапа

- [ ] `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings` без ошибок
- [ ] новый код не вызывает `layout.apply` вне `DockController`
- [ ] компоненты не импортируют `Layout`/`Node`
- [ ] нет магических чисел и `rgb(0x…)` в компонентах
- [ ] нет новых `Layout::clone()` в пути рендера (`grep`)
- [ ] границы файлов отражают цельные подсистемы; ~300 строк — ориентир, не причина дробить каждую функцию; production-модули обычно ~150–400 строк, без файлов на одну функцию по умолчанию
- [ ] функции ориентировочно ≤ ~60 строк; при >5 аргументах используется структура параметров/context

### Ручной сценарий приёмки всего проекта (после этапа 15)

1. Открыть приложение: три группы в двух колонках как раньше.
2. Ресайз всех типов ручек → Ctrl+Z → Ctrl+Shift+Z.
3. Перетащить вкладку в другую группу, на край группы, в новую колонку, в свободную область.
4. Перетащить группу (за grip) во все типы целей.
5. Открыть два плавающих окна, перекрыть, поднять нижнее кликом, изменить размер каждого за края и углы, закрыть.
6. Сузить колонку → рейка → flyout → клик снаружи → перетащить из flyout в плавающее окно.
7. Закрыть и открыть все панели через Dockers — панели возвращаются на прежние места.
8. Esc в середине каждого жеста — всё как было; увод курсора за окно и отпускание вне окна не оставляют «залипшей» сессии.
9. Перезапуск: раскладка восстанавливается; повреждённый workspace-файл даёт раскладку по умолчанию, а не краш.

### Бюджет производительности

Время кадра (`compute_rects` + нормализация + сборка элементов) на раскладке из трёх групп — в пределах однозначных миллисекунд; целевое число уточнить замером до рефакторинга (этап 0) и не допускать роста более чем на 10 %.

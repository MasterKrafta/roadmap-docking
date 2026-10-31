# ТЗ: перевод dock-gpui на нативные плавающие окна (tear-off)
Oct 31, 2026 · @master
Плавающая группа панелей должна стать отдельным нативным окном Windows, а не `div` с `position: absolute` внутри `DockHost`. Окно можно вынести за пределы главного окна и на другой монитор; панель отрывается от дока при перетаскивании заголовка и возвращается в док отпусканием над ним. Для запуска главного окна и стартовых floating-окон используется проверенная последовательность из `crates/dock-experiment/src/lib.rs`, а не порядок создания окон из `tearoff/main.rs`.
## 1. Цель и границы
**Входит:** Windows (окна и startup lifecycle по образцу `crates/dock-experiment/src/lib.rs`; нативные операции Win32 — через платформенный слой), мультимонитор с разным DPI, отрыв панели/группы/всей плавающей группы, возврат в док, drop на другое плавающее окно, нативный resize, сохранение позиций в workspace.
**Не входит:** macOS/Linux (оставляем трейт с заглушкой), перепроектирование дерева в `dock-core`, новые типы панелей.
**Startup и жизненный цикл окон — по `crates/dock-experiment/src/lib.rs`:**
- главное окно создаётся с `show: false`, `focus: false`; оно ещё не появляется на экране;
- после создания `MainWindow` и получения HWND главного окна стартовые плавающие окна открываются из `MainWindow` через `cx.open_window`, каждое с `WindowKind::PopUp`, `titlebar: None`, `show: false`, `focus: false`;
- корневой `FloatingHost` создаётся в builder каждого окна. В закреплённой GPUI-ревизии `9cd14daad768fbfaeeac8789f515f9a5f67669a6` `App::open_window` вызывает `window.draw(cx)` до возврата из метода — первое GPUI-кадрирование окна выполняется ещё до его показа ([`App::open_window`](https://github.com/zed-industries/zed/blob/9cd14daad768fbfaeeac8789f515f9a5f67669a6/crates/gpui/src/app.rs#L1535));
- пока popup скрыт, ему назначается `owner` главного окна и снимается topmost. Это важно: Win32-бэкенд закреплённой GPUI-ревизии создаёт `WindowKind::PopUp` со стилями `WS_EX_TOOLWINDOW | WS_EX_TOPMOST` ([создание Win32-окна](https://github.com/zed-industries/zed/blob/9cd14daad768fbfaeeac8789f515f9a5f67669a6/crates/gpui_windows/src/window.rs#L492)); в `dock-experiment` это обрабатывается в `configure_floating_window` через `set_owner` и `remove_topmost`;
- только после того, как главное окно и все стартовые popup-окна созданы, получили root view и прошли первый draw, выполняется показ: сначала активируется главное окно, затем показываются и активируются плавающие окна (`show_floating_panels`);
- сохраняются `WindowHandle` плавающих окон; окна скрываются, когда их панель пристыкована, и повторно показываются при возврате в floating. Для каждой плавающей композиции видимым остаётся один host;
- тот же путь `cx.open_window` применяется к окнам, которые создаются позже при работе приложения: сначала скрытое создание и первый draw, затем настройка native owner и показ.

**Нативное перемещение и resize:**
- перетаскивание опросом `GetCursorPos` + `GetAsyncKeyState(VK_LBUTTON)` каждые 4 мс (`timeBeginPeriod(1)`), порог отрыва 2 px;
- перемещение через `SetWindowPos` с `SWP_NOACTIVATE`; при обычном drag окно не должно самопроизвольно перехватывать фокус;
- попадание в док проверяется в экранных координатах, DPI берётся в момент проверки (`GetDpiForWindow`);
- нативный resize через `WS_THICKFRAME`; верхние 8 px заголовка отданы рамке и не тянут окно;
- принудительная перерисовка во время модального resize-цикла (`WM_GPUI_FORCE_UPDATE_WINDOW` по таймеру); это отдельная задача resize и не заменяет первый draw, который делает `cx.open_window`;
- единое состояние в `Entity`, главное и плавающие окна подписаны через `cx.observe`.
**Оговорка:** `dock-core` изначально не попал в архив; раздел 5 теперь сверен с его исходниками.
## 2. Было → стало
Меняется всё, что связано с координатами, вводом и жизненным циклом окна; дерево и команды `dock-core` остаются.
| Аспект | Сейчас (dock-gpui) | Должно быть |
| --- | --- | --- |
| Рендер floating | `FloatingWindowView::render` рисует `div().absolute()` внутри `DockHost`, клип по окну | Корневой view отдельного OS-окна (`FloatingHost`), содержимое в локальных координатах (0,0) |
| Старт окон | Главное окно может стать видимым раньше, чем floating-окна подготовлены | Главное окно и стартовые `PopUp` создаются скрытыми; `cx.open_window` выполняет первый draw каждого окна до возврата; все окна показываются только после подготовки |
| Координаты | Всё в viewport главного окна; Core при расчёте вписывает floating во viewport | Границы окна в экранных координатах, без вписывания во viewport главного окна |
| Ввод | Один `register_pointer_events` на `DockHost`, `capture_pointer` на root hitbox | Свой набор обработчиков у каждого окна; перетаскивание ведёт опрос курсора (экранные координаты), а не capture |
| Перенос окна | `DragHandle::FloatingTitle`: сдвиг через Core, в drop-цели не попадает | Окно двигает `SetWindowPos`; та же перетаскивающая сессия ищет drop-цель под курсором |
| Возврат в док | Через drop-зоны в пределах главного окна | Курсор над доком/другим окном: подсветка и слияние при отпускании (как `highlight` в `main.rs`) |
| Отрыв от дока | Drop на `DropTarget::FreeArea` при отпускании | Окно появляется сразу после порога 2 px и едет за курсором; в Core создаётся floating group |
| Resize | 8 hit-rect от Core, `ResizeFloatingBounds` на каждый кадр | `WS_THICKFRAME`; размер из OS возвращается в Core событием bounds-changed |
| Z-order | `z_order` в Core, `RaiseFloating` при клике | Решает OS (owner = главное окно); `z_order` в Core опционален |
| Drag-ghost | `DragGhost` + `DropOverlay` в слое `DockHost` | Ghost не нужен: за курсором едет само окно. `DropOverlay` рисуется в окне-цели |
| Фокус | `on_focus_out` отменяет жест | Отмена по фокусу убирается: фокус законно уходит в floating-окно |
| DPI | Один `scale_factor` | Свой масштаб у каждого окна, пересчёт при смене монитора |
| Persistence | `WorkspaceFloatingGroup.bounds` во viewport | Экранная позиция + размер; при загрузке проверка, что окно попадает на существующий монитор |
## 3. Архитектура
Основной принцип: **Core остаётся единственным источником правды, окна — его проекция.** Окна создаются и показываются по startup-последовательности из `crates/dock-experiment/src/lib.rs`; флаг видимости в UI-адаптере отражает состояние native window, а существование плавающей композиции определяется `FloatingGroup` в общем `Layout`.
```text
DockController (Entity, один) — Layout, history, DragSession
      ▲ observe / dispatch
FloatingWindowManager (Entity, новый)
   ├─ reconcile(): Layout.floating_groups ↔ OS-окна
   ├─ WindowRegistry: `WindowHandle` для панелей/композиций; скрытое создание и повторное использование
   ├─ TearOffDriver: опрос курсора 4 мс, порог 2 px
   └─ NativeWindowOps (trait) → crate dock-native (unsafe, Win32)
MainWindow:   DockHost      → док слева/справа, canvas, DropOverlay
FloatingWin:  FloatingHost  → одна floating group, шапка, DropOverlay
```
**Решения:**
1. **Один контроллер на все окна.** `Entity<DockController>` общий; каждое окно — свой корневой view, подписанный через `cx.observe` (так же, как `MainView` на `DockState`).
2. **Координаты Core — оконные, логические.** Единое экранное пространство не подходит: при разном DPI мониторов логические пиксели не складываются. Экранные физические px используются только в `TearOffDriver`; перед обращением к Core они переводятся в локальные логические координаты окна под курсором (scale этого окна, как `scale_of(hwnd)` в `main.rs`).
3. **Отрыв = реальная мутация Core.** При пороге 2 px вызывается команда Float, в которую сейчас превращается drop на `FreeArea`; жест продолжается как `DragPayload::FloatingGroup`. Всё внутри одной history-транзакции (`begin_transaction`, как в resize): Esc откатывает отрыв, отпускание даёт один шаг undo.
4. **Перетаскивание floating-окна тоже ищет drop-цель.** Особый случай `DragHandle::FloatingTitle` («двигает себя и в цели не попадает») не используется: тянем с FloatingCapture (раздел 5, п. 3).
5. **Стартовые окна подготавливаются до первого показа, но не создаются раньше главного окна.** Сначала главное окно открывается с `show: false`; затем из его `MainWindow` создаются все стартовые floating-окна через `cx.open_window` с `show: false`. После каждого вызова GPUI уже выполнил первый draw root view. Затем каждому popup назначаются `owner` и корректный Z-order, и только после подготовки всех окон показываются главное окно и панели. Это повторяет `run_windows` → `open_startup_panels` → `show_floating_panels` из `dock-experiment`; схема `create_floating` до открытия главного окна из `tearoff/main.rs` для старта не используется.
6. **Одна панель — один `AnyView`** (`panel_views` в контроллере). Не повторять упрощение из `tearoff/main.rs`, где для главного и floating создаются разные `Entity<PanelView>`. Перенос одного `Entity`/`AnyView` уже проверен в `tearoff/any_view_transfer.rs`: состояние сохраняется, вид рендерится только в одном окне за раз.
## 4. Изменения в dock-gpui по файлам
| Файл | Что сделать |
| --- | --- |
| `lib.rs` | `#![forbid(unsafe_code)]` остаётся; весь `unsafe` уходит в отдельный крейт (см. раздел 6). Экспортировать `FloatingWindowManager`, `FloatingHost`, убрать `FloatingWindowView` из публичного API. |
| `components/floating.rs` | Переписать в `FloatingHost: Render`. Убрать `.absolute()`, `left/top/w/h`, `shadow_lg`, `origin`-смещения; корень = `size_full()`. Вложенные `ColumnView/GroupView/PanelHost/SplitterView` вызывать с `origin = ZERO`. Убрать 8 resize-хэндлов (`resize_edges`) и свою кнопку × — закрытие остаётся, но как команда `CloseFloatingGroup`. Верхние 8 px шапки не тянут окно (`RESIZE_ZONE`). |
| `dock_host.rs` | Перестать собирать `floating_*` карты и рисовать floating-слой. Слои остаются: canvas, док, flyout, `DropOverlay`, context menu. Добавить в `DropOverlay` подсветку, когда курсор над доком (`highlight` из `main.rs`). Убрать `DragGhost` для отрыва. |
| `controller.rs`, `layout_cache.rs` | Геометрия считается по окну: `geometry_for(window, viewport)`. Сейчас один `viewport` и один `rects_key`; нужен кэш на окно (ключ: `WindowId`). `set_viewport` для floating-окна не должен вызывать `cancel_pointer_interaction`, иначе смена DPI при перетаскивании сорвёт жест. |
| `input/pointer.rs` | `register_pointer_events` вызывается и из `FloatingHost` со своим `window_origin`. Удалить `FloatingTitleCandidate` c core-point и `RaiseFloating*`; новый intent `FloatingTitleDragCandidate` запускает `TearOffDriver`. Не вызывать `capture_pointer` для drag: курсор покидает окно. |
| `input/drag.rs` | Тянуть окно с FloatingCapture, не FloatingTitle. Убрать ветки `handle == FloatingTitle` в `update_drag`, `finish_drag_at`. `DragSession` ведётся в экранных координатах (физ. px, делённые на scale исходного окна); `resolve_drop_target` получает точку в локальных координатах окна-цели и `WindowId`. `FreeArea` больше не конечная цель: отрыв происходит на пороге. |
| `input/resize.rs` | Ветку `FloatingResizeEdge` удалить. Остаются splitter и flyout. |
| `controller/commands.rs` | Из `is_resize_command` убрать `ResizeFloatingBounds`; не добавлять новую команду `SetFloatingBounds` — существующий ResizeFloatingBounds вызывается из OS-события в одной транзакции (раздел 5, п. 7). |
| `components/drop_overlay.rs` | Зоны `FloatingCenter/Top/Bottom` рисуются в окне-цели в его локальных координатах; `floating_for_target` по `z_order` не нужен. |
| `persistence.rs` | Загрузка: после `replace_layout` вызвать `reconcile()`. Сохранение: bounds в экранных координатах (раздел 5). |
| новый `floating_manager.rs` | `FloatingWindowManager`: `WindowRegistry`, `reconcile()`, `TearOffDriver`, карта `FloatingGroupId → WindowHandle + HWND` и связь `PanelId → WindowHandle`. При старте открывает скрытые popup-окна после создания скрытого MainWindow; `open_window` выполняет первый draw до возврата. Подписан на `DockEvent::LayoutChanged`. |
| новый крейт `dock-native` | Win32-операции из `tearoff_win` + трейт `NativeWindowOps` + заглушка для не-Windows. |
**Зависимости:** в `Cargo.toml` `gpui_platform` подключён только под `cfg(windows)`. Для `dock-native` нужны `windows` и `raw-window-handle` (как в `main.rs`).
## 5. Требования к dock-core
Сверено с исходниками `dock-core`: большая часть нужного уже есть. Нужны только небольшие правки Core (пункты 1, 4, 5 ниже), новых команд добавлять не надо.
### Уже есть и подходит без изменений
| Что нужно | Что есть в Core |
| --- | --- |
| Отрыв панели или группы | `DockCommand::Float { node, bounds }`; в профиле Photoshop одиночная `Panel` оборачивается в `Tabs` |
| Возврат в док или в другое окно | `DockCommand::DockFloating { group, target }`; его выдаёт `resolve_floating_capture_drag` для `DragPayload::FloatingGroup` + `DragHandle::FloatingCapture` |
| Позиция и размер окна | `MoveFloating`, `ResizeFloatingBounds` (origin + size атомарно, минимум из `minimum_floating_size`). Viewport не участвует, в `actual_value` возвращается `CommandValue::FloatingBounds` — его можно подтянуть к OS-окну |
| Закрытие | `CloseFloatingGroup` |
| Отмена жеста одним шагом | `UndoRedoHistory::begin/commit/cancel_transaction` |
| Drop-зоны плавающего окна | `FloatingCenter/Top/Bottom`, `TabInsertion`, `Left/Right`. Для источника — плавающего корня зона `FreeArea` не выдаётся (`!source_is_floating_root`) |
| Хранение bounds | `FloatingGroup.bounds` — просто конечный `Rect`; Core не знает DPI и мониторов, `MoveFloating` не клампит |
### Что нужно поменять
1. **Геометрия floating выносится из `compute_rects` (правка Core).** Сейчас `compute_rects(viewport)` для каждой floating-группы вызывает `clamp_floating_bounds_with_cache(..., viewport, ...)`: размер урезается до клиентской области главного окна, а если минимум не влезает, группа молча пропадает из снимка (`omitting floating group`). Окно на другом мониторе или больше главного так не заработает. Нужно: (а) новый `Layout::compute_floating_rects(group, metrics) -> LayoutRects`: на основе уже существующих `layout_floating_root` и `push_resize_edges`, без клампа к viewport, с оригином (0,0) — локальные координаты окна; (б) `compute_rects` перестаёт эмитить floating-группы (отдельный `compute_main_rects` или флаг), чтобы главное окно не видело их в hit-test и drop-зонах.
2. **Hit-test и drop-зоны по окну — без правок алгоритмов.** `hit_test` и `prepare_drop_resolver` уже работают на переданном `&LayoutRects`. Для floating-окна передаём его собственный снимок из пункта 1, для главного — снимок без floating. На каждое окно свой `DropZoneResolver`; `WindowId` живёт в адаптере, не в Core. Резолверы готовятся один раз на пороге отрыва, в окно под курсором подставляется локальная точка.
3. **Тянуть окно нужно с `DragHandle::FloatingCapture`, не с `FloatingTitle` (без правок Core).** Для `FloatingTitle` `DragSession::set_drop_target` возвращает `NotDockingDrag`, а запрос при отпускании без цели — только сдвиг окна. Для tear-off окно должно уметь стыковаться, поэтому и свежеоторванное окно, и заголовок уже плавающего тянутся как `FloatingCapture`. Сам сдвиг окна делает OS (`SetWindowPos`), в Core он записывается командой `ResizeFloatingBounds`.
4. **Отпускание над пустым местом (желательная правка Core).** С `FloatingCapture` и `target = None` сессия уходит в `Cancelled { reason: NoValidTarget }`, запрос не выдаётся. Для tear-off это нормальный исход «остаться плавающим». Без правки адаптер должен отличать `NoValidTarget` от `Escape`/`FocusLost`/`Explicit`: при `NoValidTarget` — `commit_transaction`, в остальных случаях — `cancel_transaction`. Рекомендация: в Core для `FloatingCapture` выдавать запрос с `target: None`, а `resolve_floating_capture_drag` пусть возвращает `NoOp`. Это избавляет от неочевидной семантики «отмена = успех».
5. **`Float` должен возвращать id созданной группы (правка Core).** Сейчас `apply_float_command` вызывает `floating_groups.insert(...)` и ничего не возвращает, а для одиночной `Panel` корень нового окна — новый `Tabs`, а не переданный `node`. Адаптеру нужен `FloatingGroupId`, чтобы продолжить жест. Добавить вариант `CommandValue::FloatingGroup(FloatingGroupId)` в `actual_value` команды `Float` (временный обход — брать группу с максимальным `z_order`).
6. **Смена payload при отрыве — в адаптере.** `DragSession` фиксирует payload и handle при `pointer_down`, и `PanelTab`/`GroupGrip` нельзя превратить в `FloatingCapture`. Порядок на пороге: `begin_transaction` → `Float { node, bounds }` → `reset` текущей сессии (без `cancel_transaction`) → новая сессия `FloatingGroup { group }` + `FloatingCapture`. После `Float` версия `Layout` меняется, поэтому адаптер обновляет `drag.layout_version`, иначе `ensure_pointer_interaction_current` отменит жест.
7. **Сдвиг и resize от OS и undo.** `MoveFloating` и `ResizeFloatingBounds` — persistent-команды с inverse, каждый вызов дал бы отдельный шаг undo. Решение: адаптер открывает транзакцию на первом событии bounds-changed и коммитит, когда `left_down()` стал ложным. Новая команда `SetFloatingBounds` не нужна.
8. **Единицы bounds — зафиксировать соглашение (документация).** Core не интерпретирует DPI, поэтому код не меняется. Предлагается в комментарии `FloatingGroup` писать: `origin` — физические px виртуального рабочего стола, `size` — логические px (так как `minimum_floating_size` тоже логический). Это открытый вопрос к вам.
9. **Загрузка workspace.** `WorkspaceLoadOptions::visible_client_region` — один `Rect`, в него `clamp_floating_groups_for_workspace_load` зажимает все окна и при `InsufficientSpace` заваливает загрузку (`FloatingBoundsInsufficientSpace`). Для окон вне главного это неверно. Адаптер передаёт `None` и сам проверяет попадание в рабочие области мониторов (`monitor_work_areas`), сдвигая непопавшие окна. Проверить тестом, что при `None` кламп не выполняется.
10. **`RaiseFloating` и `z_order`.** Порядок окон теперь решает OS. `RaiseFloating` — persistent-команда, поэтому её не диспатчим при активации окна. `z_order` остаётся в модели и workspace как есть, на рендер не влияет.
## 6. Платформенный слой (Windows)
Низкоуровневые Win32-операции (часть которых сейчас находится в `mod tearoff_win` из `tearoff/main.rs`) выносятся в отдельный крейт `dock-native`. Это не переносит в `dock-gpui` startup-схему `tearoff/main.rs`: создание окон, первичный draw, настройка owner и порядок показа остаются по образцу `crates/dock-experiment/src/lib.rs` и выполняются через GPUI `cx.open_window`. Причина отдельного native-крейта: в `dock-gpui` стоит `#![forbid(unsafe_code)]`, а `forbid` нельзя ослабить через `#[allow]` внутри модуля. Также проверить `[workspace.lints]`.
**Трейт `NativeWindowOps`** (всё принимает `NativeId = isize`):
- `hwnd_of(window)`, `cursor_pos()`, `left_down()`, `key_down(Escape)`;
- `window_rect`, `client_origin`, `scale_of`, `move_window`, `resize_window`, `show`, `show_no_activate`, `activate`, `hide`;
- `set_owner`, `remove_topmost`, `enable_resize` (включая таймер перерисовки);
- `show` используется после первичного скрытого draw и настройки owner; `show_no_activate` — для сценариев перемещения без смены активного окна;

- **новое:** `window_from_point(screen_pt, exclude)` — какое из наших окон под курсором, без перетаскиваемого (обход по z-order через `GetWindow(GW_HWNDNEXT)` и `PtInRect` по списку наших HWND);
- **новое:** `monitor_work_areas()` для проверки позиции при загрузке;
- **новое:** `set_min_size`, если выбранный коммит gpui не даёт `window_min_size` в `WindowOptions`.
**Требования:**
- Все `unsafe` только здесь, с комментарием `// SAFETY:` у каждого блока.
- Для не-Windows — `StubOps`, который возвращает `Unsupported`; `FloatingWindowManager` тогда не работает, отрыв отключён (панели остаются в доке).
- `begin_high_res_timer()` вызывается один раз в `run()`, парный `timeEndPeriod` при выходе.
- Не добавлять отдельную задержку/таймер ради первичной отрисовки floating-окон: это обеспечивает `App::open_window` в закреплённой GPUI-ревизии. Таймер перерисовки нужен только для модального resize-цикла.
- Таймер перерисовки (`SetTimer`, id 2, 8 мс) ставится на каждое floating-окно и снимается (`KillTimer`) при закрытии из пула. Сейчас в `main.rs` он не снимается.
- `WM_GPUI_FORCE_UPDATE_WINDOW = WM_USER + 5` завязан на конкретный коммит `gpui_windows`. Зафиксировать версию gpui и добавить тест/проверку при её обновлении.
## 7. Сценарии, этапы, приёмка
### Сценарии (критерии приёмки)
| # | Действие | Ожидаемый результат |
| --- | --- | --- |
| 1 | Потянуть вкладку панели из дока на 2 px и больше | Появляется OS-окно под курсором, точка захвата сохраняется; в доке панель не рисуется |
| 2 | Вытащить окно за пределы главного и на другой монитор с другим DPI | Окно следует за курсором без рывков, жест не обрывается, содержимое перерисовывается в новом масштабе |
| 3 | Подвести окно к доку и отпустить | Подсветка зоны в доке, при отпускании панель встаёт в табы/колонку, окно скрывается |
| 4 | Отпустить на пустом месте | Окно остаётся плавающим, один шаг undo на весь жест |
| 5 | Esc во время перетаскивания | Панель возвращается на исходное место, история не изменилась |
| 6 | Перетащить уже плавающее окно за шапку | Тот же механизм, что в пункте 1; верхние 8 px шапки — resize, не перенос |
| 7 | Бросить панель на другое плавающее окно | Панель становится вкладкой или встаёт в колонку этого окна |
| 8 | Меняем размер окна за край | Рисуется плавно даже во время модального цикла, минимум по `PanelConstraints` |
| 9 | Свернуть/закрыть главное окно | Плавающие окна сворачиваются вместе с ним (owner), при закрытии завершается приложение |
| 10 | Сохранить и загрузить workspace | Окна восстанавливаются на тех же местах; если монитора больше нет — окно сдвигается в рабочую область основного |
| 11 | Текстовое поле в панели, затем перенос панели док↔floating | Состояние панели (текст, scroll) сохраняется |
### Этапы
1. **Платформенный слой.** Вынести Win32-операции из прототипов в `dock-native` (или согласованный платформенный crate), включая `set_owner`, `remove_topmost`, `show/hide`, перемещение, DPI, resize, `window_from_point`, `KillTimer`. Проверять его вместе с окном, созданным через GPUI `open_window`; startup-последовательность брать из `dock-experiment`, не из `tearoff/main.rs`.
2. **Контракт `dock-core`** (раздел 5, пункты 1, 4, 5, 9): геометрия floating вне compute\_rects, Float возвращает id, FloatingCapture без цели. Покрыть unit-тестами без UI.
3. **`FloatingHost` + `FloatingWindowManager`**: `reconcile()`, реестр `WindowHandle`, привязка owner. На старте: скрытый MainWindow → скрытые popup-окна через `cx.open_window` → первый draw каждого root view → `set_owner`/`remove_topmost` → показать главное окно и floating-окна. Рестор из workspace, без drag. Сценарии 9, 10.
4. **`TearOffDriver`**: опрос курсора, порог, отрыв, возврат, Esc, одна транзакция undo. Сценарии 1–7.
5. **Resize, DPI, min size, persistence.** Сценарии 2, 8, 10, 11.
6. **Чистка.** Удалить `FloatingWindowView`, `DragGhost` для отрыва, resize-хэндлы, тесты старого floating (`controller_tests.rs`, `pointer.rs`, `resize.rs`), обновить документацию слоёв в `DockHost`.
## 8. Риски и открытые вопросы
Перенос одного `AnyView` панели между окнами GPUI проверен прототипом `tearoff/any_view_transfer.rs` до этапа 3. В рамках эксперимента одна `Entity`/`AnyView` поочерёдно отображалась в главном и плавающем окне: `EntityId` не менялся, текст, счётчик, курсор и выделение сохранялись, ввод продолжался после переноса в обе стороны, фокус не требовал повторного клика. Пересоздавать представление с передачей состояния через отдельную `Entity` по результатам этого теста не требуется. Одновременный рендер одной панели в двух окнах и отдельная проверка IME-композиции не входили в эксперимент.
| Риск / вопрос | Что делать |
| --- | --- |
| Перенос одной `Entity`/`AnyView` между окнами: фокус, `FocusHandle`, инвалидация перерисовки | **Проверено:** `tearoff/any_view_transfer.rs`. Один и тот же `EntityId` сохраняется; текст, счётчик, курсор, выделение и ввод работают при переносе туда-обратно. В проверенном сценарии фокус не пропадал и повторный клик не требовался. Оставлены за рамками теста: одновременный рендер в двух окнах и IME-композиция. Используем одну `Entity`/`AnyView`, без пересоздания представления |
| Минимальный размер окна по `PanelConstraints` меняется после создания | Выяснить, есть ли у выбранного коммита gpui динамический `min_size`. Иначе обработка `WM_GETMINMAXINFO` в `dock-native` |
| Среднее правая кнопка / контекстное меню в floating-окне: меню сейчас рисуется внутри `DockHost` | Для первой версии — меню и flyout только в главном окне; в floating меню рисуется внутри своего окна и клипуется по его границам |
| Flyout / rail внутри floating-группы (сейчас поддерживается через `ColumnView`) | Решить: оставить в окне или отключить для floating (режим Expanded только) |
| Изменения размера/позиции окна — в undo или нет | Предложение: один шаг undo на жест (раздел 5, п. 7); отрыв, возврат и закрытие тоже по одному шагу. Нужно ваше решение |
| Опрос курсора каждые 4 мс через `cx.spawn` грузит UI-поток | В `main.rs` работает; при тяжёлом рендере замерить и при необходимости двигать окно в отдельном потоке, а в Core обновлять реже |
| Базовая платформа | Важно подтвердить: только Windows в этой итерации, на остальных — панели не отрываются |
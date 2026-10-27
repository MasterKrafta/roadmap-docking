# Архитектура workspace

Стрелка означает «зависит от» и отражает реальные зависимости Cargo, а не членство в workspace.

```text
app
 └──▶ dock-gpui ──▶ dock-core ──▶ slotmap
       │   │          ├────────▶ thiserror
       │   │          ├────────▶ tracing
       │   │          └─[feature serde]──▶ serde / serde_json
       │   ├────────▶ ui
       │   ├────────▶ gpui (внешний crate)
       │   ├────────▶ thiserror / tracing
       │   └─[cfg(windows)]──▶ gpui_platform (внешний crate)

platform-windows ──▶ platform-api
```

`ui` — самостоятельный общий слой темы и UI-примитивов. `platform-api` и `platform-windows` входят в workspace, но пока не подключены к runtime-графу приложения: оба крейта намеренно содержат только заготовку будущего контракта R8.1. `gpui_platform` — отдельный внешний crate; это не то же самое, что локальный `platform-windows`.

## Границы

- `dock-core` не зависит от GPUI, Windows API или `ui`. Публичная модель раскладки меняется через типизированные `DockCommand` и `Layout::apply`; runtime-команды пользователя в приложении направляются через `DockController` и `UndoRedoHistory`.
- `Layout` владеет аренами узлов и панелей, dock-областями, floating-группами, Canvas, профилем и версией. Его поля закрыты; наружу доступны только read-only представления.
- Изменение содержимого панели (например, прокрутка или выбор элемента в Layers) не входит в `dock-core`; `PanelState` хранит только метаданные, относящиеся к раскладке.
- `dock-gpui` переводит пользовательский ввод в команды, держит controller/session state и рисует текущую модель. Отдельную копию дерева раскладки он не хранит.
- `DockController` — runtime-точка исполнения команд: undoable-изменения идут через `UndoRedoHistory`; `RaiseFloating` намеренно меняет порядок окон без записи Undo; первоначальная сборка layout выполняется до истории.
- `ui` содержит `Theme`, цвета, метрики, типографику и переиспользуемые UI-примитивы; не зависит от `dock-core` или `dock-gpui`.
- `dock-gpui` собирает `DockMetrics` из `ThemeMetrics`. Начальная раскладка, registry панелей и восстановление workspace находятся в адаптерном слое; ограничения панелей разрешаются по стабильному `PanelKey`.
- Canvas остаётся защищённой областью вне dock. Toolbar, меню и Options bar не являются панелями docking.
- В `dock-gpui` платформенный запуск и зависимость `gpui_platform` ограничены `cfg(windows)`. Linux CI проверяет компиляцию и тесты портируемых слоёв, но это не означает, что runtime-приложение уже поддерживает открытие окна на Linux: вне Windows `run` пока сообщает о неподдерживаемой платформе и завершается.
- В `dock-core` запрещён `unsafe`; платформенный код не должен проникать в ядро.

## Карта взаимодействий: жест → команда

Таблица описывает команды, которые формирует текущий UI-адаптер. Core остаётся источником истины для проверки целей drop и применяет команды транзакционно.

| Жест или источник | Команда Core | История / поведение |
| --- | --- | --- |
| Клик по вкладке или выбор скрытой вкладки из меню переполнения | `ActivateTab` | Через controller/history; no-op не создаёт checkpoint |
| Dockers: открыть отсутствующую панель / переключить уже открытую | `OpenPanel` / `ClosePanel` | Undoable при изменении layout |
| Меню группы: закрыть панель / закрыть группу | `ClosePanel` / `CloseGroup` | Undoable |
| Двойной клик по вкладке, пункт меню группы или кнопка в заголовке floating-группы | `CollapseGroup` | Переключает свернутость группы; Undoable при изменении layout |
| Перетаскивание вкладки на другую группу, край, слот колонки или рейки | `MovePanel` | Resolver выбирает команду по цели; Undoable |
| Перетаскивание вкладки из dock на свободную область | `Float` | Отрыв панели во floating-группу; Undoable |
| Перетаскивание панели из flyout в dock / на свободную область | `MovePanelFromFlyout` / `FloatPanelFromFlyout` | Undoable |
| Перетаскивание группы за grip на вкладки другой группы / край группы | `MergeGroup` / `SplitGroup` | Undoable |
| Перетаскивание группы за grip на слот колонки или рейки / главную колонку / свободную область | `MoveGroup` / `MoveGroupToMainColumn` / `Float` | Resolver учитывает вид цели; Undoable |
| Перетаскивание floating-группы за отдельную захватную ручку в dock | `DockFloating` | Только на допустимую цель; Undoable |
| Перетаскивание floating-группы за заголовок | `MoveFloating` | Одна запись истории на завершённый жест |
| Клик по floating-группе для вывода на передний план | `RaiseFloating` | Меняет hit-test/paint order, не создаёт Undo и не очищает Redo |
| Ресайз края или угла floating-группы | `ResizeFloatingBounds` | Одна запись истории на завершённый жест |
| Вертикальный сплиттер между группами | `ResizeGroups` | Одна запись истории на завершённый жест |
| Внешняя ручка ширины колонки | `ResizeColumn` | Одна запись истории на завершённый жест |
| Горизонтальный сплиттер главной области / внутри floating-колонки | `ResizeColumnBoundary` / `ResizeSplitColumnBoundary` | Одна запись истории на завершённый жест |
| Двойной клик по сплиттеру групп / главных колонок | `ResetColumnRatios` / `ResetMainColumnRatios` | Возвращает пропорции; Undoable |
| Кнопка режима колонки или автоматическое пересечение порога ширины | `SetColumnMode` | Переключает Expanded / IconsAndLabels / IconsOnly; Undoable |
| Клик по записи рейки / кнопка закрытия flyout / клик снаружи или Escape | `OpenFlyout` / `CloseFlyout` / `DismissFlyout` | Временное UI-состояние, не записывается в Undo |
| Изменение ширины flyout / настройки auto-collapse в меню | `ResizeFlyout` / `SetFlyoutAutoCollapse` | Временное UI-состояние, не записывается в Undo |
| Tab / Shift+Tab | `SetChrome` | Изменяет видимость chrome; команда идёт через controller |
| Ctrl+Z / Ctrl+Y или Ctrl+Shift+Z | `UndoRedoHistory::undo` / `UndoRedoHistory::redo` | Это операции истории, а не варианты `DockCommand` |
| Закрытие floating-композиции | `CloseGroup` для каждой tab-группы или `ClosePanel` для отдельного panel leaf | Сейчас нет атомарной `CloseFloating`: каждая команда имеет отдельный Undo-шаг |

Некоторые варианты Core намеренно не производятся активным drag resolver: `FloatGroupAsColumn` и `FloatPanelAsGroup` остаются отдельными командами, пока UI не задаст для них явный UX. `RestoreGroupRatios` предназначена для восстановления сохранённого в истории состояния, а `ResizeFloating` — для программных сценариев, где нужно изменить только размер.

Правила истории применяются по фактическому изменению `Layout::version`: no-op и transient-команды не создают checkpoint; новый persistent-изменяющий командный шаг после Undo очищает ветку Redo. Drag preview не меняет раскладку до commit.

## Контракт workspace serialization

- Единственный публичный формат файла рабочего пространства — JSON. Текущий envelope содержит `format: "roadmap-docking-workspace"` и `version: 2`; version 1 мигрирует к версии 2; формат или версия должны проверяться до восстановления.
- Миграция v1→v2 восстанавливает отсутствующий `last_expanded_width` из `width`, только если колонка сохранена в `Expanded`; для rail-режимов прежнюю ширину нельзя вывести однозначно, поэтому она остаётся `None`. Любая миграция возвращает `SchemaMigrated` в отчёте загрузки.
- Workspace serialization API и JSON-зависимость включаются только явным feature `dock-core/serde`; по умолчанию feature выключен.
- Сериализуется DTO из `WorkspaceFile`, а не внутренний `Layout` и не отдельные arena-backed структуры. Дерево хранится рекурсивно: в файле нет runtime `NodeId`, `PanelId`, `DockAreaId` или `FloatingGroupId`; panel leaf ссылается на стабильный `PanelKey`.
- Сохраняются профиль, предпочтение видимости chrome, известные bounds Canvas, структура Split/Tabs/Column, отношения групп и их ratios, активный индекс и collapse state вкладок, режим/ширина колонки и последняя expanded width, layout-only размер панели, bounds и z-order floating-групп.
- `PanelConstraints` принадлежат реестру панелей в адаптерном слое (`dock-gpui::panels`) и при загрузке разрешаются по `PanelKey`; они не копируются в файл. Загрузка создаёт новые SlotMap arenas и проверяет полученный `Layout` текущими метриками.
- Flyout selection, runtime version, session-only `LastDockLocation`, UI/scroll handles и panel content state в workspace JSON не входят. Содержимое панели и её scroll/selection state — отдельный app-owned канал.
- Загрузчик возвращает `WorkspaceLoadResult` с diagnostics. Неизвестный/удалённый `PanelKey` сохраняется как placeholder с безопасными layout constraints и предупреждением; приложение может показать недоступную панель, а её исходное место остаётся в workspace.
- Загрузчик применяет только однозначные исправления: ratios/структура, которые поддерживает нормализатор, и ширина колонки, однозначно заданная `ColumnMode`. Нарушение правил профиля, где нужно гадать или терять структуру, возвращается как `InvalidLayout` с конкретными `ValidationError`; тихого разрушительного ремонта нет.
- Для floating bounds приложение может передать в `WorkspaceLoadOptions` видимый client rectangle в логических px. Core ограничивает bounds этой областью и сообщает об изменении, но не сжимает содержимое ниже его минимального размера: если client region слишком мал, загрузка возвращает `FloatingBoundsInsufficientSpace`. Определение монитора, физического DPI и предпочтительной позиции окна остаётся в app/platform слое.

## Undo/Redo contract

- Session Undo/Redo входит в `dock-core` как `UndoRedoHistory`; история намеренно не сохраняется в JSON workspace для MVP.
- История ведётся через фасад `UndoRedoHistory::apply`. После успешной команды, увеличившей `Layout::version`, записывается checkpoint до изменения. Это session-only снимки всей модели, а не последовательность best-effort inverse commands: так отмена покрывает команды, для которых inverse ещё отсутствует, включая reorder/move, ratios, floating bounds и stacking.
- UI-адаптер должен отправлять undoable user commands через `UndoRedoHistory::apply`; прямой вызов `Layout::apply` сохраняется для сценариев без истории и намеренно не записывает undo entry.
- No-op (не изменивший версию), отклонённая команда и transient flyout-команда не создают запись. Drag preview не меняет Layout и в API истории не входит. После новой persistent-команды на разветвлении undo redo-ветка очищается.
- Undo/Redo восстанавливает точное логическое persistent-состояние, но не откатывает `Layout::version`: версия остаётся монотонной. ID существующих в обеих версиях сущностей сохраняются, удалённые и воссозданные сущности получают новые runtime keys, поэтому stale handles не оживают; UI-адаптер должен перечитать ID после undo/redo. Flyout при восстановлении закрывается; session-only flyout auto-collapse preference сохраняется текущим.
- История ограничена количеством записей (по умолчанию 100); `clear()` сбрасывает её без изменения раскладки. Ошибка проверки checkpoint при текущих metrics не меняет Layout и возвращает запись в стек.

## Ограничения реализации

- Один собственный `struct`/`enum` на файл. `lib.rs` и `mod.rs` содержат только объявления модулей и re-export.
- Значения геометрии — логические пиксели в `f32`. Округление до физических пикселей происходит на границе отрисовки.
- `dock-core` сообщает для вертикального overflow viewport и полный `content_extent`, но сам не прокручивает содержимое; `dock-gpui` обеспечивает column-level scrolling. Tab-strip overflow возвращает видимые заголовки, область overflow-кнопки и ID скрытых вкладок; отображение overflow-кнопки и списка — UX-решение проекта, а не утверждение о точном поведении Photoshop.
- Геометрия рассчитана на положительный viewport: нулевые размеры отклоняются. Если Canvas или split не помещаются по ширине, Core возвращает явную ошибку нехватки места вместо прямоугольников отрицательного размера. По вертикали Core сохраняет минимальные размеры содержимого и сообщает полный `content_extent`; клиппинг и прокрутка реализованы на стороне GPUI-адаптера. Пустая раскладка оставляет Canvas на всю доступную область при соблюдении минимальной ширины Canvas.
- В `dock-core` запрещён `unsafe`; GPUI и платформенный код изолированы от ядра.

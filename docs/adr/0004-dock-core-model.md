# ADR 0004 — модель Dock Core и границы состояния

**Статус:** принято  
**Дата:** 2026-10-21

## Контекст

Рассматривались несовместимые модели: вкладки со списком PanelId и дерево с отдельными узлами Panel. Эти варианты нельзя смешивать: при первой модели вкладки владеют идентичностью панелей напрямую, при второй структура дерева явно выражает листья и позволяет ссылаться на дочерние узлы.

## Решение

### Дерево и владение

- Node — сумма вариантов Split, Tabs, Panel и Column. Каждый вариант объявляется в собственном Rust-файле.
- Node::Panel — лист со ссылкой на уникальный runtime PanelId. Данные экземпляра живут отдельно как PanelInstance, принадлежащий Layout.
- Tabs содержит упорядоченные NodeId, active_index и флаг collapsed. В профиле Photoshop дети Tabs — только узлы Panel.
- Общий Split содержит ось, не менее двух дочерних NodeId и соответствующие ratios. При нормализации split с одним ребёнком схлопывается.
- Column — контейнер профиля Photoshop с width, mode, упорядоченными группами и group_ratios. Колонка с одной непустой группой сохраняется и не схлопывается.
- DockArea содержит kind (Main или Floating), сторону для main-area и один корневой NodeId.
- Корень main dock — одна Column либо Split(X) из колонок. Слева и справа допускается не более одной main dock area; Canvas находится вне деревьев.
- Корень floating area — Tabs либо Split(Y) из Tabs. Bounds и z-order принадлежат метаданным floating-area, а не панелям по отдельности.

### Arena и идентичность

Layout является единственным владельцем арен узлов, экземпляров панелей, main dock areas и floating areas. Объекты не дублируются в независимых деревьях/коллекциях. Внутренние NodeId, PanelId, DockAreaId и FloatingGroupId — runtime handles, не стабильные пользовательские ID. При загрузке сохранённой раскладки ссылки преобразуются в новые runtime handles; PanelKey остаётся стабильным ключом вида панели.

### Профили

Профиль выбирается явно и сохраняется в workspace:

- Generic предоставляет общую модель без ограничений Photoshop на форму дерева.
- Photoshop включает правила колонок и групп, запрещает вложенные split в dock-группах и задаёт семантику сворачивания/flyout.

Профиль не выводится автоматически из формы дерева.

### Границы состояния

- Persistent layout включает дерево, экземпляры панелей, dock areas, floating bounds/z-order, Canvas bounds/защиту, профиль, версию и persistent chrome visibility.
- `CanvasState` обязателен и находится вне дерева; он может помнить последние измеренные bounds Canvas, но не хранит состояние документа/сцены. Flyout, drag preview и другие transient UI-состояния хранятся отдельно от persistent дерева и сами по себе не создают undo-записи. Текущий `FlyoutState` лишь ссылается на Column и группу, не создаёт дополнительные узлы и не сериализуется.
- `PanelState` may remember layout-only metadata such as `last_known_size`; it must not store content state.
- Состояние содержимого панели принадлежит app/panel implementation, а не Core.
- dock-core не зависит от GPUI, Theme, платформы, окон, представлений, UI handles, scroll handles или фабрик содержимого. PanelDescriptor, иконки, фабрики и источники меню принадлежат app.

### Валидация

`Layout::validate(&DockMetrics)` собирает полный список диагностик, а не прекращает проверку на первой ошибке. Проверяются разрешимость runtime-ссылок, циклы и повторные родители, достижимость узлов, соответствие `PanelId`/`PanelInstance`, singleton-панели, арность и ratios Split, активная вкладка Tabs, группы и размеры Column, геометрия Canvas/floating-area и связи DockArea/FloatingGroup. Правила формы дерева Photoshop применяются только при `Profile::Photoshop`. Ratio tolerance — `1e-4`, pixel tolerance для метрик — `1e-3`.

### Нормализация

`normalize_in_place(&DockMetrics)` — приватный помощник: вызывается только на candidate-копии внутри транзакции или в тестовом harness. Пустые Tabs, Columns и Split-контейнеры удаляются; пустой корневой узел удаляет соответствующую dock area. Общий Split с одним ребёнком схлопывается, а ratios оставшихся детей нормализуются; отсутствующие/повреждённые ratios заменяются равными долями. Непустая Column сохраняется даже с одной группой. Активная вкладка сохраняется при возможности, иначе индекс ограничивается диапазоном оставшихся вкладок. Перед преобразованиями блокируются повреждённые ссылки, циклы, повторные родители, несогласованные PanelInstance и нарушения профиля; после нормализации обязательно вызывается validate.

### Тестовая сборка модели

`LayoutBuilder` существует только в test configuration, создаёт реальные runtime handles через Rust API и не использует парсер текстовой схемы. Unit-тесты покрывают stale handles, дубликаты размещения панелей, циклы, пропущенные ссылки, orphan nodes, singleton, ограничения Photoshop и очистку пустых контейнеров. Property-тесты генерируют допустимые композиции и варианты ratios; каждая успешно нормализованная раскладка повторно проходит validate, а повторная нормализация должна быть идемпотентной. После стабилизации операций C4 property-тесты расширяются на случайные последовательности команд, включая проверки инвариантов после каждой успешной команды и неизменности состояния после отказа.

### Изменение раскладки

Все изменения Layout извне проходят через `Layout::apply(DockCommand, &DockMetrics)`. Команда применяется к candidate copy, затем состояние нормализуется и валидируется. Исходный layout заменяется только после полного успеха; неуспешная команда не меняет состояние. Транзиентные действия тоже проходят через этот путь, но не увеличивают persistent version и не создают undo record.

### Контракты команд (C4.1)

- `DockCommand`, `DropTarget`, `DropZone`, `CommandError`, `CommandOutcome` и значения фактического результата объявляются в отдельных файлах.
- Команды описывают намерение и не зависят от UI; применение выполняет `Layout::apply` через транзакционный candidate.
- `DropTarget` задаёт точную цель: индекс вкладки, край существующей группы, индекс колонки на стороне dock или индекс вставки в icon rail. Для rail используется отдельный `RailSlot { column, index }`, чтобы порядок не выводился из координат указателя.
- `MoveGroup` задаёт колонку назначения и точный индекс группы; `SplitGroup` задаёт соседнюю группу, край и пропорцию разделения. Это разделяет перестановку в колонке и операцию разбиения.
- Команды изменения размера принимают запрошенную величину/пропорции. `CommandOutcome` сообщает изменённые узлы, новую версию, фактическое значение после ограничений и необязательную обратную команду.
- Сериализуемые команды, цели, зоны и результаты поддерживают optional Serde feature. `CommandError` остаётся runtime-диагностикой и не является частью сохранённого workspace.
- `CollapseGroup`, `SetColumnMode`, `OpenFlyout`, `CloseFlyout` и `SetChrome` уже представлены как контракты; их применение и интеграция с UI не входят в C4.1. Скрытие chrome не перестраивает дерево.

### Транзакционный apply (C4.2)

- `Layout::apply(command, metrics)` — единственная публичная точка изменения состояния Layout. Арены и их изменяемые ссылки не выдаются наружу; нормализация остаётся приватной.
- До создания candidate проверяется исходный Layout, затем команда дополнительно проверяет ID, тип и положение цели, геометрию/constraints и числовые аргументы. Любая операция и нормализация выполняются над клоном.
- Candidate проходит нормализацию и полную валидацию; commit заменяет исходный Layout только если обе операции успешны. При любой ошибке исходный Layout, включая version и runtime handles, не изменяется.
- Изменившая persistent state команда увеличивает version ровно на единицу. No-op не увеличивает version и возвращает `inverse: None`. Flyout-команды могут менять transient состояние без увеличения persistent version и без undo record.
- `CommandOutcome` содержит изменившиеся NodeId, итоговую version, необязательное эффективное значение resize и необязательную обратную команду. Обратная команда выдаётся только там, где core может безопасно предложить её; перестановка групп сохраняет корректный insertion index для обратной операции.
- `SplitGroup` делит пропорцию именно целевой группы/колонки, а не соседнего элемента, когда размещение происходит снизу или справа.
- Принятие и отклонение команд логируются через `tracing`. Ошибки входных данных возвращаются как `CommandError`, не как panic.
- Unit-тесты покрывают изменение с одним увеличением version, повтор команды как no-op, drop на текущую позицию/самого себя, отказ по отсутствующему ID, некорректное числовое значение, overflow version, rollback после отказа финальной валидации (включая лимит колонок Photoshop), восстановление порядка и ratios групп обратной командой, target-anchored ratios для вертикального split и transient flyout без изменения version. Property-тесты последовательностей команд добавляются после стабилизации операций.
 
### Операции вкладок и lifecycle (C4.3)

- `ActivateTab` активирует уже существующий Panel leaf; `ReorderTab` меняет порядок вкладок, сохраняя активный leaf, а не старый числовой индекс.
- `MovePanel` переносит тот же Panel leaf и PanelInstance между группами, main dock и floating areas. В целевой группе перемещённая вкладка становится активной; удалённый исходный контейнер убирается нормализацией.
- `ClosePanel` удаляет Panel leaf и принадлежащий ему PanelInstance. `CloseGroup` удаляет все листья и экземпляры целевой Tabs-группы. После каждой успешной операции candidate нормализуется и проходит validate до commit.
- Повторный `OpenPanel` для уже открытого singleton активирует его, не закрывает и не создаёт новый экземпляр. Для `OpenPanel { target: None }` сначала используется последняя runtime dock-позиция, затем профильный fallback: первая подходящая Tabs-группа справа, либо первая колонка справа, если dock ещё пуст.
- `LastDockLocation` хранится только в in-memory `Layout`, по стабильному `PanelKey` и ordinal позиции side/column/group. При закрытии панели/группы позиция запоминается до нормализации; floating не перезаписывает последнюю dock-позицию, а успешное docking обновляет её. Тип и карта не реализуют Serde и не входят в формат workspace.
- Решение сохранять last-location в workspace **отложено** до отдельного этапа workspace persistence. До этого перезапуск/создание Layout сбрасывает runtime preference; при отсутствии валидной позиции используется профильный fallback.

### Split, MoveGroup и перемещение между областями (C4.4)

- Геометрия hit-test не передаётся в Core: команды получают семантические цели `DropTarget` и `DropZone`. `Tabs` — явная цель вставки вкладки/merge; `GroupEdge` задаёт Top/Bottom/Left/Right; `MainColumn { side, index }` задаёт точный слот колонки, включая крайние слоты 0 и len; `RailSlot` задаёт точный слот рейки.
- Для профиля Photoshop Top/Bottom создаёт группу в той же Column или вертикальном Split(Y) плавающего стека; Left/Right создаёт соседнюю main Column по горизонтальной оси X. Вставка колонки на край выражается `MainColumn` с индексом 0 или числом текущих колонок.
- `MoveGroup` перемещает существующий Tabs NodeId, не пересоздавая Tabs, Panel leaves или PanelInstances. Поэтому порядок вкладок, active_index, collapsed и PanelId сохраняются при переносе между колонками. `Float` и `DockFloating` перемещают тот же Tabs root, когда целью является отдельная колонка/рейка.
- Drop на собственную позицию или на соседнюю сторону, которая уже даёт тот же порядок дерева, возвращает no-op без version/undo. Горизонтальный SplitGroup no-op, если исходная группа — единственная в соседней колонке и запрошенный край совпадает с её уже существующим положением.
- `max_columns` — окончательный инвариант Photoshop-профиля. Кандидат может временно иметь пустую исходную колонку во время переноса группы; нормализация разрешает диагностике TooManyColumns пройти preflight, чтобы убрать пустые контейнеры, но обязательно запускает полную validate после нормализации. Если итог всё ещё превышает лимит, транзакция отклоняется целиком и исходный Layout/version остаются неизменными. Поэтому устаревший drop-target не может зафиксировать лишнюю колонку.
- Тесты проверяют создание горизонтальной соседней колонки, сохранение набора вкладок/active_index/collapsed/PanelInstances через Column → FloatingGroup → MainColumn и перенос в существующий floating stack, no-op собственной/соседней позиции по горизонтали и вертикали, перенос при максимальном числе колонок с удалением пустой исходной колонки и атомарный отказ, когда итоговый dock всё ещё превышает лимит.

### Resize командная логика (C4.5)

- `ResizeGroups` адресует ровно одну границу через `boundary_index` и signed `delta`; `available_height` — измеренная высота содержимого Column после вычета splitter gaps, не координаты мыши. Для обеих соседних раскрытых групп Core рассчитывает текущие высоты по весам, учитывает `max(metrics.min_group_height, panel min height)` и ограничивает фактическую дельту, возвращая `CommandValue::GroupBoundaryDelta`. Граница рядом со свёрнутой группой не перемещается; collapsed height фиксирована.
- Группы получают свободную высоту пропорционально своим весам; collapsed-группы из распределения свободной высоты исключены. `ResetColumnRatios { column, available_height }` делит свободную высоту поровну между раскрытыми группами, а группы, упёршиеся в minimum height, фиксируются на минимуме и оставшаяся высота распределяется между остальными. Для точного undo существует `RestoreGroupRatios`; это команда восстановления сохранённого вектора, а не интерактивный resize.
- Старый reset горизонтальных долей главного dock теперь имеет отдельное имя `ResetMainColumnRatios { side }`, чтобы не смешивать горизонтальные ширины колонок и вертикальные доли групп в одной колонке.
- `ResizeColumn` допустим для колонки у внешней границы dock, обращённой к Canvas (последняя колонка слева или первая справа); внутренние колонки должны менять ширину через `ResizeColumnBoundary`. Он проверяет requested width на finite/positive, затем clamp-ит его в диапазон из метрики минимальной ширины, минимальных размеров panel и заданных panel max widths. Если известны последние bounds Canvas, увеличение dock-колонки уменьшает ширину Canvas на ту же дельту; для левого dock также сдвигается Canvas origin. Clamp учитывает `min_canvas_width`, когда доступная ширина известна. Если bounds ещё не измерены, окончательный остаток рассчитывает UI geometry pass.
- `ResizeColumnBoundary` вычисляет допустимый диапазон дельты сразу из min/max обеих соседних колонок и применяет противоположные дельты атомарно. Суммарная ширина пары сохраняется; достижение min/max возвращает фактическую дельту, а не ошибку. Не finite delta и некорректные размеры отклоняются без commit.
- Тесты покрывают clamping группы на minimum, фиксированную collapsed height, равные доли раскрытых групп, распределение при различающихся minimum heights, max/min колонки, сохранение суммы ширин, остаток Canvas и отказ при NaN/Infinity/неположительной доступной высоте.

### Floating-команды и Z-order (C4.6)

- `Float { node, bounds }` переносит Panel leaf или Tabs-группу в новую FloatingGroup. В Photoshop одиночная панель получает контейнер `Tabs`, но сам Panel leaf и его `PanelId`/`PanelInstance` не создаются заново. При переносе целой Tabs-группы сохраняются её NodeId, порядок дочерних вкладок, active index и collapsed state.
- `MoveFloating` изменяет origin окна при конечных координатах; `ResizeFloating` принимает только положительный конечный размер и возвращает эффективный размер после ограничения минимальными размерами содержимого; `RaiseFloating` задаёт z-order выше остальных floating windows. FloatingGroup и DockArea удаляются только когда окно действительно dock-ится или опустошается нормализацией.
- `DockFloating` в отдельную MainColumn/рейку переносит существующие Tabs-группы без пересоздания panel leaves. Для root Tabs и center-target команда объединяет панели в целевую Tabs-группу в исходном порядке и активирует вкладку, которая была активной в исходной плавающей группе. Split(Y)-стек переносится в Column/рейку как упорядоченный список Tabs-групп; при этом ratios становятся долями group heights, а сами Tabs-группы остаются отдельными.
- Одна панель внутри плавающего стека может быть перенесена через `MovePanel` в другую Tabs-группу; целая Tabs-группа переносится через `SplitGroup` сверху/снизу, где создаётся или расширяется вертикальный Split(Y). В Photoshop horizontal Left/Right group split внутри FloatingGroup отклоняется на этапе проверки семантической цели, а транзакция оставляет Layout/version неизменными. Горизонтальный Split в generic profile остаётся допустимым.
- `MoveFloating`, `ResizeFloating`, `RaiseFloating` и `DockFloating` выполняются через `Layout::apply`, поэтому каждая успешная команда проверяется после нормализации и до commit. Для center merge сохраняются все существующие PanelId/PanelInstance; активная вкладка исходной floating Tabs-группы становится активной в целевой группе.
- Тесты C4.6 покрывают Float одиночной панели, перемещение/resize/raise окна, clamping размеров, центр-merging с сохранением active index, перенос панели между группами внутри Split(Y), перенос целой Tabs-группы наверх floating stack, dock стека в Column без flattening Tabs-групп и атомарный отказ от horizontal split в Photoshop.
- Успешный перенос последней панели из floating root приводит к удалению пустого Tabs root, FloatingGroup и связанного DockArea в процессе нормализации. Если команда отклонена, candidate не commit-ится: исходная FloatingGroup/area/root, PanelInstance и version остаются неизменными.
- Геометрия floating хранится как обычные finite logical-pixel `Rect` bounds. Core не знает о GPUI или мониторах и не меняет persistent floating bounds; чистый `compute_rects` adapter-style resolver ограничивает только возвращаемые geometry bounds переданным client viewport. Отрицательный конечный origin допустим в модели.
- Для однозначного порядка рендеринга `floating_groups_in_paint_order()` возвращает окна по возрастающему z_order (back-to-front), а `floating_groups_in_hit_test_order()` — по убыванию (front-to-back). Layout validation запрещает повторяющиеся z_order; `RaiseFloating` назначает уровень выше текущего максимума.
- Дополнительные regression-тесты проверяют удаление floating area при успешном переносе последней панели, сохранение исходного area при отказе DockFloating, отсутствие экранного clamping в модели, обратный порядок paint/hit-test и merge одной floating-группы в другую без потери PanelId/PanelInstance.

### Последовательности команд и негативные сценарии (C4.7)

- Команды, имеющие runtime handles, проверяются с отсутствующими `NodeId`, `PanelId` или `FloatingGroupId`. Для каждого отказа сравнивается полное debug-представление Layout и `version` до/после; после rollback исходный Layout обязан проходить `validate`.
- Отдельная матрица проверяет ID существующего, но неверного типа узла и запрещённые назначения: rail drop в Expanded column, tab/group-команды с Column вместо Tabs, flyout для не принадлежащей rail группе и docking floating-окна в несовместимую цель.
- Drop/move на ту же группу или ту же позицию считается допустимым no-op: версия не изменяется, inverse отсутствует, Layout остаётся валидным.
- Property-тест `generated_command_sequences_from_current_ids_remain_valid` использует `proptest`: строит до 35 команд на последовательность, на каждом шаге заново получает доступные IDs из текущего Layout и формирует только применимые команды. После каждого успешного `apply` проверяется `validate`; неожиданная ошибка generated-команды валит property.
- Ограничения размеров и количества колонок опираются также на focused regressions C4.4/C4.5: `c44_stale_horizontal_drop_at_column_limit_is_rejected_atomically`, `c45_resize_column_clamps_min_and_max_and_canvas_gets_remainder`, `c45_resize_rejects_non_finite_or_non_positive_inputs_atomically`, а также тесты невозможности разместить группы при недостаточной доступной высоте.
- Дополнительная failure-atomicity матрица охватывает некорректные constraints/индексы/ratios, недостаточную высоту, non-finite resize/move, пустые bounds, stale IDs и ошибки целей. Для каждой ошибки сравниваются полное состояние Layout, version и результат `validate`.
- Под feature `serde` журнал `Vec<DockCommand>` сериализуется в JSON, декодируется и проигрывается на таком же исходном состоянии; после каждой команды выполняется `validate`. Итог сравнивается семантическим снимком дерева, а не сырыми runtime handles.
- Семантический снимок включает профиль/chrome/canvas, ordered tree shape, ключи и constraints панелей, активные/свёрнутые tabs, split/column ratios, floating bounds/Z-order и flyout. Отдельный тест строит одинаковую раскладку с другим порядком выдачи PanelId/leaf NodeId и подтверждает идентичность снимка.
- Важно: `DockCommand` сохраняет runtime-ID ссылки на цели. Поэтому replay такого JSON-журнала требует того же совместимого исходного дерева/набора handles; независимое восстановление журнала поверх workspace с заново выданными IDs потребует отдельного стабильного selector/rebinding слоя и не заявляется как гарантия этого этапа.

### Чистые геометрические результаты (C5.1)

- Каждый результат размещения имеет свой файл и содержит только Rust-данные/модельные handles и `Rect`: `LayoutRects`, `ColumnRect`, `GroupRect`, `TabStripRect`, `TabHeaderRect`, `PanelRect`, `SplitterRect`, `FloatingGroupRect`, `FloatingResizeEdgeRect`, `DropZoneRect`, `HitResult`.
- `LayoutRects` — контейнер результатов geometry pass для основных областей, колонок, групп, tab strip/header, содержимого панелей, splitter, floating windows/resize handles и drop zones. Отдельные записи сохраняют ID и индекс, необходимые для преобразования hit в семантическую команду.
- `FloatingGroupRect` сохраняет `z_order` рядом с bounds, чтобы геометрический снимок мог передавать тот же порядок отрисовки/hit-test, что и модель.
- `HitResult` — tagged enum с семантическими вариантами для column/group/tab strip/tab header/panel/splitter/floating group/resize edge/drop zone, центрального Canvas и miss. Он не хранит GPUI entity, view, mouse event или platform window.
- `ResizeEdge` — отдельный pure-data enum для четырёх сторон и четырёх углов плавающего окна; геометрические типы и результаты допускают optional Serde feature и не вводят зависимость от UI/GPUI.

### Чистый resolver геометрии (C5.2)

- Публичный метод `Layout::compute_rects(viewport, metrics)` — pure pass: он не меняет Layout, CanvasState, version, logical widths, ratios или floating bounds. Возвращает `LayoutRects`, включая `canvas: Some(Rect)`; значение `None` в новом/пустом snapshot означает, что geometry pass ещё не выполнялся.
- Main dock слева привязан к левой границе viewport, справа — к правой. Для `Split(X)` порядок дочерних колонок читается слева направо и сохраняется. Expanded column использует её сохранённую `Column.width`; IconsOnly использует `rail_width` и соблюдает существующий инвариант модели, что `Column.width == rail_width` в этом режиме. В icon rail `GroupRect` обозначает fixed-height group hit target; tab headers и panel content внутри узкой рейки не выдаются. Внутренние splitter gaps учитываются при вычислении общей ширины dock.
- Canvas получает ровно остаток ширины viewport после левого и правого dock; его высота равна viewport height. Resolver не сжимает колонки и не создаёт Canvas уже `min_canvas_width`: возвращает `ComputeRectsError::InsufficientCanvasWidth`. При нехватке вертикальной высоты `LayoutRects.vertical_overflow` содержит viewport и расширенный `content_extent`; группы получают минимум по своим constraints и не сжимаются ниже него. Adapter отвечает за scroll/clip, а ratios и persistent model остаются неизменны.
- Высота stack-групп рассчитывается после резервирования splitter gaps и фиксированных collapsed heights. Collapsed-группа занимает `max(collapsed_group_height, tab_strip_height)` и полностью исключается из набора ratio weights. Остаток между обычными группами делится по их ratios с water-filling: нижний предел каждой группы учитывает `min_group_height`, header, tab strip и минимальную высоту содержимого дочерних панелей/узлов; если сумма минимумов выше доступной высоты, выдаётся `VerticalOverflow` с полным `content_extent`, ratios не меняются.
- `GroupRect` включает всю область Tabs-группы; для раскрытой группы tab strip идёт под header, а `PanelRect` получает только content bounds активного дочернего узла. `TabHeaderRect` строится для каждого прямого Panel leaf в порядке вкладок с равными долями ширины. Для collapsed-группы сохраняется фиксированный strip-height даже тогда, когда родительский floating bounds выше; panel content bounds не выдаются. Splitter hit regions центрируются на фактическом splitter gap, а drop-edge rectangles адресуют группу семантическими `DropTarget::GroupEdge`. `VerticalOverflow` используется для Columns, вертикальных Splits, а также иных корневых узлов, минимальная высота которых больше выделенного viewport.
- Floating bounds в геометрическом snapshot clamp-ятся к переданному client viewport: origin ограничивается так, чтобы окно оставалось внутри области, а size увеличивается до минимального размера subtree или уменьшается до клиентского предела только если minimum всё ещё соблюдается. Minimum учитывает PanelConstraints, header и tab strip; если minimum физически не помещается в client area, возвращается `ComputeRectsError::InsufficientSpace`. Stored bounds модели при этом не мутируются. Floating rects и их содержимое выдаются back-to-front по уникальному `z_order`; geometry pass также строит восемь resize hit regions. Вычисление проверяет metrics, viewport и `Layout::validate`; NaN/Infinity не попадают в результат, ошибки возвращаются как typed result, без panic и без частичного изменения состояния.
- Unit-тесты покрывают фиксированные ширины колонок при изменении размеров окна, остаток Canvas, порядок и splitter, ratio allocation с per-panel minimums, исключение collapsed из ratios, `vertical_overflow`/content extent, точные эталонные Rect для предсказуемого viewport, active panel/tab bounds, floating clamp к client area, минимум floating содержимого, Z-order/resize edges и отказы при недостаточной ширине Canvas или невозможности уместить floating minimum в client area.

### Детерминированный hit-test (C5.3)

- Публичный `Layout::hit_test(point, rects, metrics)` работает только с логическими координатами, Layout handles и заранее вычисленным `LayoutRects`; window/view/event объекты не нужны. Snapshot должен быть рассчитан для того же Layout, что принимает hit-test. NaN/Infinity и некорректные прямоугольники возвращают геометрическую ошибку, а не меняют выбор попадания.
- Floating windows сначала сортируются по убыванию `z_order` (front-to-back). Для каждого окна в этом порядке сначала проверяются восемь resize-зон, затем title band. Если точка лежит внутри верхнего floating window, hit-test ищет только геометрию дерева этого окна; при попадании в пустую область возвращает `HitResult::FloatingGroup`, не передавая событие нижележащему floating/dock.
- Resize-зоны проверяются в порядке NW, NE, SW, SE, W, E, N, S: углы имеют преимущество над перекрывающимися краями. В частности, верхний край и верхние углы не могут быть ошибочно классифицированы как title drag, потому что resize проверяется раньше title band. Title band возвращает `HitResult::FloatingGroup`.
- Для dock geometry установлен порядок: splitter; верхняя header-полоса Tabs-группы (`HitResult::Group`, используемая как group/column header); tab header; drop zone; panel content; tab strip; тело группы; область Column; Canvas; Miss. В модели пока нет отдельного `ColumnHeaderRect`, поэтому существующая геометрия group header имеет собственную зону, а `ColumnRect` остаётся fallback-target для остальной области колонки.
- Все containment проверки наследуют полуоткрытую семантику `Rect::contains`: left/top включены, right/bottom исключены. На перекрывающихся hit-zone используется фиксированный приоритет выше и детерминированный порядок массива геометрии; hit-test не зависит от порядка OS окон или состояния UI runtime.
- Regression-тесты покрывают все восемь floating resize-zone directions, resize-over-title precedence, overlapping floating Z-order и запрет попадания сквозь front window, splitter hit-band над panel content и его полуоткрытую границу, соседние tab-header границы, group header, Canvas/Miss, collapsed tab header и non-finite pointer.


### Геометрия

Все координаты и размеры — логические px в f32. NaN и бесконечности запрещены. Допуск сравнения пикселей — 1e-3, ratios — 1e-4. Округление до физических пикселей выполняет rendering adapter на границе UI.

## Последствия

- Вкладки ссылаются на NodeId, а не хранят Vec<PanelId>.
- Идентичность панели и её позиция в дереве — разные понятия.
- Column остаётся профильным контейнером, а не разновидностью обычного Split.
- Команды, валидатор, нормализатор и геометрия могут тестироваться без приложения и UI runtime.
- Формат сериализации (JSON или RON) остаётся отдельным решением до реализации workspace persistence; Serde feature не делает runtime ID стабильными между загрузками.

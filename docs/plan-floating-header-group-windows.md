# План переделки Floating: отдельные окна шапок и Group

Проект: MasterKrafta/roadmap-docking  
Ветка: main  
Дата: 16 ноября 2026 г.  
Статус: планирование; код не изменялся.

---

## 1. Цель

Перестать отображать всю Floating Composition в одном HWND. В Floating каждое окно должно отвечать за один визуальный элемент:

- одна Column получает одно отдельное окно шапки;
- каждая Group внутри Column получает собственное отдельное окно;
- все эти окна располагаются как единая сетка через существующий WindowGrid;
- шапка закреплена сверху своей колонки и служит опорой для всей колонки;
- Group располагаются под шапкой. При добавлении или перемещении Group меняются положения окон Group, но сама шапка остаётся на месте.

Пример двух колонок:

        ┌──────────────┬──────────────┐
        │   Header A   │   Header B   │  верхний ряд шапок
        ├──────────────┼──────────────┤
        │   Group A1   │   Group B1   │
        ├──────────────┤              │
        │   Group A2   ├──────────────┤
        ├──────────────┤   Group B2   │
        │   Group A3   │              │
        └──────────────┴──────────────┘

Это схема принадлежности окон колонкам, а не требование одинаковой высоты Group.

Main пока остаётся одним главным окном со своей текущей Composition. Отдельные HWND для Header и Group создаются только в Floating. Прямой сценарий Main → Main в этот план не входит.

## 2. Зафиксированные правила

### 2.1. Структура Floating

1. У каждой Floating Column ровно одно окно Header.
2. У каждой Floating Group ровно одно окно Group.
3. Header находится в верхнем горизонтальном ряду своей колонки.
4. Все Group этой колонки находятся ниже Header и сохраняют порядок Core Layout.
5. Все окна Group внутри одной колонки выровнены по X и имеют общую ширину колонки.
6. Вставка Group сверху, снизу или между существующими Group не перемещает Header. Переставляются только окна Group, начиная с места вставки.
7. Создание новой колонки создаёт для неё собственное окно Header и окна её Group.
8. Удаление последней Group из колонки удаляет также Header этой колонки.
9. Перетаскивание за Header перемещает всю соответствующую колонку: Header вместе со всеми её Group. Соседние колонки не должны случайно становиться частью этого перемещения.

### 2.2. Разделение моделей и идентификаторов

Core Layout остаётся владельцем структуры Composition → Column → Group → Tab. Нативные окна — это отдельное представление этой структуры в GPUI, а не новая копия Dock Layout.

- FloatingGroupId продолжает обозначать Core Floating Composition/область. Он больше не используется как идентификатор каждого отдельного HWND.
- Для участников WindowGrid нужна типизированная идентичность окна: Header конкретной Column или окно конкретной Group. Предпочтительная форма — enum в GPUI-модели: Header(Column NodeId) или Group(Group NodeId).
- NodeId остаётся идентификатором Core-узла. Не создавать отдельные фиктивные FloatingGroupId для шапок.
- PanelId продолжает идентифицировать PanelView и вкладку, но больше не выбирает единственное окно, которое отображает всю Floating Composition.

Связь между Core-узлами, FloatingGroupId, WindowId/HWND и участниками WindowGrid хранится в GPUI-реестре окон.

### 2.3. Разделение постоянных и временных окон

Нужно строго различать:

- **DragHost** — временное окно активного переноса. Оно существует, пока пользователь держит кнопку мыши, и само по себе не становится постоянным участником WindowGrid.
- **Header window** — постоянное окно шапки Floating Column.
- **Group window** — постоянное окно одной Floating Group.

При подтверждённом Drop в Floating DragHost передаёт содержимое постоянному Group window. Если это безопасно, существующий HWND DragHost можно переиспользовать как Group window через механизм adoption; иначе нужен двухфазный handoff в созданное Group window. В обоих случаях Header создаётся отдельно. Не допускать одновременного монтирования одного AnyView в двух окнах.

Если Drop завершился в Main или жест отменён, временный DragHost закрывается, а постоянные Floating Header/Group окна не создаются.

## 3. Три сценария переноса

### Сценарий A. Main → Floating

1. Пользователь начинает перетаскивать вкладку из Main.
2. После прохождения drag threshold создаётся временный DragHost с переносимым содержимым. Создание временного окна не зависит от того, где в итоге будет Drop.
3. Если Drop принят в свободную Floating-область, Core создаёт Floating Composition с Column и Group. FloatingWindowManager создаёт постоянное окно Header и отдельное окно Group.
4. Если Drop направлен в существующую Floating Column:
   - Drop в TabStrip/append-зону добавляет вкладку в существующую Group; отдельное новое окно Group не требуется;
   - Drop в вертикальную зону вставки создаёт/перемещает Group в нужную позицию и соответствующее окно Group;
   - Drop, создающий новую Column, создаёт для неё отдельный Header и окна её Group.
5. После того как постоянное окно Group отрисовало перенесённые PanelView, DragHost освобождается через handoff.

Точная команда Core и вид DropZone остаются такими, какие уже определены текущим DockCore. Эта переделка не должна менять смысл работающих DropZone без отдельной причины.

### Сценарий B. Floating → Floating

1. При переносе вкладки или Group используется тот же DragHost и тот же общий маршрут разрешения Drop.
2. Core обновляет владение Tab/Group и структуру Column.
3. FloatingWindowManager согласует постоянные Header/Group окна с новым Layout.
4. Окно пустой исходной Group удаляется. Если в исходной Column ещё есть Group, её Header остаётся на месте, а оставшиеся окна Group перестраиваются.
5. Если перемещается вся Column за её Header, вместе перемещаются Header и все Group именно этой колонки.
6. После Drop не должно оставаться дублирующих окон, пустых Group или потерянных PanelView.

### Сценарий C. Floating → Main

1. Пользователь переносит Tab или Group из Floating в Main через существующий DragHost.
2. Core выполняет обычную операцию Drop в Main.
3. Окно вынесенной Group удаляется из Floating-сетки после безопасного handoff.
4. Если исходная Column остаётся непустой, её Header сохраняется, а оставшиеся Group располагаются под ним.
5. Если Column опустела, удаляются её Header и связанные записи WindowGrid.
6. Main продолжает отображать результат своей обычной Composition внутри главного HWND.

### Обязательный вариант сценария A: Main → временный Floating → Main

1. Пользователь начинает drag в Main.
2. DragHost создаётся и следует за курсором.
3. Не отпуская мышь, пользователь возвращается в Main.
4. Drop разрешается как операция в Main.
5. После отрисовки результата в Main временный DragHost закрывается.
6. Постоянные Header/Group окна не создаются. Не должно быть второго Core Drop или дублированной вкладки.

### Особый вариант: перетаскивание простой Composition за Header

Если исходная Main Composition состоит ровно из одной Column, одной Group и одной Tab, то при начале перетаскивания за её Header сразу формируется Floating-представление из двух отдельных окон:

1. Header window.
2. Group window с исходной одной вкладкой.

Оба окна образуют одну колонку WindowGrid. Это не означает, что для обычного tab drag нужно заранее создавать постоянное окно Header: для tab drag до подтверждённого Drop используется временный DragHost.

## 4. Роли существующих файлов

### Отображение компонентов

- crates/dock-gpui/src/dock-components/composition_header.rs
  - остаётся реализацией визуального компонента шапки и её кнопок;
  - должен уметь отобразить шапку одной Column;
  - не должен самостоятельно создавать HWND или управлять WindowGrid.

- crates/dock-gpui/src/dock-components/group.rs
  - остаётся реализацией визуальной Group: TabStrip, вкладки, grip/controls и поверхность содержимого;
  - должен переиспользоваться как содержимое одного Group window;
  - не должен самостоятельно управлять нативным окном.

- crates/dock-gpui/src/dock-components/composition.rs
  - остаётся сборщиком Composition для Main и preview;
  - больше не должен использоваться Floating как способ отрисовать все Group и Header внутри одного окна;
  - общие компоненты и обработчики из него нужно переиспользовать, а не копировать во вторую реализацию.

Важно: composition_header.rs и group.rs — компоненты отображения. Отдельными нативными окнами они становятся через GPUI root host, который монтирует соответствующий компонент.

### Оконная архитектура и перенос

- crates/dock-gpui/src/components/floating.rs
  - текущий FloatingHost отображает всю Floating Composition;
  - его нужно перевести на отображение одного типа участника окна либо разделить host-слои Header/Group, сохранив общий жизненный цикл там, где он одинаков;
  - не копировать обработчики фокуса, темы, redraw и общие подписки.

- crates/dock-gpui/src/floating_manager.rs
  - перестаёт выбирать один PanelId-host для всей Floating Composition;
  - начинает согласовывать набор постоянных окон Header/Group с актуальными Column/Group в Core Layout;
  - реестр окон должен быть привязан к типизированной идентичности окна, а не к PanelId или одному FloatingGroupId на всю Composition;
  - сохраняет ответственность за HWND, owner, show/hide, native bounds, DPI, handoff и reconcile.

- crates/dock-gpui/src/window_chain.rs
  - сохраняет модель связей/соседства и операции над WindowGrid;
  - переводит ключ участника с FloatingGroupId на типизированную идентичность Header/Group window;
  - описывает связи верхнего ряда Header и вертикальные цепочки Group внутри каждой Column;
  - не получает ответственность за Core Layout, рендеринг или создание HWND.

- crates/dock-gpui/src/window_chain_geometry.rs
  - сохраняет чистые расчёты native frame/client geometry и DPI;
  - пересчитывает расположение Header/Group без перекрытия шапки соседним окном;
  - вертикальные окна должны стыковаться границами, а не накладываться друг на друга на высоту CompositionHeader;
  - при вставке Group пересчитываются позиции окон её Column, при этом Header остаётся на прежнем месте.

- crates/dock-gpui/src/drag_host_manager.rs
  - остаётся единственным временным DragHost-механизмом для переноса содержимого;
  - использует общий маршрут Drop для Main и Floating;
  - передаёт результат в FloatingWindowManager через существующий handoff, не создавая второй параллельный drag manager.

- crates/dock-gpui/src/input_drag.rs
  - сохраняет общие правила начала/обновления/завершения жеста и существующий special-case tab reorder;
  - не должен получать отдельные копии логики для Main-origin и Floating-origin там, где различается только источник или target role.

Новые host-модули добавлять только если это действительно разделяет независимые обязанности. Визуальные компоненты не переносить в менеджер HWND, а менеджер HWND не помещать в dock-components.

## 5. Этапы реализации

Каждый этап — отдельный небольшой набор изменений. После локальной проверки пользователем этап фиксируется, и только после этого начинается следующий.

### Этап 0. Зафиксировать контракты и текущие переходы

<Статус: выполнен 16 ноября 2026 г.>

Цель этапа — не менять поведение, а зафиксировать реальный путь одного жеста от источника до окончательного Drop.

#### 0.1. Начало жеста и временный DragHost

1. input_drag.rs::begin_drag_at передаёт в DockController исходные DragPayload, DragHandle, DragSourceHost и WindowId. Core сохраняет семантический источник; DragHostId не заменяет его.
2. До прохождения drag threshold постоянные окна не создаются.
3. После перехода в Dragging UI вызывает один из существующих адаптеров:
   - begin_panel_drag_host для PanelTab/FlyoutPanel;
   - begin_group_drag_host для GroupGrip;
   - begin_dock_area_drag_host для DockAreaHeader.
4. Эти адаптеры используют DragHostManager::open_panel_at_cursor, open_panel_tab_at_cursor, open_group_at_cursor или open_dock_area_at_cursor. Все они сходятся в open_at_cursor_with_content.
5. DragHostManager создаёт одно временное native-окно, подключает его к DragHostSession, передаёт ему существующие cached PanelView и ждёт, пока исходное окно отрисует placeholder. Только после detachment источника временное окно показывается.
6. Для Main-origin PanelTab и GroupGrip включён promote_on_float. Для tab/group жестов резервируется высота CompositionHeader внутри временного DragHost; для DockArea drag эта высота отдельно не резервируется. Это старое поведение DragHost, не правило будущей постоянной геометрии Header/Group.
7. Для Floating-origin PanelTab/GroupGrip используется тот же DragHost lifecycle. В текущем коде promote_on_float для этих источников выключен: исходный Floating host обычно остаётся постоянным, а DragHost передаёт содержимое через handoff.
8. Перестановка вкладки внутри той же Floating TabStrip остаётся отдельным локальным сценарием finish_floating_tab_drag и не обязана создавать DragHost, пока жест остаётся внутри исходного TabStrip.

#### 0.2. Определение цели во время перетаскивания

1. DragHost отслеживает окно под курсором и хранит target вместе с target role.
2. Для Main вызывается Main drop resolver; для Floating — resolver, подготовленный для конкретных WindowId и FloatingGroupId.
3. Если под курсором нет управляемого Dock-окна, целевой зоной становится FreeArea. Для DockArea drag отдельное правило также рассматривает некоторые внешние окна как FreeArea.
4. Обычные цели внутри Main/Floating передаются Core без подмены: например, Tabs/GroupTabs, GroupEdge, ColumnSlot или MainColumn.
5. Для PanelTab, GroupGrip и DockAreaHeader, сброшенных на поддерживаемую внешнюю грань Floating, floating_window_chain_target вычисляет направление WindowGrid. В этом случае finish_active_drag намеренно заменяет Core request target на FreeArea: сначала создаётся самостоятельная Core FloatingGroup, затем её native-окно присоединяется к целевой WindowGrid.
6. Единое преобразование направления сейчас находится в floating_window_chain_direction:
   - Left → Left;
   - Right → Right;
   - Top/FloatingTop → Above;
   - Bottom/FloatingBottom → Below.
7. Drop в TabStrip/append-зону не является WindowGrid side-drop: он остаётся обычным переносом Panel в существующую Group, если Core принимает target.

#### 0.3. Что именно делает Core при Drop

- PanelTab → FreeArea: Float над узлом Panel; в Photoshop profile floating_root_for_node оборачивает Panel в Tabs, а затем в Column.
- GroupGrip → FreeArea: Float над существующим Tabs/Group; в Photoshop profile Tabs оборачивается в Column.
- DockAreaHeader → FreeArea: для Photoshop profile используется FloatMainAreaColumns. Горизонтальный Main Split(X) преобразуется в отдельные Core FloatingGroup по одной на каждую Column; возвращается упорядоченный список FloatingGroupId.
- Простая композиция 1 Column / 1 Group / 1 Tab распознаётся через simple_composition_root_for_drag_request. Для PanelTab, GroupGrip и DockAreaHeader Core использует Column как общий смысловой корень. При FreeArea создаётся Floating именно из Column, а не из случайного дочернего узла.
- PanelTab → Tabs/Group append: MovePanel; PanelTab не может передать целую Group через GroupTabs.
- GroupGrip → GroupTabs: MergeGroup.
- GroupGrip → GroupEdge: SplitGroup.
- GroupGrip → ColumnSlot/RailSlot: MoveGroup.
- PanelTab → собственная Top/Bottom-граница может отделить вкладку в соседнюю Group только если в исходной Group остаётся другая вкладка; иначе такой self-drop не должен создавать пустую Group.
- При Floating → Main эти же операции направлены в Main target resolver. После успешной мутации Panel/Group больше не принадлежат исходной Floating Composition; reconcile должен удалить только ставшие пустыми Floating-элементы.

#### 0.4. Что делает DragHostManager после отпускания мыши

1. finish_active_drag получает один DragDropRequest из pointer_up и вызывает Layout::resolve_drag_drop.
2. Core возвращает NoOp либо одну DockCommand. GPUI выполняет одну dispatch/dispatch_in; DragHostManager извлекает созданные FloatingGroupId из CommandValue.
3. Если Drop выполнен в Main, новая FloatingGroup не создаётся. Main получает обновлённый Layout, временный DragHost закрывается. Для Main-origin transfer с promote_on_float события LayoutChanged сначала удерживаются, затем публикуются перед закрытием временного окна.
4. Если Drop выполнен в FreeArea, создаётся новая Core FloatingGroup.
5. Если Drop выполнен на внешнюю сторону Floating и сработал chain_join, Core также создаёт новую FloatingGroup через FreeArea, после чего FloatingWindowManager добавляет её в WindowGrid.
6. Для Main-origin tab/group DragHost может быть принят FloatingWindowManager как persistent host, если восстановление native frame и adoption успешны. Иначе используется двухфазный handoff: DragHost перестаёт монтировать PanelView, целевой host должен отрисовать его, после чего временный DragHost освобождается.
7. Main → временный Floating → Main — это тот же путь, что и обычный Drop в Main: DragHost существует во время жеста, но после успешного Main Drop закрывается; постоянные Floating-окна не должны появляться.
8. Если операция отменена или не создала ожидаемую FloatingGroup для FreeArea/chain_join, DragHost закрывается, а исходный Layout должен остаться целостным.

#### 0.5. Что сейчас считается постоянным Floating-окном

- FloatingGroupId в DockCore обозначает одну Floating Composition/DockArea, а не отдельное визуальное окно Header или Group.
- DockWindowRole::Floating сейчас содержит один FloatingGroupId.
- FloatingWindowManager::reconcile собирает Core FloatingGroup, выбирает один PanelId в качестве Composition host и создаёт/использует окно для этого host. Остальные PanelWindow могут оставаться скрытыми.
- PanelWindowRegistry сейчас индексирует native entries через PanelId; CompositionHostRegistry выбирает host Panel для FloatingGroup.
- FloatingHost и FloatingWindowContent рисуют все Column/Group данной Floating Composition в одном HWND. CompositionHeader добавляется туда же условно через hide_header.
- WindowGridRegistry сейчас связывает FloatingGroupId. Поэтому один участник WindowGrid соответствует целой текущей Floating Composition, а не отдельной Header или Group.
- window_chain_geometry.rs располагает вертикальных соседей с overlap на высоту CompositionHeader: верхний HWND перекрывает шапку нижнего. Это правило старой архитектуры и должно быть заменено.
- В текущем WindowGrid уже есть проверенные алгоритмы направленных связей, порядка участников, native frame/client insets, DPI, Z-order, совместного перемещения, Resize и pruning. Их нужно адаптировать, а не переписывать без причины.

#### 0.6. Контракт новой модели после аудита

1. Core Composition → Column → Group → Tab остаётся единственной моделью содержимого. На этапе 0 Core не меняется.
2. Для постоянных Floating windows вводится отдельная GPUI identity:
   - Header(Column NodeId) — ровно одно окно шапки на Column;
   - Group(Group NodeId) — ровно одно окно на Group.
3. FloatingGroupId остаётся идентификатором Core Floating Composition/DockArea. Он связывает содержимое с Floating-контекстом, но не заменяет identity отдельных HWND.
4. PanelId остаётся идентификатором вкладки/PanelView и не должен выбирать единственное окно всей Composition.
5. DragHost остаётся временным окном транспорта. Он может быть принят как постоянное Group window после успешного Floating Drop, если это сохраняет существующий безопасный handoff; он не становится Header и не заменяет всю сетку.
6. Каждая Column имеет отдельный Header в зарезервированном верхнем ряду. Её Group windows стоят ниже в Core-порядке, без перекрытия высоты Header.
7. Добавление/перемещение Group меняет позиции окон Group её Column, но не позицию Header. Перетаскивание за Header перемещает Header вместе со всеми Group именно своей Column.
8. Main остаётся одним главным HWND и продолжает собирать Composition через composition.rs. Отдельные Header/Group HWND относятся только к постоянному Floating-представлению.
9. При простом Main Composition 1/1/1, перетаскиваемой за Header, переход в detached drag presentation должен сразу подготовить пару Header window + Group window. Если финальный Drop сделан в Floating, эта пара становится постоянной; если Drop вернулся в Main или жест отменён, временная пара освобождается без создания постоянной Floating-сетки.
10. Для обычного Main-origin PanelTab drag используется временный Group/DragHost во время жеста. Постоянный Header создаётся только при успешном Floating Drop. Если Drop вернулся в Main, постоянные Header/Group windows не создаются.

#### 0.7. Результат и границы этапа

Этап 0 завершён на уровне архитектурного аудита и документации. Следующий этап может вводить identity WindowGridMember, но не должен одновременно переписывать рендеринг, геометрию и Drop routing.

На этапе 0:
- код и Core Layout не изменялись;
- DropZone и существующая семантика DockCommand не менялись;
- сборка и тесты не запускались;
- Main → Main не входит в задачу;
- оптимизация откладывается до завершения функциональной переделки.

### Этап 1. Ввести идентичность нативных окон

<Статус: реализован в коде 16 ноября 2026 г.; локальные проверки ожидают пользователя.>

Файлы: `window_chain.rs`, `window_chain_geometry.rs`, `floating_manager.rs` и связанные unit-тесты.

Что сделано:

1. Введён типизированный `WindowGridMember`:
   - `FloatingComposition(FloatingGroupId)` — переходная идентичность для текущей архитектуры с одним HWND на Floating Composition;
   - `Header(Column NodeId)` — отдельный будущий HWND шапки Column;
   - `Group(Group NodeId)` — отдельный будущий HWND конкретной Group.
2. `WindowGridLink`, внутренний граф, индексы Registry и результаты pruning переведены с `FloatingGroupId` на `WindowGridMember`.
3. `retain_groups` обобщён и переименован в `retain_members`: очистка графа принимает множество живых оконных участников, а не только Core FloatingGroup.
4. Чистая геометрия и native Z-order используют `WindowGridMember`. Алгоритмы соседства, направлений, последовательных ветвей, расчёта frame/client bounds, DPI, Resize и Z-order сохранены; геометрическое поведение пока не менялось.
5. `FloatingWindowManager` использует один переходный адаптер `FloatingGroupId → FloatingComposition(FloatingGroupId)`. Все его текущие операции WindowGrid работают с типизированными ключами.
6. Для перехода обратно к Core-операциям добавлено явное преобразование bounds из членов WindowGrid в пары `(FloatingGroupId, Rect)`. Core Layout и его идентификаторы не менялись.
7. Текущий реестр HWND по-прежнему разрешает только `FloatingComposition` через `CompositionHostRegistry → PanelWindowRegistry`. `Header` и `Group` уже имеют независимую идентичность в графе, но их нативные host-окна ещё не создаются — это следующий этап.
8. Добавлен unit-тест, который помещает Header, Group и legacy Composition в один WindowGrid и проверяет, что это три разные идентичности.

Критерий этапа:

- WindowGrid больше не привязан к Core FloatingGroupId как к единственному типу участника.
- Текущие Floating-окна продолжают представляться переходным `FloatingComposition(id)`.
- Header и Group могут иметь собственные ключи без фиктивных Core FloatingGroupId.
- Все существующие алгоритмы графа и чистой геометрии работают с новым ключом.
- Рендеринг, создание HWND, DropZone и семантика Core-команд этим этапом не менялись.

Проверки сборки и тестов не запускались: их выполняет пользователь в своём локальном окружении.

### Этап 2. Разделить Floating-рендеринг на Header и Group

<Статус: root-host представления реализованы 16 ноября 2026 г.; локальные проверки ожидают пользователя.>

Файл: `crates/dock-gpui/src/components/floating.rs`.

Что сделано:

1. Добавлен типизированный `FloatingPartKind`: `Header { column }` или `Group { column, group }`.
2. Добавлен общий `FloatingPartHost` с отдельными конструкторами для Header и Group. Один host монтирует только один указанный Core-элемент.
3. Header-host получает геометрию одной Column и переиспользует `DockCompositionView::render_header` → `CompositionHeader`.
4. Group-host проверяет принадлежность Group указанной Column, переводит её геометрию в локальные координаты окна и переиспользует `DockGroupView::render_core` и `PanelHost`.
5. Для Group сохранены общий кеш PanelView, `PanelTransferPresentation` и контекстное меню. Переносимые вкладки исключаются из списка монтируемых PanelView так же, как в существующем FloatingHost.
6. Main не менялся и продолжает рендерить Composition через текущий `composition.rs`.
7. Добавлен unit-тест преобразования геометрии Group в локальные координаты отдельного окна.

Граница этапа на момент его завершения: `FloatingWindowManager` ещё использовал прежний полно-композиционный `FloatingHost`; создание постоянных Header/Group HWND и переключение реестра были оставлены этапу 3, описанному ниже. Это разделило подготовку рендереров и жизненный цикл нативных окон. Pointer overlay для активации вкладок, drag/resize и focus не копировался из старого `FloatingHost`; подключение этих обработчиков остаётся в этапах 5 и 6.

Критерий этапа 2: каждый Header-host способен отрисовать только шапку одной Column, а каждый Group-host — только одну Group и её активный PanelView в локальных координатах. Сквозное появление отдельных HWND для одной тестовой Column проверяется после подключения reconcile на этапе 3.

### Этап 3. Создание и согласование окон через reconcile

<Статус: реализация в коде завершена 16 ноября 2026 г.; ожидаются локальные проверки пользователя.>

Файлы: `floating_manager.rs`, `components/floating.rs`, `drag_host_manager.rs`, `input_drag.rs`.

Что сделано:

1. Постоянный реестр Floating окон получил отдельную таблицу `WindowGridMember → PanelWindowEntry`. Header и Group хранятся по своим Core `NodeId`; старые `PanelId`/`FloatingComposition` таблицы больше не выбирают постоянное окно для всей Floating Composition.
2. Добавлен сбор желаемых окон непосредственно из Core Layout: одна запись Header на каждую Floating Column и одна запись Group на каждую Group внутри Column.
3. `reconcile_part_windows` создаёт недостающие окна скрытыми, назначает Main native owner, устанавливает начальные bounds, сохраняет native scale factor, затем показывает окна, когда их PanelViews можно безопасно монтировать.
4. Существующие окна сохраняются по стабильному Core `NodeId`. Удалённые или недоступные HWND и исчезнувшие Column/Group удаляются из typed registry.
5. Повторный reconcile переиспользует уже зарегистрированные окна и синхронизирует только изменившиеся bounds/visibility.
6. `FloatingPartHost` подключён к создаваемым окнам. Group получает только собственную Core Group; Header — только свою Column. Main остаётся на прежнем `MainDockView` и общем `composition.rs`.
7. Удалён старый путь adoption, в котором временный DragHost становился постоянным окном целой Floating Composition. DragHost теперь остаётся транспортным окном, а PanelViews переходят в отдельные Group hosts через двухфазный handoff.
8. Handoff учитывает перенос Group, отдельной вкладки и DockArea, который может создать несколько FloatingGroups. Для закрытия DragHost собираются реальные активные PanelViews по затронутым Core Groups; он закрывается только после подтверждения кадра тех Group hosts, которым нужно смонтировать эти PanelViews.
9. Существующий общий кеш PanelView и `PanelTransferPresentation` переиспользуются. Переносимая панель не монтируется одновременно в исходном и целевом окнах.

Граница этапа: WindowGrid пока сохраняет переходную геометрию `FloatingComposition`, которая разрешается через один Group host. Полный граф Header/Group и их frame/client стыковка, включая отсутствие старого header overlap, относятся к этапу 4. Новые HWND пока не имеют подключённых пользовательских drag/resize/focus обработчиков; это следующие этапы.

Критерий реализации: Core Layout согласуется с одним Header HWND на Column и одним Group HWND на каждую Group; окна сохраняются между reconcile и удаляются вместе со своими Core узлами. Сквозную визуальную и поведенческую проверку выполняет пользователь локально.

Сборка и тесты не запускались: их выполняет пользователь в своём локальном окружении.

### Этап 4. Переработать геометрию WindowGrid

<Статус: базовая геометрия и typed WindowGrid реализованы 16 ноября 2026 г.>

Файлы: `window_chain.rs`, `window_chain_geometry.rs`, `floating_manager.rs`, `components/floating.rs` и тесты геометрии/топологии.

Реализовано:

- Удалён старый vertical overlap на высоту CompositionHeader. Вертикальные соседи стыкуются по client top/bottom с отдельными native insets и DPI каждого HWND.
- Добавлен общий перевод Core client geometry в native outer bounds. Учитываются физический origin исходного Floating окна, локальный логический origin Column/Group, client insets и DPI целевого HWND.
- `reconcile_part_windows` пересчитывает native bounds при изменении Core geometry, DPI и invalidation внешних bounds.
- WindowGrid перестраивается из реальных Core members: горизонтальная цепочка Header по Column и вертикальная цепочка Group под каждым Header.
- Внешние связи прежних `FloatingComposition` переносятся на граничные Header/Group members. Pending reconnect links и anchors тоже мигрируют на новые идентификаторы.
- Удаление Header/Group проходит через typed pruning; straight-through reconnect проверяется по реальным native bounds и frame geometry.
- Отложенное выравнивание читает bounds каждого HWND отдельно. В Core применяется только единый translation всей Floating Composition; несовместимые независимые смещения не записываются.
- Z-order строится по typed Header/Group графу. Добавлены тесты для дробного DPI, стыковки без overlap, транзакционной пересборки графа, двух колонок и переноса внешней связи между композициями.

Граница этапа: отдельные HWND пока намеренно созданы без native move/resize hooks. Перемещение Header вместе с Column stack подключается на этапе 5; согласованный Resize — на этапе 6. Геометрия и Core ratios уже поддерживают пересчёт после изменения Layout, поэтому новые Core-команды или параллельная модель размеров не требуются.

Критерий: Header/Group окна размещаются по Core-порядку, не перекрываются, сохраняют общую ширину Column и корректно стыкуются по client edges при дробном DPI.

### Этап 5. Подключить три маршрута переноса и временный DragHost

<Статус: частично реализован 16 ноября 2026 г.; общий pointer pipeline подключён к Floating Group HWND.>

Файлы: drag_host_manager.rs, input_drag.rs, floating_manager.rs, components/floating.rs и тесты маршрутизации.

Уже реализовано:

- Обработчики мыши FloatingHost вынесены в общий render_floating_pointer_input; legacy host продолжает использовать тот же pipeline.
- FloatingPartHost::Group использует общий pipeline для tab/group drag, capture, DragHost handoff, Escape и отмены при потере фокуса.
- Локальная позиция указателя Group HWND переводится в координаты Core Composition через origin соответствующей Group.
- DragSourceHost сохраняет точную идентичность Header/Group HWND. Координаты FreeArea и проверка окна под курсором используют именно исходный part-host.
- Drop между окнами, принадлежащими одной Floating Composition, остаётся Core Drop. WindowGrid-side insertion применяется только к другой Floating Composition.

Осталось:

- Подключить Header input и перемещение только своей Column stack; его нельзя направлять через старый whole-composition move.
- Завершить и локально проверить все три маршрута переноса, возврат Main → временный Floating → Main и особый drag простой Main Composition.
- После проверки отметить этап завершённым.

1. Main → Floating: использовать текущий DragHost; после принятого Drop создать/обновить Header и Group окна и выполнить handoff.
2. Floating → Floating: тот же DragHost и общий target resolver; обновить Core Layout и набор окон источника/цели.
3. Floating → Main: выполнить Core Drop в Main, затем освободить пустые Group/Header окна и перестроить оставшуюся колонку.
4. Main → временный Floating → Main: завершить Core Drop в Main, закрыть только временный DragHost; постоянные Floating окна не создавать.
5. Перетаскивание простой Main Composition за Header должно создавать Floating-представление из двух окон: Header + Group с исходной вкладкой.
6. Перетаскивание существующего Floating Header перемещает только его Column stack — Header и все Group этой колонки.
7. Сохранить уже работающую семантику DropZone и общий mapping направления WindowGrid. Не добавлять вторую реализацию hit-test или Drop routing только для Main-origin.
8. Переиспользовать существующий pointer pipeline `FloatingHost`: выделить общие части обработки мыши, capture и cancellation и подключить их к typed Header/Group hosts. Не копировать второй независимый набор mouse handlers.
9. Передавать в общий pipeline роль и NodeId текущего part-host; источник/цель переводить через существующие методы FloatingWindowManager.

Критерий: все три основных сценария и вариант возврата Main → временный Floating → Main проходят через один DragHost lifecycle и не теряют/не дублируют вкладки.

### Этап 6. Жизненный цикл, Z-order, Resize и восстановление

Файлы: floating_manager.rs, window_chain.rs, window_chain_geometry.rs, host-модули и unit-тесты.

1. При удалении Group удалять её окно и связи; при удалении последней Group удалять Header.
2. При закрытии Column удалять всю её оконную цепочку.
3. При смене состава Group сохранять Header, если Column не пуста.
4. Перестраивать Z-order по фактической топологии: Header сверху своей колонки, Group ниже; окна не должны закрывать друг друга из-за старого header overlap.
5. Перемещение Header синхронно перемещает все HWND его Column stack одним физическим dx/dy.
6. Resize сохраняет общую ширину Column и вертикальные стыки Group, не запускает циклическую синхронизацию.
7. Учитывать host loss, внешнее закрытие HWND, повторный reconcile, DPI, Escape, отмену drag и Undo/Redo.
8. Для Resize переиспользовать `DockCommand::ResizeGroups`, `DockCommand::ResizeSplitColumnBoundary`, `input_resize.rs` и `SplitterView`. Не вводить отдельную модель ratios для Floating.
9. Пока согласованный Resize нескольких HWND не подключён, не включать независимые native resize edges на Header/Group окнах.
10. Сохранение WindowGrid на диск не включать в этот этап.

Критерий: после любых перечисленных переходов нет пустых окон, stale-связей, дубликатов HWND и зависших временных DragHost.

### Этап 7. Удалить устаревшую модель и провести регрессию

1. Удалить Floating-путь, который рисует CompositionHeader и все Group внутри одного окна.
2. Удалить старые правила вертикального перекрытия CompositionHeader в геометрии WindowGrid.
3. Удалить PanelId-host selection, если он больше не нужен новому реестру окон.
4. Не удалять общие компоненты, рендереры и Core-операции, используемые Main.
5. Оптимизацию выполнять только после прохождения всех критериев приёмки.

## 6. Критерии приёмки

- [ ] Одна Floating Column с одной Group создаёт два HWND: Header и Group.
- [ ] Одна Floating Column с тремя Group создаёт четыре HWND: один Header и три Group.
- [ ] Две Floating Column имеют два окна Header в одном верхнем ряду.
- [ ] Перетаскивание за Header сдвигает Header и все Group только своей колонки.
- [ ] Вставка Group сверху не двигает Header; существующие Group сдвигаются вниз.
- [ ] Вставка Group между двумя Group оставляет Header на месте и правильно переставляет нижние окна.
- [ ] Вставка Group снизу добавляет окно в конец колонки.
- [ ] Добавление вкладки в TabStrip существующей Group не создаёт лишнюю Group или Header.
- [ ] Main → Floating через tab drag создаёт постоянные Header/Group окна после принятого Drop.
- [ ] Перетаскивание простой Main Composition за Header создаёт ровно два окна: Header и Group.
- [ ] Floating → Floating переносит содержимое без потери вкладок и удаляет опустевшие исходные окна.
- [ ] Floating → Main удаляет пустые исходные Group/Header окна, но сохраняет Header непустой колонки.
- [ ] Main → временный Floating → Main закрывает только DragHost и не создаёт постоянные Floating окна.
- [ ] Escape/отмена не оставляют новых окон или изменений Layout.
- [ ] Undo/Redo восстанавливают содержимое и правильный набор окон.
- [ ] Повторный reconcile не создаёт дубликаты.
- [ ] Перемещение/Resize не вызывает overlap, белых полос, мигания старой шапки или скачков bounds.
- [ ] Геометрия работает при разных scale factor и native frame/client inset.

## 7. Что не входит в эту переделку

- Прямой Main → Main drag/drop.
- Переписывание Core Layout или создание второй модели Composition.
- Изменение семантики уже работающих DropZone без отдельного подтверждённого дефекта.
- Персистентность WindowGrid.
- Оптимизация до завершения функциональных этапов.

## 8. Связь с предыдущим планом WindowGrid 2D

План docs/plan-peredelki-windowchain-2d.txt остаётся источником уже реализованных алгоритмов связности, направлений, native frame/DPI, перемещения, Resize, Z-order и pruning, которые можно сохранить после адаптации идентичности участников.

Для новой архитектуры **больше не действует** его прежнее правило: «нижняя CompositionHeader остаётся внутри FloatingHost и перекрывается HWND верхнего окна». Теперь Header — самостоятельное окно в отдельном верхнем ряду; Group окна располагаются ниже него без перекрытия.

Не переносить старую модель один-в-один: сначала заменить идентичность участника WindowGrid, затем адаптировать геометрию под Header/Group.

---

## Порядок работы

Работать строго по этапам. На каждом этапе менять только перечисленную область, переиспользовать существующие реализации и не запускать следующий этап до локальной проверки пользователем. Сборку и тесты выполняет пользователь в своём окружении.
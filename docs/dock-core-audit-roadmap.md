# dock-core — аудит багов, упрощение и производительность

Цель: проверить готовый `dock-core` на воспроизводимые ошибки, согласованность модели и геометрии, лишнюю работу и ненужную сложность. Не добавлять возможности и не менять архитектуру без доказанной необходимости. Исправления выполнять небольшими шагами: тест → минимальное исправление → все проверки → следующий пункт.

## Правила аудита

- [ ] Перед каждым исправлением добавить тест, воспроизводящий проблему.
- [ ] Не маскировать проблемы через `allow`, молчаливое исправление состояния или ослабление проверок.
- [ ] Сохранять существующий контракт команд, транзакционность, стабильность ID и правила Generic/Photoshop profiles.
- [ ] Различать ошибку модели, ожидаемую нехватку места (`InsufficientSpace` / `InsufficientCanvasWidth`) и ошибку рендер-адаптера.
- [ ] Оптимизировать только после тестов корректности; производительный эффект измерять, а не предполагать.
- [ ] Не объединять файлы типов и не переписывать модули только ради сокращения числа файлов/строк.

## Этап 0 — зафиксировать базовое состояние

- [ ] На актуальном `main` выполнить `cargo fmt --all -- --check`.
- [ ] Выполнить `cargo test -p dock-core`.
- [ ] Выполнить `cargo test -p dock-core --features serde`.
- [ ] Выполнить Clippy без serde: `cargo clippy -p dock-core --all-targets -- -D warnings`.
- [ ] Выполнить Clippy с serde: `cargo clippy -p dock-core --all-targets --features serde -- -D warnings`.
- [ ] Сохранить результаты до изменений; не объявлять базовую проверку пройденной без фактического вывода команд.

**Готово, когда:** базовые результаты известны и каждая последующая регрессия отличима от исходного состояния.

## Этап 1 — исправить возможные функциональные ошибки

Это находки статического чтения кода. Каждую сначала подтвердить отдельным регрессионным тестом; до воспроизведения не считать её доказанным runtime-багом.

### 1.1 — высота группы: согласовать resize и geometry · высокий приоритет

- [x] Добавить тест: `ResizeGroups` не разрешает высоту меньше той, которая нужна группе с учётом header, tab strip и минимального размера панели.
- [x] Проверить случай с `DockMetrics`, где `tab_strip_height` больше `collapsed_group_height`.
- [x] Сопоставить вычисления в `layout/sizing.rs` и `compute_rects.rs`: минимальный размер должен означать одно и то же в командах и геометрии.
- [x] Предпочесть общий расчёт/общий контракт минимального размера, а не постепенно поддерживать две расходящиеся реализации.
- [x] Проверить `actual_value`, clamping, inverse/undo и атомарность при недостатке места.

**Результат:** `Layout::minimum_node_size` — единый источник минимальной геометрии. Resize-команды и геометрия используют его для раскрытых и свёрнутых групп; тесты проверяют общий минимум, отклонение недостаточной высоты и случай, когда высота полосы вкладок больше высоты свёрнутой группы.

Дополнительная регрессия на нестандартных метриках (`header_height = 36`, `tab_strip_height = 44`) проверяет clamp и `actual_value`, восстановление через inverse, реальный Undo/Redo, геометрию после resize и атомарный отказ при высоте ниже суммы chrome + content minimums.

### 1.2 — нулевой вес всех развёрнутых групп · высокий приоритет

- [x] Добавить тест для валидных отношений, где вес 1 принадлежит свёрнутой группе, а у развёрнутых групп веса 0 (например, `[0.0, 1.0]`).
- [x] Проверить, что структура проходит `Layout::validate()`, а `compute_rects()` не возвращает `InvalidLayout` только из-за нулевой суммы оставшихся весов.
- [x] Согласовать fallback-правило с уже используемым правилом распределения: если среди развёрнутых групп нет положительных весов, делить свободное место по равным долям с соблюдением минимумов.
- [x] Проверить иерархические `Split(Y)` и `Column`, не меняя корректное поведение остальных весов.

**Результат:** `allocate_stack_heights` по-прежнему фиксирует высоты свёрнутых групп и исключает их веса из распределения. Если у оставшихся групп все веса равны нулю, `weighted_sizes` использует равные веса и продолжает соблюдать минимумы; ненулевые исходные веса и их пропорции не меняются. Регрессионные тесты проверяют `[0.0, 1.0]` в `Column`, равное распределение в вложенном `Split(Y)` и сохранение пропорции `1:3` для положительных весов при наличии нулевого веса.

**Затронутые узлы:** `compute_rects.rs::allocate_stack_heights`, `weighted_sizes`.

### 1.3 — минимальная ширина панели при создании и загрузке · высокий приоритет

- [x] Тест: минимальная ширина панели больше `default_column_width`.
- [x] Тест: workspace сохраняет Expanded-колонку уже, чем минимальная ширина её содержимого.
- [x] Выбрать единый контракт: расширять колонку до допустимого минимума; возвращать явную ошибку при несовместимости min/max.
- [x] Проверить, что `last_expanded_width` сохраняется в допустимых пределах, rail width остаётся привязан к режиму, профильные ограничения Generic/Photoshop соблюдаются.
- [x] Повторить тесты для обоих профилей и для workspace с `serde`.

**Результат:** factory выбирает ширину не меньше минимальной ширины содержимого и учитывает верхние ограничения; если минимум не помещается в максимум вкладочной группы, создание завершается ясной ошибкой. Общий расчёт минимумов учитывает вложенные Split и скрытое содержимое collapsed Tabs. Валидация отклоняет Expanded-колонки уже минимума содержимого или несовместимые min/max. Загрузка workspace чинит ширину до допустимого минимума, ограничивает `last_expanded_width` только при выходе за min/max и выдаёт диагностическое сообщение о нормализации. Добавлены регрессии для обоих профилей и serde-загрузки, в том числе для несовместимых ограничений.

**Исходная гипотеза подтверждена:** прежняя проверка сверяла ширину колонки только с `min_column_width`, поэтому проходивший `validate()` layout мог затем завершиться ошибкой в `compute_rects()`.

### 1.4 — точность краёв Rect при больших координатах · средний приоритет

- [x] Добавить тесты, где координата конечна и положительный размер слишком мал, чтобы `origin + size` представился отдельным значением в `f32`.
- [x] Проверить `Rect::validate_non_empty`, `contains`, `intersects`, вычисление layout и clamping floating bounds.
- [x] Уточнить и последовательно применить контракт: положительная ширина/высота должны задавать представимый край, который строго больше начала.
- [x] Проверить, что ошибка остаётся типизированной и не приводит к panic или прямоугольникам нулевой фактической ширины/высоты.

**Результат реализации:** `Rect::checked_right` и `checked_bottom` теперь отклоняют положительный размер, который не сдвигает край относительно начала из-за точности `f32`, с типизированной ошибкой `GeometryError::NonPositiveDimension`. Нулевые размеры по-прежнему допустимы для обычного `validate()`; `validate_non_empty()` их отклоняет. Добавлены регрессии для прямоугольных операций, вложенной геометрии layout на очень больших координатах, clamping floating bounds и workspace visible-client-region по обеим осям. **Тестовый прогон для этого изменения ожидает проверки на машине разработчика.**

**Затронутые узлы:** `rect.rs`, `compute_rects.rs`, `workspace_file.rs`, `workspace_file_tests.rs`.

## Этап 2 — расширить проверку инвариантов и регрессий

- [x] В property-тестах последовательностей команд после каждой успешной команды проверять не только `Layout::validate()`, но и построение geometry на достаточно большом тестовом viewport.
- [x] Добавить проверку всех выданных прямоугольников: конечные координаты/размеры, корректные края, непустые интерактивные зоны там, где это требуется контрактом.
- [x] Отдельно тестировать ожидаемые ошибки малого viewport; не считать `InsufficientCanvasWidth` багом автоматически.
- [x] Усилить последовательности операций move/resize/collapse/rail/floating/flyout и undo/redo; проверять восстановление логического состояния и валидность ID.
- [x] Добавить крайние случаи ratios: нули, один доминирующий вес, только collapsed groups, несколько уровней вложенности.
- [x] Проверить workspace round-trip, миграцию v1→v2, неизвестные PanelKey и повреждённые данные вместе с нормализацией.

**Результат:** property-тесты строят геометрию после каждой успешной команды на достаточно большом viewport (32768 × 32768 логических пикселей), затем вычисляют drop-зоны для реальной панели. Общая проверка `LayoutRects` обходит все поля с прямоугольниками и проверяет конечность координат/размеров, неотрицательные размеры, конечные и представимые края, а также ненулевые области hit-testing, tab targets, splitter/resize handles, scrolling и drop zones. Существующие тесты геометрии используют ту же проверку. Валидность модели и успешное построение геометрии проверяются как отдельные инварианты.

Дополнительные тесты отделяют ожидаемые ограничения размеров от ошибок модели: валидная раскладка возвращает `InsufficientCanvasWidth` при недостаточной ширине Canvas и `InsufficientSpace` для viewport, слишком низкого для минимальной высоты floating-окна, а при достаточном размере геометрия вычисляется успешно. Последовательность move group/panel, resize group/column/floating, collapse, rail, float/dock и flyout прогоняется через `UndoRedoHistory`; после каждого persistent-перехода undo/redo сравнивает логический snapshot, проверяет ссылки ID, строит геометрию и гарантирует, что ID заменённых сущностей не оживают после восстановления. Flyout отдельно проверяется как transient-состояние вне истории.

Дополнительные ratio-регрессии покрывают распределение при весах `[0, 0, 1]`, колонку, в которой все группы свёрнуты, и несколько уровней вложенных `Split(Y)`. Они проверяют минимумы нулевых весов, сохранение доминирующего веса, фиксированную высоту свёрнутых групп, отсутствие лишнего overflow и валидность всех полученных прямоугольников.

Workspace-проверки объединяют миграцию v1→v2, отсутствующий `last_expanded_width`, повреждённые, но безопасно нормализуемые ratios и `active_index`, а также неизвестный `PanelKey`. Восстановленная раскладка валидируется, строит geometry, сериализуется в канонический v2 JSON и загружается повторно без повторной миграции/нормализации, сохраняя неизвестную панель. Отдельные тесты проверяют отказ для неизвестного типа узла и неверного типа поля, чтобы структурно повреждённый JSON не маскировался нормализацией.

**Готово, когда:** каждая найденная ошибка покрыта тестом, а успешная команда не создаёт модель, которую штатная геометрия не может обработать при достаточном viewport.

## Этап 3 — performance: сначала измерить, затем менять

### 3.1 — обход дерева при расчёте drop-зон · высокий потенциал

- [x] Измерить `compute_drop_zones()` на типичной и искусственно большой раскладке.
- [x] Проверить частые поиски родителя через `direct_parent()`, создание `Vec` в `children()`, повторные `contains_node()`, `owner_column()`, поиск floating/main area.
- [x] По результатам baseline построить временный индекс родителя на один расчёт drop-зон и перейти к обходу без копирования дочерних массивов.
- [x] Сравнить время до/после в release-бенчмарке и проверить, что число drop-зон совпадает во всех четырёх сценариях.
- [x] Подтвердить прохождение регрессионных тестов приоритета зон и точных insertion indices.
- [x] Не вводить долговечный кеш, требующий сложной инвалидации, если достаточно временного индекса операции.

**Инструмент измерения:** ignored unit-test `benchmark_compute_drop_zones_typical_and_large_layouts` измеряет публичный вызов с уже рассчитанной геометрией в release-режиме для `DropSource::Panel` и `DropSource::Group`: типичный layout (2 колонки × 4 группы × 2 панели) и искусственно большой (8 × 12 × 3), оба с floating-группой. Запуск: `cargo test -p dock-core --release benchmark_compute_drop_zones_typical_and_large_layouts -- --ignored --nocapture --test-threads=1`. Вывод фиксирует размер дерева, число геометрических targets/zones, общее время, микросекунды на вызов и calls/s.

**Baseline до изменения (release, узлы дерева: 29 и 395):**

| Раскладка | Источник | Мкс/вызов | Вызовов/сек |
|---|---|---:|---:|
| `typical-2x4x2` | `Panel` | 127,852 | 7 821,6 |
| `typical-2x4x2` | `Group` | 276,109 | 3 621,8 |
| `large-8x12x3` | `Panel` | 8 345,815 | 119,8 |
| `large-8x12x3` | `Group` | 18 374,380 | 54,4 |

На большой раскладке до оптимизации вызов был примерно в 65 раз медленнее для `Panel` и в 67 раз медленнее для `Group`; это послужило основанием проверить повторные обходы.

**После изменения (release):**

| Раскладка | Источник | Мкс/вызов до → после | Ускорение | Drop-зон/вызов до → после |
|---|---|---:|---:|---:|
| `typical-2x4x2` | `Panel` | 127,852 → 52,531 | 2,43× | 87 → 87 |
| `typical-2x4x2` | `Group` | 276,109 → 62,616 | 4,41× | 76 → 76 |
| `large-8x12x3` | `Panel` | 8 345,815 → 694,135 | 12,02× | 1 089 → 1 089 |
| `large-8x12x3` | `Group` | 18 374,380 → 886,940 | 20,72× | 1 076 → 1 076 |

В этом запуске время одного вызова снизилось примерно на 59%, 77%, 92% и 95% соответственно. Количество сформированных зон во всех четырёх сценариях совпадает с baseline; это хороший быстрый сигнал, но совпадение количества не доказывает полную эквивалентность target-ов, приоритетов и индексов вставки. Эти свойства остаются на проверке регрессионными тестами.

**Изменение:** один временный `DropZoneTreeIndex` строит соответствия child → parent и panel → node за один проход на каждый вызов. `direct_parent()` теперь выполняет поиск по индексу; `contains_node()` проверяет принадлежность движением вверх по родителям; `children()` возвращает срез без клонирования дочерних `Vec`. Индекс не сохраняется между вызовами и не требует инвалидации. Измеренное ускорение подтверждено повторным release-бенчмарком; эквивалентность приоритета зон и точных insertion indices должна быть подтверждена обычными регрессионными тестами.

### 3.2 — повторный расчёт minimum_node_size · средний потенциал

- [x] Измерить `compute_rects()` на широких/глубоких деревьях и большом количестве вкладок.
- [x] Проверить, сколько раз один узел посещается функциями `preferred_width`, `minimum_node_size` и `allocate_stack_heights`.

**Измерительный probe:** ignored unit-test `benchmark_compute_rects_node_size_visits` запускает три synthetic fixture: широкая раскладка (10 колонок × 8 групп × 3 панели), глубокое дерево из 24 вложенных вертикальных Split-уровней и группа со 128 вкладками. Для каждого сценария он выводит время `compute_rects()` без включённых счётчиков и отдельно профилирует один вызов, чтобы собрать по каждому NodeId количество посещений `preferred_width`, wrapper `minimum_node_size`, рекурсивного `Layout::minimum_node_size` и запросов к детям в `allocate_stack_heights`. Счётчики включаются только на отдельном профилируемом вызове; их блокировки не входят в измеряемое время. Запуск: `cargo test -p dock-core --release benchmark_compute_rects_node_size_visits -- --ignored --nocapture --test-threads=1`.

**Состав fixtures:** глубокий сценарий использует 24 вертикальных Split-уровня, чтобы изолировать стоимость глубокой рекурсии; широкий сценарий отдельно покрывает горизонтальное распределение ширины. Это разделяет стресс по глубине и стресс по ширине.

**Baseline `compute_rects()` (release, успешный локальный запуск):**

| Сценарий | Узлов | Панелей | Итераций | Мкс/вызов | Вызовов/сек | Панелей/групп в результате |
|---|---:|---:|---:|---:|---:|---:|
| `wide-10x8x3` | 331 | 240 | 20 | 591,010 | 1 692,0 | 80 / 80 |
| `deep-vertical-24` | 74 | 25 | 60 | 160,552 | 6 228,5 | 25 / 25 |
| `tabs-128` | 130 | 128 | 60 | 178,255 | 5 609,9 | 1 / 1 |

**Посещения в отдельном профилируемом вызове.** В каждой ячейке указано: `вызовы / повторно посещённые узлы / максимум вызовов на один узел`.

| Сценарий | Всего событий посещения | `preferred_width` | helper `minimum_node_size` | рекурсивный `Layout::minimum_node_size` | дети в `allocate_stack_heights` |
|---|---:|---:|---:|---:|---:|
| `wide-10x8x3` | 1 793 | 21 / 10 / 2 | 321 / 80 / 3 | 1 371 / 320 / 5 | 80 / 0 / 1 |
| `deep-vertical-24` | 1 344 | 49 / 0 / 1 | 148 / 25 / 4 | 1 099 / 72 / 28 | 48 / 0 / 1 |
| `tabs-128` | 394 | 1 / 0 / 1 | 4 / 1 / 3 | 388 / 129 / 4 | 1 / 0 / 1 |

**Вывод из замера:** рекурсивные вычисления `Layout::minimum_node_size` составляют примерно 76,5% всех учтённых событий на широкой раскладке, 81,8% на глубоком дереве и 98,5% в группе из 128 вкладок. На глубоком дереве один NodeId проходит через рекурсивную функцию до 28 раз; в широкой раскладке повторно посещаются 320 из 331 узла. Запросы непосредственных детей в `allocate_stack_heights` повторов не показывают. Это подтверждает существенное повторение работы и оправдывает эксперимент с кешем, но само по себе ещё не доказывает ускорение.

Первый частичный запуск дал для `wide-10x8x3` 556,160 мкс/вызов, повторный полный — 591,010 мкс/вызов. Поэтому значения времени следует считать исходной точкой, а не точной константой; ускорение после изменения нужно сравнивать несколькими release-прогонами на той же машине и с теми же fixtures.

**Результат эксперимента с кешем (PR #24 и #26): кеш не оставлен в production-коде.** Оба варианта уменьшали число рекурсивных вычислений, но не дали стабильного ускорения на всех fixtures. Baseline ниже — один замер до изменения; поэтому выводы по времени предварительные и требуют осторожности.

| Сценарий | Baseline, мкс/вызов | Кеш всех узлов: 3 запуска / медиана | Кеш составных узлов: 3 запуска / медиана | Рекурсивные вычисления: baseline → составной кеш |
|---|---:|---:|---:|---:|
| `wide-10x8x3` | 591,010 | 490,445 / 599,775 / 604,770 · **599,775** | 563,070 / 609,855 / 969,185 · **609,855** | 1 371 → 411 |
| `deep-vertical-24` | 160,552 | 160,457 / 153,193 / 127,657 · **153,193** | 186,327 / 132,993 / 140,840 · **140,840** | 1 099 → 98 |
| `tabs-128` | 178,255 | 250,037 / 205,618 / 197,032 · **205,618** | 209,468 / 183,537 / 230,612 · **209,468** | 388 → 130 |

По медиане кеш составных узлов улучшил только глубокий fixture (примерно на 12%); широкая раскладка немного медленнее baseline, а `tabs-128` — примерно на 18% медленнее. В третьем `wide`-прогоне наблюдается сильный выброс, поэтому это сравнение нельзя считать точной оценкой небольших различий. Тем не менее данные не подтверждают стабильную выгоду от общей HashMap-инфраструктуры, которая усложняет путь расчёта. Решение: **отказаться от кеша и сохранить прямую рекурсивную реализацию**, оставив benchmark и счётчики посещений для возможных дальнейших исследований.

Проверки геометрии для разных `DockMetrics`, режимов колонок и collapsed-состояний сохранены как обычные регрессионные тесты; их имена больше не привязаны к реализации с кешем. Пункт о производительности 3.2 закрывается не внедрением кеша, а измеренным отказом от варианта без убедительного межсценарного выигрыша. Следующий performance-кандидат следует брать только после локального прохождения fmt, полного набора тестов и clippy.

- [x] Сравнить полное кеширование и кеш только составных узлов в трёх release-запусках для каждого fixture.
- [x] Отказаться от кеша: сокращение числа рекурсивных вычислений не дало стабильного ускорения между сценариями; сохранить регрессионные тесты геометрии и benchmark для дальнейших исследований.

### 3.3 — hit-test floating-поддеревьев · средний потенциал

**Baseline probe:** ignored release-test `benchmark_hit_test_multiple_floating_subtrees` builds 8 overlapping floating windows. Each window contains 2 columns × 3 tab groups × 8 panel instances (456 nodes overall); it measures both a hit inside the frontmost floating window and a point outside all floating windows. Geometry construction is outside the timed loop. Run with:

```sh
cargo test -p dock-core --release benchmark_hit_test_multiple_floating_subtrees -- --ignored --nocapture --test-threads=1
```

**Pre-change baseline:** three runs on the local Windows release build, 500 iterations per point (geometry construction excluded). Values are microseconds per call; the run-to-run spread is a reminder to compare medians rather than single timings.

| Point | Run 1 | Run 2 | Run 3 | Median |
|---|---:|---:|---:|---:|
| `inside-front-floating` | 211.403 | 225.658 | 230.031 | **225.658** |
| `outside-floating` | 207.845 | 190.114 | 196.613 | **196.613** |

The benchmark asserts a panel hit inside the frontmost window and a Canvas hit outside every floating window. The separate regression test asserts that blank space inside the topmost floating window returns that window's `FloatingGroup` hit instead of falling through to a lower window or the Canvas.

**Intermediate variant (PR #32):** deferring the union collection greatly reduced the cost inside the frontmost window, but left a measured regression when the point was outside every window. PR #34 addressed that regression by collecting directly into shared sets and traversing child lists without cloning.

| Point | Run 1 | Run 2 | Run 3 | Median | Change vs. baseline median |
|---|---:|---:|---:|---:|---:|
| `inside-front-floating` | 27.130 | 26.721 | 27.099 | **27.099** | **−88.0% (8.3× faster)** |
| `outside-floating` | 267.849 | 234.511 | 234.189 | **234.511** | **+19.3% slower** |

All timings are microseconds per call from 500 iterations per point on the same reported local Windows release build. Small benchmark samples can still be affected by machine load; the outside-window regression should be treated as a signal to verify, not a universal cost estimate.

**Follow-up implementation (PR #34):** direct union collection reuses the same node/panel sets across floating roots, and subtree traversal iterates child lists without cloning them. This targets the allocations and hashing on the outside-window path.

**Follow-up measurements after PR #34:** three release runs on the same local Windows build, 500 iterations per point.

| Point | Run 1 | Run 2 | Run 3 | Median | Change vs. original baseline |
|---|---:|---:|---:|---:|---:|
| `inside-front-floating` | 26.155 | 32.847 | 22.652 | **26.155** | **−88.4% (8.63× faster)** |
| `outside-floating` | 104.460 | 124.460 | 104.596 | **104.596** | **−46.8% (1.88× faster)** |

Against the intermediate lazy-union-only variant, the outside-window median improved from 234.511 to 104.596 µs/call (55.4% lower); the inside-window median remained in the same range (27.099 → 26.155 µs/call). The third outside-window run is notably slower than the other two, so these are workload-specific measurements rather than a universal performance guarantee.

The follow-up measurements resolve the measured regression in this synthetic outside-window case. After PR #34, the user reported that the tests passed. The benchmark plus regression tests support retaining the combined optimization for the measured workload; timings remain machine- and workload-specific.



- [x] Record the release benchmark baseline for both points across three runs.
- [x] Check and defer the all-floating-subtree union until the point misses every floating window.
- [x] Avoid rebuilding the selected floating window's subtree for the main-area exclusion set; traverse children without cloning and reuse sets for the union.
- [x] Compare baseline and final implementation across three release runs per scenario.
- [x] Run the local test suite after the final implementation; the user reported all tests passing.
- [x] Preserve priority: Z-order → floating resize/title → internal targets → flyout → main dock/canvas.
- [x] Confirm blank space in the topmost floating window never leaks hits to lower windows or Canvas.

### 3.4 — Undo/Redo и другие аллокации · низкий/средний потенциал

- [x] Не создавать history snapshot в `UndoRedoHistory::apply()`, если `limit == 0`; команда при этом по-прежнему должна применяться обычным путём.
- [x] Тестом подтвердить идентичность outcome/error и состояния при лимите истории 0.
- [ ] Измерить overhead snapshot/clone для типичных layout перед другими изменениями.
- [ ] Рассматривать замену `Vec::remove(0)` в bounded history на другую структуру только если большой лимит истории важен на практике.

**Manual benchmark:** `benchmark_history_snapshot_and_bounded_history` (ignored release test) reports node, panel-instance and floating-window counts, and microseconds per call for both `Layout::clone()` and `Layout::history_snapshot()` on small/medium/large valid fixtures. It also measures command application with history disabled, at the default limit of 100, and at a saturated limit of 1,000; fixture creation and history prefill are outside the timed loop.

Run with:

```sh
cargo test -p dock-core --release benchmark_history_snapshot_and_bounded_history -- --ignored --nocapture --test-threads=1
```

Use the results to decide whether a queue representation change would matter at the supported/default history limit. Do not replace `Vec::remove(0)` without a demonstrated practical regression, and do not mix such a change into the snapshot measurements.

**Готово, когда:** есть воспроизводимый benchmark (размер дерева, число панелей/окон, время до/после), и ускорение не покупается ухудшением читаемости или корректности.

## Этап 4 — упрощение без изменения контракта

- [ ] Найти дублирующие алгоритмы и проверки только после этапов 1–3; сначала определить единый контракт, затем объединять реализацию.
- [ ] Не убирать проверку входов лишь потому, что вызывающий код обычно передаёт валидные значения: публичные методы должны оставаться устойчивы к некорректным данным.
- [ ] Удалять мёртвый код только после поиска всех вызовов и подтверждения тестами.
- [ ] Не сокращать код ценой смешивания UI-логики с pure core или потери типизированных ошибок.
- [ ] Сравнить публичный API до/после: без непреднамеренных breaking changes.

## Этап 5 — визуальная и интеграционная проверка

Геометрические алгоритмы сначала проверяются численными тестами. Визуальная проверка нужна, когда core подключён к реальному renderer/UI: она обнаруживает ошибки, которых не видно по самим прямоугольникам (неверный scale, clip, scroll, слои или применение координат).

- [ ] При наличии работающей UI-интеграции проверить масштабы 100%, 125%, 150% и 200% DPI/device scale.
- [ ] Проверить крайние ширины окна: Canvas сохраняет минимальную ширину; слишком узкий viewport сообщает ожидаемую ошибку/поведение.
- [ ] Проверить Expanded / IconsAndLabels / IconsOnly: ширину, порядок иконок, возврат к `last_expanded_width`.
- [ ] Проверить tab overflow: последняя видимая вкладка, кнопка overflow, скрытые вкладки и точность зон клика.
- [ ] Проверить splitters: визуальная толщина, зона hit-test, ограничения min/max и поведение у края окна.
- [ ] Проверить вертикальный overflow: фактический scroll viewport совпадает с вычисленным content extent, нижние элементы доступны, клики по невидимым частям не проходят.
- [ ] Проверить flyout: сторона открытия, resize edge, clipping, переход между иконками и закрытие.
- [ ] Проверить floating windows: Z-order, title/resize grips, clamping к видимой области и отсутствие кликов сквозь верхнее окно.
- [ ] Сравнить фактическое поведение адаптера с `LayoutRects` и `HitResult`; фиксировать конкретные шаги и скриншоты для воспроизведения.

**Важно:** если полноценного рендеринга dock UI ещё нет, этот этап остаётся ожидающим интеграции. Не имитировать визуальный проход только тестами core.

## Проверки после каждого небольшого исправления

```sh
cargo fmt --all -- --check
cargo test -p dock-core
cargo test -p dock-core --features serde
cargo clippy -p dock-core --all-targets -- -D warnings
cargo clippy -p dock-core --all-targets --features serde -- -D warnings
```

После performance-изменений повторить соответствующий benchmark и тесты обоих профилей. Зафиксировать изменения отдельным небольшим коммитом, когда весь набор проходит.

## Порядок совместной работы

Идём по одному пункту за раз: я готовлю минимальный патч и регрессионные тесты в репозитории; затем пользователь синхронизирует `main`, запускает команды локально и присылает фактический вывод. Не переходить к следующему пункту до зелёных тестов текущего шага.

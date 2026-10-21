# Архитектура workspace

Стрелка означает «зависит от».

```text
app ──▶ dock-core
 │ ├──▶ dock-gpui ──▶ dock-core
 │ │         └─────▶ ui
 │ ├──▶ ui
 │ ├──▶ platform-api ◀── platform-windows (cfg(windows))
 │ └──▶ platform-windows (только при сборке Windows)
```

`app` также использует GPUI для запуска окна. `platform-windows` подключён только как target-specific dependency; прикладной код должен использовать контракт `platform-api`, а не типы WinAPI.

## Границы

- `dock-core` не зависит от GPUI, Windows API или `ui`; единственная публичная запись раскладки будет проходить через `Layout::apply(DockCommand)`.
- `Layout` владеет аренами узлов и панелей, dock-областями, внутренними floating-группами, Canvas, профилем и версией. Его поля закрыты; наружу доступны только неизменяемые представления.
- Изменение содержимого панели (например, прокрутка Layers) не входит в `dock-core`; `PanelState` хранит только layout-метаданные.
- `dock-gpui` отображает состояние ядра и переводит ввод в команды, но не хранит копию модели раскладки.
- `ui` содержит общий `Theme` и общие UI-примитивы; не зависит от `dock-core` и `dock-gpui`.
- `app` создаёт ровно один `Theme`, передаёт его представлениям и собирает `DockMetrics` из `ThemeMetrics`.
- Canvas остаётся защищённой областью вне dock. Toolbar, меню и Options bar не являются панелями docking.
- `platform-api` и `platform-windows` пока оставлены без фиктивных интерфейсов — это будет задача R8.1.

## Контракт workspace serialization

- Единственный публичный формат файла рабочего пространства — JSON. Текущий envelope содержит `format: "roadmap-docking-workspace"` и `version: 2`; version 1 мигрирует к версии 2; формат или версия должны проверяться до восстановления.
- Миграция v1→v2 восстанавливает отсутствующий `last_expanded_width` из `width`, только если колонка сохранена в `Expanded`; для rail-режимов прежнюю ширину нельзя вывести однозначно, поэтому она остаётся `None`. Любая миграция возвращает `SchemaMigrated` в отчёте загрузки.
- Workspace serialization API и JSON-зависимость включаются только явным feature `dock-core/serde`; по умолчанию feature выключен.
- Сериализуется DTO из `WorkspaceFile`, а не внутренний `Layout` и не отдельные arena-backed структуры. Дерево хранится рекурсивно: в файле нет runtime `NodeId`, `PanelId`, `DockAreaId` или `FloatingGroupId`; panel leaf ссылается на стабильный `PanelKey`.
- Сохраняются профиль, предпочтение видимости chrome, известные bounds Canvas, структура Split/Tabs/Column, отношения групп и их ratios, активный индекс и collapse state вкладок, режим/ширина колонки и последняя expanded width, layout-only размер панели, bounds и z-order floating-групп.
- `PanelConstraints` принадлежат app-owned registry и при загрузке разрешаются по `PanelKey`; они не копируются в файл. Загрузка создаёт новые SlotMap arenas и проверяет полученный `Layout` текущими метриками.
- Flyout selection, runtime version, session-only `LastDockLocation`, UI/scroll handles и panel content state в workspace JSON не входят. Содержимое панели и её scroll/selection state — отдельный app-owned канал.
- Загрузчик возвращает `WorkspaceLoadResult` с diagnostics. Неизвестный/удалённый `PanelKey` сохраняется как placeholder с безопасными layout constraints и предупреждением; приложение может показать недоступную панель, а её исходное место остаётся в workspace.
- Загрузчик применяет только однозначные исправления: ratios/структура, которые поддерживает нормализатор, и ширина колонки, однозначно заданная `ColumnMode`. Нарушение правил профиля, где нужно гадать или терять структуру, возвращается как `InvalidLayout` с конкретными `ValidationError`; тихого разрушительного ремонта нет.
- Для floating bounds приложение может передать в `WorkspaceLoadOptions` видимый client rectangle в логических px. Core ограничивает bounds этой областью и сообщает об изменении, но не сжимает содержимое ниже его минимального размера: если client region слишком мал, загрузка возвращает `FloatingBoundsInsufficientSpace`. Определение монитора, физического DPI и предпочтительной позиции окна остаётся в app/platform слое.

## Undo/Redo contract

- Session Undo/Redo входит в `dock-core` как `UndoRedoHistory`; история намеренно не сохраняется в JSON workspace для MVP.
- История ведётся через фасад `UndoRedoHistory::apply`. После успешной команды, увеличившей `Layout::version`, записывается checkpoint до изменения. Это session-only снимки всей модели, а не последовательность best-effort inverse commands: так отмена покрывает команды, для которых inverse ещё отсутствует, включая reorder/move, ratios, floating bounds и stacking.
- No-op (не изменивший версию), отклонённая команда и transient flyout-команда не создают запись. Drag preview не меняет Layout и в API истории не входит. После новой persistent-команды на разветвлении undo redo-ветка очищается.
- Undo/Redo восстанавливает точный persistent checkpoint, но не откатывает `Layout::version`: версия остаётся монотонной. Flyout при восстановлении закрывается; session-only flyout auto-collapse preference сохраняется текущим.
- История ограничена количеством записей (по умолчанию 100); `clear()` сбрасывает её без изменения раскладки. Ошибка проверки checkpoint при текущих metrics не меняет Layout и возвращает запись в стек.

## Ограничения реализации

- Один собственный `struct`/`enum` на файл. `lib.rs` и `mod.rs` содержат только объявления модулей и re-export.
- Значения геометрии — логические пиксели в `f32`. Округление до физических пикселей происходит на границе отрисовки.
- `dock-core` сообщает для вертикального overflow viewport и полный `content_extent`, но не прокручивает содержимое; будущий GPUI-адаптер обеспечивает column-level scrolling. Tab-strip overflow возвращает видимые заголовки, область overflow-кнопки и ID скрытых вкладок. Overflow-кнопка/список — явный UX-выбор проекта, не подтверждённая особенность Photoshop.
- Геометрия рассчитана на положительный viewport: нулевые размеры отклоняются. Если Canvas или split не помещаются по ширине, Core возвращает явную ошибку нехватки места вместо прямоугольников отрицательного размера. По вертикали Core сохраняет минимальные размеры содержимого и сообщает полный `content_extent`; GPUI-адаптер позже реализует клиппинг/прокрутку. Пустая раскладка оставляет Canvas на всю доступную область при соблюдении минимальной ширины Canvas.
- В `dock-core` запрещён `unsafe`; GPUI и платформенный код изолированы от ядра.

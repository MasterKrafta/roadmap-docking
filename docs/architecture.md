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

- `dock-core` не зависит от GPUI, Windows API или `ui`; единственная запись раскладки в дальнейшем будет проходить через `Layout::apply(DockCommand)`.
- `dock-gpui` отображает состояние ядра и переводит ввод в команды, но не хранит копию модели раскладки.
- `ui` содержит общий `Theme` и общие UI-примитивы; не зависит от `dock-core` и `dock-gpui`.
- `app` создаёт ровно один `Theme`, передаёт его представлениям и собирает `DockMetrics` из `ThemeMetrics`.
- Canvas остаётся защищённой областью вне dock. Toolbar, меню и Options bar не являются панелями docking.
- `platform-api` и `platform-windows` пока оставлены без фиктивных интерфейсов — это будет задача R8.1.

## Ограничения реализации

- Один собственный `struct`/`enum` на файл. `lib.rs` и `mod.rs` содержат только объявления модулей и re-export.
- Значения геометрии — логические пиксели в `f32`. Округление до физических пикселей происходит на границе отрисовки.
- В `dock-core` запрещён `unsafe`; GPUI и платформенный код изолированы от ядра.

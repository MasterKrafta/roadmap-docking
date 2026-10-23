# ADR 0001 — источник и версия GPUI

**Статус:** источник закреплён в manifest; воспроизводимая сборка ещё требует проверки R0.2.

- Репозиторий: <https://github.com/zed-industries/zed>
- Commit: `9cd14daad768fbfaeeac8789f515f9a5f67669a6`
- Commit URL: <https://github.com/zed-industries/zed/commit/9cd14daad768fbfaeeac8789f515f9a5f67669a6>
- Rust toolchain проекта: `1.99.1`.

В `Cargo.toml` crates `gpui` и `gpui_platform` используют один и тот же `rev`. Зависимости не ссылаются на `main` или на плавающую версию.

## Не закрыто

- `cargo fetch` и `cargo build` на чистой машине Windows 10;
- проверка, что upstream commit компилируется в целевой конфигурации и не требует недокументированных patch-файлов;
- повторное получение зависимостей с полным `Cargo.lock` и контроль сборки без обновления lockfile.

Если commit не проходит проверку на Windows 10, зафиксируйте найденную проблему и выберите другой точный commit либо release; не переходите на плавающую ветку.

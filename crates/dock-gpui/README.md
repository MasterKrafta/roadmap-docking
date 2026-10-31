# dock-gpui

GPUI presentation and input adapter for the UI-independent `dock-core` layout engine.

## Module boundaries

- `DockHost` owns the root composition and renders stateless presentation components.
- `DockController` owns the only `dock_core::Layout`, undo/redo history, metrics, pointer gesture state, and the derived `LayoutRects` cache.
- `geometry.rs` is the GPUI/logical-pixel coordinate boundary. The controller's cache and hit-test methods live in `controller/layout_cache.rs`.
- `input/` translates pointer, drag, resize, and scroll input into Core hits and semantic commands.
- `actions.rs` is the application-facing command dispatch boundary; keyboard action bindings are not wired yet.
- `registry.rs` contains panel descriptors and GPUI view factories, not docking state.
- `persistence.rs` owns workspace JSON and file I/O helpers.
- `components/` contains stateless views for the Core geometry snapshot.
- `theme.rs` currently adapts the shared application UI theme under the stable `DockTheme` name.

## Data flow

GPUI input is converted to host-local logical pixels, resolved through Core hit testing, and translated into a `DockCommand` or a Core drag/resize interaction. The controller applies commands through `UndoRedoHistory`; Core remains the authority for layout mutations and constraints. A changed cache key triggers `Layout::compute_rects`, after which `DockHost` renders the resulting geometry through presentation components.

## Current scope

The present module layout is an incremental alignment with the planned architecture. Keyboard shortcuts, a separate dock-owned visual theme model, application examples, and image-based integration snapshots remain to be implemented with their actual behavior rather than empty placeholder modules.

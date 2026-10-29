//! Native-window lifecycle and docking behavior for next_dock on Windows.

use std::time::Duration;

use gpui::{
    App, AppContext, Context, WindowBounds, WindowKind, WindowOptions, point, px, size,
};
use platform::windows::NativeWindowHandle;

use super::floating_window::FloatingCompositionWindow;
use super::geometry;
use super::model::{DockNode, SplitAxis};
use super::window::{CompositionDropZone, NextDockWindow, COMPOSITION_DROP_ZONE_WIDTH};

impl NextDockWindow {
    pub(super) fn start_native_drag_polling(&mut self, cx: &mut Context<Self>) {
        if self.native_drag_polling {
            return;
        }
        self.native_drag_polling = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(4))
                    .await;
                if !matches!(
                    this.update(cx, |main, cx| main.tick_native_drag_polling(cx)),
                    Ok(true)
                ) {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn dock_floating_composition(
        &mut self,
        id: u64,
        drop_zone: CompositionDropZone,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(floating) = self
            .floating_compositions
            .iter()
            .find(|floating| floating.id == id)
            .copied()
        else {
            return false;
        };

        let source_index = floating.composition_index;
        let empty_before = matches!(drop_zone, CompositionDropZone::RightEdge);

        if self
            .container
            .reserve_empty_neighbor(source_index, empty_before)
        {
            self.floating_compositions
                .retain(|candidate| candidate.id != id);
            self.clear_native_move_state();
            cx.notify();
            return true;
        }

        // If there is no reserved empty composition slot, merge the floating
        // tree beside the first docked tree rather than dropping its contents.
        // Prefer a docked composition. If every remaining composition is
        // floating, merge into another floating layout and make it docked too.
        let target_index = self
            .container
            .slots
            .iter()
            .enumerate()
            .find(|(index, slot)| {
                *index != source_index
                    && slot.layout.is_some()
                    && !self
                        .floating_compositions
                        .iter()
                        .any(|candidate| candidate.composition_index == *index)
            })
            .map(|(index, _)| index)
            .or_else(|| {
                self.container
                    .slots
                    .iter()
                    .enumerate()
                    .find(|(index, slot)| *index != source_index && slot.layout.is_some())
                    .map(|(index, _)| index)
            });

        let Some(target_index) = target_index else {
            return false;
        };
        let target_was_floating = self
            .floating_compositions
            .iter()
            .any(|candidate| candidate.composition_index == target_index);

        let (target_slot, source_slot) = if source_index < target_index {
            let (before, after) = self.container.slots.split_at_mut(target_index);
            (&mut after[0].layout, &mut before[source_index].layout)
        } else {
            let (before, after) = self.container.slots.split_at_mut(source_index);
            (&mut before[target_index].layout, &mut after[0].layout)
        };

        let Some(source_layout) = source_slot.take() else {
            return false;
        };
        let Some(mut target_layout) = target_slot.take() else {
            *source_slot = Some(source_layout);
            return false;
        };

        let floating_root = source_layout.root;
        let docked_root = target_layout.root;
        target_layout.root = if empty_before {
            DockNode::Split {
                axis: SplitAxis::Horizontal,
                weights: vec![0.7, 0.3],
                children: vec![docked_root, floating_root],
            }
        } else {
            DockNode::Split {
                axis: SplitAxis::Horizontal,
                weights: vec![0.3, 0.7],
                children: vec![floating_root, docked_root],
            }
        };
        *target_slot = Some(target_layout);

        self.container.slots.remove(source_index);
        self.floating_compositions.retain_mut(|candidate| {
            if candidate.id == id
                || (target_was_floating && candidate.composition_index == target_index)
            {
                false
            } else {
                if candidate.composition_index > source_index {
                    candidate.composition_index -= 1;
                }
                true
            }
        });
        self.clear_native_move_state();
        cx.notify();
        true
    }

    pub(super) fn update_native_floating_drop_zone(
        &mut self,
        cursor: (i32, i32),
        cx: &mut Context<Self>,
    ) {
        let next_drop_zone = self.native_drop_zone(cursor);
        if self.composition_drop_zone != next_drop_zone {
            self.composition_drop_zone = next_drop_zone;
            cx.notify();
        }
    }

    pub(super) fn finish_native_floating_drag(
        &mut self,
        id: u64,
        cursor: (i32, i32),
        cx: &mut Context<Self>,
    ) {
        let drop_zone = self.native_drop_zone(cursor);
        self.clear_native_move_state();
        if let Some(drop_zone) = drop_zone {
            if self.dock_floating_composition(id, drop_zone, cx) {
                return;
            }
        }
        cx.notify();
    }

    fn native_drop_zone(&self, cursor: (i32, i32)) -> Option<CompositionDropZone> {
        let main_handle = self.native_main_handle?;
        let (client_x, client_y) = platform::windows::client_origin(main_handle)?;
        let scale = platform::windows::scale_factor(main_handle).max(0.1);
        let width = (self.native_main_viewport.width * scale).round() as i32;
        let height = (self.native_main_viewport.height * scale).round() as i32;
        let content_top = client_y + (geometry::HEADER_HEIGHT * scale).round() as i32;
        let content_bottom = client_y + height;
        let content_right = client_x + width;

        if cursor.0 < client_x
            || cursor.0 >= content_right
            || cursor.1 < content_top
            || cursor.1 >= content_bottom
        {
            return None;
        }

        let edge_width = (COMPOSITION_DROP_ZONE_WIDTH * scale).round() as i32;
        if cursor.0 - client_x <= edge_width {
            Some(CompositionDropZone::LeftEdge)
        } else if content_right - cursor.0 <= edge_width {
            Some(CompositionDropZone::RightEdge)
        } else {
            None
        }
    }
}

pub(super) fn synchronize_floating_windows(
    owner: gpui::WeakEntity<NextDockWindow>,
    main_handle: NativeWindowHandle,
    app: &mut App,
) {
    let Some(owner_entity) = owner.upgrade() else {
        return;
    };

    let (specs, existing) = owner_entity.update(app, |main, _cx| {
        (
            main.floating_compositions.clone(),
            main.native_floating_windows
                .iter()
                .map(|(id, handle)| (*id, *handle))
                .collect::<Vec<_>>(),
        )
    });

    // Return no-longer-used windows to the hidden pool instead of destroying
    // them, so the next tear-off does not have to create a new GPUI window.
    for (id, handle) in existing {
        if specs.iter().any(|floating| floating.id == id) {
            continue;
        }

        let reusable = match handle.update(app, |view, window, cx| {
            view.set_composition_id(None, cx);
            if let Some(native_handle) = NativeWindowHandle::from_window(window) {
                platform::windows::hide_window(native_handle);
            }
        }) {
            Ok(()) => true,
            Err(error) => {
                tracing::debug!(
                    composition_id = id,
                    error = %error,
                    "could not recycle stale native floating composition"
                );
                false
            }
        };

        owner_entity.update(app, |main, cx| {
            main.native_floating_windows.remove(&id);
            if reusable {
                main.native_floating_pool.push(handle);
            }
            cx.notify();
        });
    }

    for floating in specs {
        if owner_entity
            .read(app)
            .native_floating_windows
            .contains_key(&floating.id)
        {
            continue;
        }

        ensure_hidden_floating_window(owner.clone(), main_handle, app);
        let Some(handle) =
            owner_entity.update(app, |main, _cx| main.native_floating_pool.pop())
        else {
            tracing::debug!(composition_id = floating.id, "no prepared floating surface is available");
            continue;
        };

        let Some((client_x, client_y)) = platform::windows::client_origin(main_handle) else {
            owner_entity.update(app, |main, _cx| main.native_floating_pool.push(handle));
            tracing::debug!("could not determine the main window client origin");
            continue;
        };
        let scale = platform::windows::scale_factor(main_handle).max(0.1);
        let left = client_x + (floating.bounds.origin.x * scale).round() as i32;
        let top = client_y
            + ((geometry::HEADER_HEIGHT + floating.bounds.origin.y) * scale).round() as i32;
        let width = (floating.bounds.size.width * scale)
            .round()
            .max((180.0 * scale).round()) as i32;
        let height = (floating.bounds.size.height * scale)
            .round()
            .max((120.0 * scale).round()) as i32;
        let start_move = platform::windows::left_button_down();
        let id = floating.id;

        match handle.update(app, |view, window, cx| {
            view.set_composition_id(Some(id), cx);

            if let Some(native_handle) = NativeWindowHandle::from_window(window) {
                platform::windows::move_window(native_handle, left, top);
                platform::windows::resize_window(native_handle, width, height);
                platform::windows::show_window(native_handle);
            }

            if start_move && platform::windows::left_button_down() {
                view.begin_move(window, cx);
            }
        }) {
            Ok(()) => {
                owner_entity.update(app, |main, cx| {
                    main.native_floating_windows.insert(id, handle);
                    cx.notify();
                });
            }
            Err(error) => {
                tracing::debug!(
                    composition_id = id,
                    error = %error,
                    "could not activate a prepared floating surface"
                );
            }
        }
    }

    // Keep one hidden surface ready even when none are currently floating.
    ensure_hidden_floating_window(owner, main_handle, app);
}

fn ensure_hidden_floating_window(
    owner: gpui::WeakEntity<NextDockWindow>,
    main_handle: NativeWindowHandle,
    app: &mut App,
) {
    let Some(owner_entity) = owner.upgrade() else {
        return;
    };
    if !owner_entity.read(app).native_floating_pool.is_empty() {
        return;
    }

    let Some((client_x, client_y)) = platform::windows::client_origin(main_handle) else {
        tracing::debug!("could not determine the main window client origin for a prepared surface");
        return;
    };
    let scale = platform::windows::scale_factor(main_handle).max(0.1);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(gpui::Bounds {
            origin: point(px(client_x as f32 / scale), px(client_y as f32 / scale)),
            size: size(px(480.0), px(320.0)),
        })),
        titlebar: None,
        kind: WindowKind::PopUp,
        show: false,
        focus: false,
        is_resizable: true,
        is_minimizable: false,
        is_movable: false,
        window_min_size: Some(size(px(180.0), px(120.0))),
        ..Default::default()
    };
    let owner_for_view = owner.clone();
    let owner_for_close = owner.clone();

    match app.open_window(options, move |window, cx| {
        if let Some(native_handle) = NativeWindowHandle::from_window(window) {
            platform::windows::set_owner(native_handle, main_handle);
            platform::windows::enable_native_resize(native_handle);
            platform::windows::hide_window(native_handle);
        }

        let view = cx.new(|_| FloatingCompositionWindow::new(owner_for_view.clone(), None));
        let view_for_close = view.downgrade();
        view.update(cx, |_, view_cx| {
            window.on_window_should_close(view_cx, move |_window, app| {
                let Some(view) = view_for_close.upgrade() else {
                    return true;
                };
                let Some(id) = view.read(app).composition_id() else {
                    return true;
                };
                let Some(owner) = owner_for_close.upgrade() else {
                    return true;
                };

                owner.update(app, |main, cx| {
                    main.dock_floating_composition(id, CompositionDropZone::RightEdge, cx);
                });

                // Keep the hidden surface alive; the next sync hides and pools it.
                false
            });
        });
        view
    }) {
        Ok(handle) => {
            owner_entity.update(app, |main, cx| {
                main.native_floating_pool.push(handle);
                cx.notify();
            });
        }
        Err(error) => {
            tracing::warn!(error = %error, "could not prepare a hidden floating surface");
        }
    }
}

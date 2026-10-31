#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::ops::Range;

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, rgb, rgba, size, white, App,
    AnyView, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, KeyBinding, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    ShapedLine, SharedString, Style, TextAlign, TextRun, UTF16Selection, Window, WindowBounds,
    WindowKind, WindowOptions,
};
use gpui_platform::application;

actions!(
    transfer_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy,
    ]
);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PanelLocation {
    #[default]
    Main,
    Floating,
}

impl PanelLocation {
    fn label(self) -> &'static str {
        match self {
            Self::Main => "главное окно",
            Self::Floating => "плавающее окно",
        }
    }
}

#[derive(Default)]
struct TransferState {
    location: PanelLocation,
    main_hwnd: isize,
    floating_hwnd: isize,
}

struct TransferPanel {
    state: Entity<TransferState>,
    focus_handle: FocusHandle,
    content: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    click_count: u64,
    last_reported_location: Option<PanelLocation>,
}

impl TransferPanel {
    fn new(state: Entity<TransferState>, cx: &mut Context<Self>) -> Self {
        let content: SharedString = "Проверь перенос: набери текст здесь".into();
        let end = content.len();

        Self {
            state,
            focus_handle: cx.focus_handle(),
            content,
            selected_range: end..end,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            click_count: 0,
            last_reported_location: None,
        }
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut byte_offset = 0;
        let mut utf16_offset = 0;

        for ch in self.content.chars() {
            if utf16_offset >= offset {
                break;
            }

            utf16_offset += ch.len_utf16();
            byte_offset += ch.len_utf8();
        }

        byte_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut byte_offset = 0;

        for ch in self.content.chars() {
            if byte_offset >= offset {
                break;
            }

            byte_offset += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }

        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content[..offset]
            .char_indices()
            .next_back()
            .map(|(index, _)| index)
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content[offset..]
            .chars()
            .next()
            .map(|ch| offset + ch.len_utf8())
            .unwrap_or(self.content.len())
    }

    fn move_to(&mut self, offset: usize, window: &Window, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        window.notify_text_selection_changed(&self.focus_handle);
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, window: &Window, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }

        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }

        window.notify_text_selection_changed(&self.focus_handle);
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let content = format!(
            "{}{}{}",
            &self.content[..range.start],
            new_text,
            &self.content[range.end..]
        );

        self.content = content.into();
        let cursor = range.start + new_text.len();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.marked_range = None;

        window.notify_text_input_changed(&self.focus_handle);
        window.notify_text_selection_changed(&self.focus_handle);
        cx.notify();
    }

    fn replace_current_selection(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self
            .marked_range
            .clone()
            .unwrap_or_else(|| self.selected_range.clone());

        self.replace_text_in_range(range, text, window, cx);
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(line)) = (&self.last_bounds, &self.last_layout) else {
            return self.content.len();
        };

        if position.x <= bounds.left() {
            return 0;
        }

        if position.x >= bounds.right() {
            return self.content.len();
        }

        line.closest_index_for_x(position.x - bounds.left())
            .min(self.content.len())
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        window.request_virtual_keyboard();

        self.is_selecting = true;
        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), window, cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), window, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), window, cx);
        }
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let cursor = self.cursor_offset();
            let previous = self.previous_boundary(cursor);
            if previous == cursor {
                return;
            }
            self.select_to(previous, window, cx);
        }

        self.replace_current_selection("", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let cursor = self.cursor_offset();
            let next = self.next_boundary(cursor);
            if next == cursor {
                return;
            }
            self.select_to(next, window, cx);
        }

        self.replace_current_selection("", window, cx);
    }

    fn left(&mut self, _: &Left, window: &mut Window, cx: &mut Context<Self>) {
        let offset = if self.selected_range.is_empty() {
            self.previous_boundary(self.cursor_offset())
        } else {
            self.selected_range.start
        };
        self.move_to(offset, window, cx);
    }

    fn right(&mut self, _: &Right, window: &mut Window, cx: &mut Context<Self>) {
        let offset = if self.selected_range.is_empty() {
            self.next_boundary(self.cursor_offset())
        } else {
            self.selected_range.end
        };
        self.move_to(offset, window, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), window, cx);
    }

    fn select_right(&mut self, _: &SelectRight, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), window, cx);
    }

    fn select_all(&mut self, _: &SelectAll, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, window, cx);
        self.select_to(self.content.len(), window, cx);
    }

    fn home(&mut self, _: &Home, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, window, cx);
    }

    fn end(&mut self, _: &End, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), window, cx);
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_current_selection(&text.replace('\n', " "), window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_current_selection("", window, cx);
        }
    }

    fn increment_counter(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.click_count += 1;
        cx.notify();
    }
}

impl EntityInputHandler for TransferPanel {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_range = self.range_from_utf16(&range_utf16);
        self.selection_reversed = false;
        window.notify_text_selection_changed(&self.focus_handle);
        cx.notify();
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.content.encode_utf16().count())
    }

    fn marked_text_range(
        &self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        window.notify_text_input_changed(&self.focus_handle);
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());

        self.replace_text_in_range(range, new_text, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());

        let content = format!(
            "{}{}{}",
            &self.content[..range.start],
            new_text,
            &self.content[range.end..]
        );
        self.content = content.into();

        if new_text.is_empty() {
            self.marked_range = None;
        } else {
            self.marked_range = Some(range.start..range.start + new_text.len());
        }

        let insertion_start_utf16 = self.offset_to_utf16(range.start);
        let inserted_length_utf16 = new_text.encode_utf16().count();
        self.selected_range = new_selected_range_utf16
            .map(|selected| {
                self.range_from_utf16(
                    &(insertion_start_utf16 + selected.start.min(inserted_length_utf16)
                        ..insertion_start_utf16 + selected.end.min(inserted_length_utf16)),
                )
            })
            .unwrap_or_else(|| {
                let cursor = range.start + new_text.len();
                cursor..cursor
            });
        self.selection_reversed = false;

        window.notify_text_input_changed(&self.focus_handle);
        window.notify_text_selection_changed(&self.focus_handle);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.last_layout.as_ref()?;
        if line.text != self.content {
            return None;
        }

        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(
                element_bounds.left() + line.x_for_index(range.start),
                element_bounds.top(),
            ),
            point(
                element_bounds.left() + line.x_for_index(range.end),
                element_bounds.bottom(),
            ),
        ))
    }

    fn selection_bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Bounds<Pixels>> {
        self.bounds_for_range(range_utf16, element_bounds, window, cx)
            .into_iter()
            .collect()
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        let local_point = bounds.localize(&point)?;
        let line = self.last_layout.as_ref()?;
        if line.text != self.content {
            return None;
        }

        Some(self.offset_to_utf16(line.closest_index_for_x(local_point.x)))
    }
}

impl Focusable for TransferPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

struct TextElement {
    input: Entity<TransferPanel>,
}

struct TextPrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = TextPrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let is_placeholder = input.content.is_empty();
        let display_text: SharedString = if is_placeholder {
            "Введите текст; переносите панель между окнами".into()
        } else {
            input.content.clone()
        };
        let selected_range = input.selected_range.clone();
        let cursor_offset = input.cursor_offset();
        let style = window.text_style();
        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: if is_placeholder {
                rgb(0x888888)
            } else {
                style.color
            },
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &[run], None);

        let cursor_x = line.x_for_index(cursor_offset);
        let (selection, cursor) = if selected_range.is_empty() || is_placeholder {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + cursor_x, bounds.top()),
                        size(px(2.), bounds.bottom() - bounds.top()),
                    ),
                    rgb(0x4a90e2),
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + line.x_for_index(selected_range.start),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + line.x_for_index(selected_range.end),
                            bounds.bottom(),
                        ),
                    ),
                    rgba(0x334a90e2),
                )),
                None,
            )
        };

        TextPrepaintState {
            line: Some(line),
            cursor,
            selection,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );

        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection);
        }

        let line = prepaint.line.take().expect("text line was prepainted");
        line.paint(
            bounds.origin,
            window.line_height(),
            TextAlign::Left,
            None,
            window,
            cx,
        )
        .expect("text line should paint");

        if focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        self.input.update(cx, |input, _| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
        });
    }
}

impl Render for TransferPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let location = self.state.read(cx).location;
        let focused = self.focus_handle.is_focused(window);
        let entity_id = cx.entity().entity_id();

        if self.last_reported_location != Some(location) {
            eprintln!(
                "[AnyView transfer] Entity {:?} is now rendered in {} (focused={})",
                entity_id,
                location.label(),
                focused
            );
            self.last_reported_location = Some(location);
        }

        let focus_handle = self.focus_handle.clone();
        let cursor = self.cursor_offset();
        let selection = self.selected_range.clone();
        let content_len = self.content.len();
        let click_count = self.click_count;
        let entity_label = format!("{:?}", entity_id);

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .bg(rgb(0x292d32))
            .text_color(rgb(0xe5e7eb))
            .child("Одна и та же Entity / AnyView")
            .child(format!("EntityId: {entity_label}"))
            .child(format!("Текущий host: {}", location.label()))
            .child(format!(
                "Фокус в этом окне: {}",
                if focused { "ДА" } else { "нет" }
            ))
            .child(format!(
                "Состояние курсора: byte {cursor}/{content_len}, выделение {selection:?}"
            ))
            .child(
                div()
                    .w_full()
                    .track_focus(&focus_handle)
                    .key_context("TransferPanelInput")
                    .cursor(CursorStyle::IBeam)
                    .on_action(cx.listener(Self::backspace))
                    .on_action(cx.listener(Self::delete))
                    .on_action(cx.listener(Self::left))
                    .on_action(cx.listener(Self::right))
                    .on_action(cx.listener(Self::select_left))
                    .on_action(cx.listener(Self::select_right))
                    .on_action(cx.listener(Self::select_all))
                    .on_action(cx.listener(Self::home))
                    .on_action(cx.listener(Self::end))
                    .on_action(cx.listener(Self::paste))
                    .on_action(cx.listener(Self::copy))
                    .on_action(cx.listener(Self::cut))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
                    .on_mouse_move(cx.listener(Self::on_mouse_move))
                    .border_1()
                    .border_color(rgb(0x687383))
                    .bg(rgb(0x171a1f))
                    .p_2()
                    .child(TextElement {
                        input: cx.entity(),
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(format!("Счётчик внутри панели: {click_count}"))
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .bg(rgb(0x43566d))
                            .cursor_pointer()
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(Self::increment_counter),
                            )
                            .child("Увеличить"),
                    ),
            )
    }
}

struct MainWindowView {
    state: Entity<TransferState>,
    panel: AnyView,
}

impl MainWindowView {
    fn move_to_floating(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let hwnd = self.state.update(cx, |state, cx| {
            state.location = PanelLocation::Floating;
            cx.notify();
            state.floating_hwnd
        });

        if hwnd != 0 {
            tearoff_win::show_window(hwnd);
        }
    }
}

impl Render for MainWindowView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel_is_here = self.state.read(cx).location == PanelLocation::Main;
        let content = if panel_is_here {
            div().flex_1().p_3().child(self.panel.clone())
        } else {
            div()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(rgb(0x888888))
                .child("Панель находится в отдельном floating-окне")
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1b1e22))
            .child(
                div()
                    .h(px(42.))
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .bg(rgb(0x30363d))
                    .text_color(white())
                    .child("MAIN WINDOW")
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .bg(rgb(0x43566d))
                            .cursor_pointer()
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(Self::move_to_floating),
                            )
                            .child("Перенести панель во floating"),
                    ),
            )
            .child(content)
    }
}

struct FloatingWindowView {
    state: Entity<TransferState>,
    panel: AnyView,
}

impl FloatingWindowView {
    fn move_to_main(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (floating_hwnd, main_hwnd) = self.state.update(cx, |state, cx| {
            state.location = PanelLocation::Main;
            cx.notify();
            (state.floating_hwnd, state.main_hwnd)
        });

        if floating_hwnd != 0 {
            tearoff_win::hide_window(floating_hwnd);
        }
        if main_hwnd != 0 {
            tearoff_win::activate_window(main_hwnd);
        }
    }
}

impl Render for FloatingWindowView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel_is_here = self.state.read(cx).location == PanelLocation::Floating;
        let content = if panel_is_here {
            div().flex_1().p_3().child(self.panel.clone())
        } else {
            div()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(rgb(0x888888))
                .child("Панель находится в главном окне")
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1b1e22))
            .child(
                div()
                    .h(px(42.))
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .px_3()
                    .bg(rgb(0x30363d))
                    .text_color(white())
                    .child("FLOATING WINDOW")
                    .child(
                        div()
                            .ml_auto()
                            .px_2()
                            .py_1()
                            .bg(rgb(0x43566d))
                            .cursor_pointer()
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(Self::move_to_main),
                            )
                            .child("Вернуть в главное"),
                    ),
            )
            .child(content)
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("backspace", Backspace, Some("TransferPanelInput")),
            KeyBinding::new("delete", Delete, Some("TransferPanelInput")),
            KeyBinding::new("left", Left, Some("TransferPanelInput")),
            KeyBinding::new("right", Right, Some("TransferPanelInput")),
            KeyBinding::new("shift-left", SelectLeft, Some("TransferPanelInput")),
            KeyBinding::new("shift-right", SelectRight, Some("TransferPanelInput")),
            KeyBinding::new("ctrl-a", SelectAll, Some("TransferPanelInput")),
            KeyBinding::new("ctrl-c", Copy, Some("TransferPanelInput")),
            KeyBinding::new("ctrl-v", Paste, Some("TransferPanelInput")),
            KeyBinding::new("ctrl-x", Cut, Some("TransferPanelInput")),
            KeyBinding::new("home", Home, Some("TransferPanelInput")),
            KeyBinding::new("end", End, Some("TransferPanelInput")),
        ]);

        let state = cx.new(|_| TransferState::default());

        // This is the only panel Entity. Both window roots receive clones of this
        // same type-erased AnyView; their location flag ensures only one mounts it.
        let panel = cx.new(|cx| TransferPanel::new(state.clone(), cx));
        let panel_view = AnyView::from(panel.clone());

        let floating_options = WindowOptions {
            titlebar: None,
            kind: WindowKind::PopUp,
            show: false,
            focus: false,
            is_resizable: true,
            is_minimizable: false,
            is_movable: false,
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(1040.0), px(180.0)),
                size: size(px(440.0), px(340.0)),
            })),
            ..Default::default()
        };

        let floating_state = state.clone();
        let floating_panel = panel_view.clone();
        let floating_window = cx
            .open_window(floating_options, move |_, cx| {
                cx.new(|cx| {
                    cx.observe(&floating_state, |_, _, cx| cx.notify())
                        .detach();
                    FloatingWindowView {
                        state: floating_state,
                        panel: floating_panel,
                    }
                })
            })
            .expect("не удалось создать скрытое floating-окно");

        let floating_hwnd = floating_window
            .update(cx, |_, window, _| tearoff_win::hwnd_of(window).unwrap_or(0))
            .unwrap_or(0);
        if floating_hwnd != 0 {
            tearoff_win::hide_window(floating_hwnd);
        }

        state.update(cx, |state, _| state.floating_hwnd = floating_hwnd);

        let main_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(160.0), px(120.0)),
                size: size(px(1000.0), px(620.0)),
            })),
            ..Default::default()
        };

        let main_state = state.clone();
        let main_panel = panel_view.clone();
        let main_window = cx
            .open_window(main_options, move |_, cx| {
                cx.new(|cx| {
                    cx.observe(&main_state, |_, _, cx| cx.notify()).detach();
                    MainWindowView {
                        state: main_state,
                        panel: main_panel,
                    }
                })
            })
            .expect("не удалось создать главное окно");

        let main_hwnd = main_window
            .update(cx, |_, window, _| tearoff_win::hwnd_of(window).unwrap_or(0))
            .unwrap_or(0);

        state.update(cx, |state, _| state.main_hwnd = main_hwnd);

        if floating_hwnd != 0 && main_hwnd != 0 {
            tearoff_win::set_owner(floating_hwnd, main_hwnd);
        }

        // Start with the input focused in the main window. After transfer the
        // focus indicator tells whether GPUI carried that focus to the target.
        main_window
            .update(cx, |_, window, cx| {
                let focus_handle = panel.read(cx).focus_handle.clone();
                window.focus(&focus_handle, cx);
            })
            .expect("не удалось установить начальный фокус");

        cx.activate(true);
    });
}

mod tearoff_win {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            GWLP_HWNDPARENT, SW_HIDE, SW_SHOW, SetForegroundWindow, SetWindowLongPtrW,
            ShowWindow,
        },
    };

    fn to_hwnd(value: isize) -> HWND {
        HWND(value as _)
    }

    pub fn hwnd_of(window: &gpui::Window) -> Option<isize> {
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        }
    }

    pub fn set_owner(window_hwnd: isize, owner_hwnd: isize) {
        unsafe {
            let _ = SetWindowLongPtrW(to_hwnd(window_hwnd), GWLP_HWNDPARENT, owner_hwnd);
        }
    }

    pub fn show_window(hwnd: isize) {
        unsafe {
            let _ = ShowWindow(to_hwnd(hwnd), SW_SHOW);
        }
    }

    pub fn hide_window(hwnd: isize) {
        unsafe {
            let _ = ShowWindow(to_hwnd(hwnd), SW_HIDE);
        }
    }

    pub fn activate_window(hwnd: isize) {
        unsafe {
            let _ = SetForegroundWindow(to_hwnd(hwnd));
        }
    }
}

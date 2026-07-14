//! [`TuiOptionSelector`]: a reusable single-select option list for TUI
//! permission prompts, rendered from a frontend-neutral
//! [`OptionSnapshot`]. One configuration page shows a header (title,
//! "n of m" position, question), a highlightable option list with viewport
//! scrolling, optional Loading/Failed/Empty status rows, and an optional
//! custom-text footer editor.
//!
//! Enter/Escape are owned by the embedding card's keymap bindings and arrive
//! through [`TuiOptionSelector::confirm_highlighted`] /
//! [`TuiOptionSelector::handle_back`]; arrows, viewport-relative digits,
//! printable characters, clicks, and wheel scrolling are handled at the
//! element level since the selector is only rendered while its card is the
//! active blocking interaction.

use warp::tui_export::{OptionBadge, OptionFooter, OptionRow, OptionSnapshot, OptionSourceStatus};
use warp_search_core::inline_menu::InlineMenuSelection;
use warpui_core::elements::tui::{
    Modifier, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiFlex, TuiHoverable,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiParentElement, TuiPresentationContext,
    TuiScreenPoint, TuiScreenPosition, TuiSize, TuiStyle, TuiText,
};
use warpui_core::elements::MouseStateHandle;
use warpui_core::{AppContext, Entity, TuiView, TypedActionView, ViewContext};

use crate::inline_menu::keep_selected_visible;
use crate::tui_builder::TuiUiBuilder;

/// Maximum option rows visible at once; longer lists scroll.
pub(crate) const MAX_VISIBLE_OPTION_ROWS: usize = 4;

/// Validation copy shown when the custom-text editor is submitted empty.
const CUSTOM_TEXT_EMPTY_ERROR: &str = "Enter a value to continue.";

/// Header metadata rendered above the option list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OptionSelectorHeader {
    pub(crate) title: String,
    /// One-based position in the current page sequence: `(current, total)`.
    pub(crate) position: (usize, usize),
    pub(crate) question: String,
}

/// Events emitted to the embedding card view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TuiOptionSelectorEvent {
    /// An enabled option row was confirmed.
    Confirmed { id: String },
    /// The custom-text footer editor was submitted with a valid value.
    CustomTextSubmitted { value: String },
    /// The Retry affordance of a `Failed` catalog was activated.
    RetryRequested,
    /// The selector asked to be dismissed (element-level Escape fallback for
    /// hosts without their own Escape binding).
    Dismissed,
}

/// User interactions dispatched from the selector's element tree.
#[derive(Clone, Debug)]
pub(crate) enum TuiOptionSelectorAction {
    MoveUp,
    MoveDown,
    /// Confirm (or highlight, when disabled) the item at a viewport-relative
    /// digit position 1-9.
    SelectNumberedOption(u8),
    /// Confirm (or highlight, when disabled) the item at an absolute index;
    /// dispatched by row clicks.
    SelectItem(usize),
    /// Scroll the viewport by whole rows without moving the highlight.
    ScrollBy(isize),
    /// Append a printable character to the custom-text editor.
    InsertChar(char),
    /// Delete the last character of the custom-text editor.
    Backspace,
    /// Element-level Escape fallback (see [`TuiOptionSelectorEvent::Dismissed`]).
    Dismiss,
}

/// One navigable entry in the selector, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectorItem {
    /// Index into `snapshot.rows`.
    Row(usize),
    /// The Retry affordance shown for a `Failed` catalog.
    Retry,
    /// The custom-text footer entry point.
    CustomText,
}

/// State of the one-line custom-text editor while it is active.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CustomTextEditor {
    buffer: String,
    error: Option<String>,
}

/// A reusable single-select option list view. See the module docs.
pub(crate) struct TuiOptionSelector {
    header: OptionSelectorHeader,
    snapshot: OptionSnapshot,
    selection: InlineMenuSelection,
    scroll_offset: usize,
    /// `Some` while the custom-text footer editor is active.
    custom_text: Option<CustomTextEditor>,
    /// Submitted value for a custom-text footer. It replaces the generic
    /// footer label and pre-fills the editor when the user edits it again.
    custom_text_value: Option<String>,
    /// Per-item mouse state, indexed like [`Self::items`]. Owned here (not
    /// created inline during render) so hover/click state survives
    /// element-tree rebuilds.
    item_mouse_states: Vec<MouseStateHandle>,
}

impl TuiOptionSelector {
    /// Creates an empty selector; hosts call [`Self::set_page`] before render.
    pub(crate) fn new() -> Self {
        Self {
            header: OptionSelectorHeader::default(),
            snapshot: OptionSnapshot {
                rows: Vec::new(),
                selected_id: None,
                status: OptionSourceStatus::Ready,
                footer: None,
            },
            selection: InlineMenuSelection::default(),
            scroll_offset: 0,
            custom_text: None,
            custom_text_value: None,
            item_mouse_states: Vec::new(),
        }
    }

    /// Replaces the header and snapshot for a new page: the highlight starts
    /// on the snapshot's current value and any in-progress
    /// custom-text editing is discarded.
    pub(crate) fn set_page(
        &mut self,
        header: OptionSelectorHeader,
        snapshot: OptionSnapshot,
        ctx: &mut ViewContext<Self>,
    ) {
        self.header = header;
        self.custom_text_value = custom_text_value(&snapshot);
        self.snapshot = snapshot;
        self.custom_text = None;
        self.selection.clear();
        self.highlight_id(self.snapshot.selected_id.clone());
        self.sync_after_items_changed();
        ctx.notify();
    }

    /// Refreshes the snapshot in place after a live catalog change,
    /// preserving the highlighted row when it still exists and falling back
    /// to the snapshot's selected value otherwise.
    pub(crate) fn refresh_snapshot(
        &mut self,
        snapshot: OptionSnapshot,
        ctx: &mut ViewContext<Self>,
    ) {
        let highlighted = self.highlighted_row_id();
        self.custom_text_value = custom_text_value(&snapshot);
        self.snapshot = snapshot;
        let target = highlighted
            .filter(|id| self.snapshot.rows.iter().any(|row| &row.id == id))
            .or_else(|| self.snapshot.selected_id.clone());
        self.highlight_id(target);
        self.sync_after_items_changed();
        ctx.notify();
    }

    /// Whether the custom-text footer editor is currently active.
    #[cfg(test)]
    fn is_editing_custom_text(&self) -> bool {
        self.custom_text.is_some()
    }

    /// Confirms the highlighted item (Enter): enabled rows emit
    /// [`TuiOptionSelectorEvent::Confirmed`]; disabled rows are kept
    /// highlighted so their reason stays visible. While the
    /// custom-text editor is active, Enter validates and submits it instead
    ///.
    pub(crate) fn confirm_highlighted(&mut self, ctx: &mut ViewContext<Self>) {
        if self.custom_text.is_some() {
            self.submit_custom_text(ctx);
            return;
        }
        let Some(index) = self.selection.selected_index() else {
            return;
        };
        self.confirm_item(index, ctx);
    }

    /// Handles Escape from the embedding card: cancels active custom-text
    /// editing and reports whether the key was consumed, so the card only
    /// leaves the page when the selector had nothing to unwind.
    pub(crate) fn handle_back(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if self.custom_text.take().is_some() {
            ctx.notify();
            return true;
        }
        false
    }

    /// The navigable entries, in display order.
    fn items(&self) -> Vec<SelectorItem> {
        let mut items: Vec<SelectorItem> = (0..self.snapshot.rows.len())
            .map(SelectorItem::Row)
            .collect();
        if matches!(self.snapshot.status, OptionSourceStatus::Failed { .. }) {
            items.push(SelectorItem::Retry);
        }
        match &self.snapshot.footer {
            Some(OptionFooter::CustomText { .. }) => items.push(SelectorItem::CustomText),
            // Resource creation is out of scope in the TUI.
            Some(OptionFooter::CreateNewAuthSecret) | None => {}
        }
        items
    }

    /// Whether the item can be confirmed. Disabled rows stay highlightable
    /// but unconfirmable.
    fn item_is_confirmable(&self, item: SelectorItem) -> bool {
        match item {
            SelectorItem::Row(index) => self
                .snapshot
                .rows
                .get(index)
                .is_some_and(|row| row.disabled_reason.is_none()),
            SelectorItem::Retry | SelectorItem::CustomText => true,
        }
    }

    /// The row id currently highlighted, when the highlight is on a row.
    fn highlighted_row_id(&self) -> Option<String> {
        let items = self.items();
        match self.selection.selected_index().and_then(|i| items.get(i)) {
            Some(SelectorItem::Row(index)) => {
                self.snapshot.rows.get(*index).map(|row| row.id.clone())
            }
            Some(SelectorItem::Retry) | Some(SelectorItem::CustomText) | None => None,
        }
    }

    /// Moves the highlight to the row with `id`, else the first item.
    fn highlight_id(&mut self, id: Option<String>) {
        let items = self.items();
        let target = id
            .and_then(|id| {
                items.iter().position(|item| match item {
                    SelectorItem::Row(index) => self
                        .snapshot
                        .rows
                        .get(*index)
                        .is_some_and(|row| row.id == id),
                    SelectorItem::CustomText => self.custom_text_value.as_ref() == Some(&id),
                    SelectorItem::Retry => false,
                })
            })
            .or(if items.is_empty() { None } else { Some(0) });
        self.selection.clear();
        if let Some(target) = target {
            self.selection.select(target, items.len(), |_| true);
        }
    }

    /// Clamps scroll state and mouse-handle storage to the current items.
    fn sync_after_items_changed(&mut self) {
        let items_len = self.items().len();
        self.scroll_offset = self
            .scroll_offset
            .min(items_len.saturating_sub(MAX_VISIBLE_OPTION_ROWS));
        if let Some(selected) = self.selection.selected_index() {
            keep_selected_visible(
                items_len,
                selected,
                MAX_VISIBLE_OPTION_ROWS,
                &mut self.scroll_offset,
            );
        }
        // Handles are stable per item index across renders; grow as needed.
        while self.item_mouse_states.len() < items_len {
            self.item_mouse_states.push(MouseStateHandle::default());
        }
    }

    /// Moves the highlight one step, scrolling to keep it visible.
    fn move_highlight(&mut self, forward: bool, ctx: &mut ViewContext<Self>) {
        let items_len = self.items().len();
        if forward {
            self.selection.select_next(items_len, |_| true);
        } else {
            self.selection.select_previous(items_len, |_| true);
        }
        if let Some(selected) = self.selection.selected_index() {
            keep_selected_visible(
                items_len,
                selected,
                MAX_VISIBLE_OPTION_ROWS,
                &mut self.scroll_offset,
            );
        }
        ctx.notify();
    }

    /// Confirms the item at `index` when enabled; otherwise highlights it so
    /// its disabled reason is surfaced.
    fn confirm_item(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        let items = self.items();
        let Some(item) = items.get(index).copied() else {
            return;
        };
        self.selection.select(index, items.len(), |_| true);
        keep_selected_visible(
            items.len(),
            index,
            MAX_VISIBLE_OPTION_ROWS,
            &mut self.scroll_offset,
        );
        if !self.item_is_confirmable(item) {
            ctx.notify();
            return;
        }
        match item {
            SelectorItem::Row(row_index) => {
                if let Some(row) = self.snapshot.rows.get(row_index) {
                    ctx.emit(TuiOptionSelectorEvent::Confirmed { id: row.id.clone() });
                }
            }
            SelectorItem::Retry => ctx.emit(TuiOptionSelectorEvent::RetryRequested),
            SelectorItem::CustomText => {
                self.custom_text = Some(CustomTextEditor {
                    buffer: self.custom_text_value.clone().unwrap_or_default(),
                    error: None,
                });
            }
        }
        ctx.notify();
    }

    /// Validates and submits the custom-text editor: the value
    /// is trimmed; empty input stays editable with a concise error.
    fn submit_custom_text(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(editor) = &mut self.custom_text else {
            return;
        };
        let value = editor.buffer.trim().to_string();
        if value.is_empty() {
            editor.error = Some(CUSTOM_TEXT_EMPTY_ERROR.to_string());
        } else {
            self.custom_text = None;
            self.snapshot.selected_id = Some(value.clone());
            self.custom_text_value = Some(value.clone());
            ctx.emit(TuiOptionSelectorEvent::CustomTextSubmitted { value });
        }
        ctx.notify();
    }

    /// Scrolls the viewport by `rows` without moving the highlight
    ///.
    fn scroll_by(&mut self, rows: isize, ctx: &mut ViewContext<Self>) {
        let items_len = self.items().len();
        let max_offset = items_len.saturating_sub(MAX_VISIBLE_OPTION_ROWS);
        self.scroll_offset = self
            .scroll_offset
            .saturating_add_signed(rows)
            .min(max_offset);
        ctx.notify();
    }

    // ── Rendering ───────────────────────────────────────────────────

    /// One header block: title + position, then the page's question.
    fn render_header(&self, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
        let (current, total) = self.header.position;
        let title = TuiText::new(self.header.title.clone())
            .with_style(builder.primary_text_style())
            .truncate()
            .finish();
        let previous_style = if current > 1 {
            builder.primary_text_style()
        } else {
            builder.muted_text_style()
        };
        let next_style = if current < total {
            builder.primary_text_style()
        } else {
            builder.muted_text_style()
        };
        let position = TuiText::from_spans([
            ("←".to_string(), previous_style),
            (format!(" {current} "), builder.primary_text_style()),
            (format!("of {total} "), builder.muted_text_style()),
            ("→".to_string(), next_style),
        ])
        .truncate()
        .finish();
        let title_row = TuiFlex::row()
            .child(title)
            .flex_child(TuiFlex::row().finish())
            .child(position)
            .finish();
        TuiFlex::column()
            .child(title_row)
            .child(TuiText::new(" ").finish())
            .child(
                TuiText::new(self.header.question.clone())
                    .with_style(builder.primary_text_style().add_modifier(Modifier::BOLD))
                    .finish(),
            )
            .finish()
    }

    /// One option row: viewport-relative digit, label, badge, and disabled
    /// reason, with the current selection rendered in bold magenta.
    fn render_row(
        &self,
        row: &OptionRow,
        digit: Option<usize>,
        is_highlighted: bool,
        builder: &TuiUiBuilder,
    ) -> Box<dyn TuiElement> {
        let disabled = row.disabled_reason.is_some();
        let label_style = if is_highlighted {
            builder.orchestration_option_selected_style()
        } else if disabled {
            builder.dim_text_style()
        } else {
            builder.primary_text_style()
        };
        let detail_style = if is_highlighted {
            builder.orchestration_option_selected_style()
        } else if disabled {
            builder.dim_text_style()
        } else {
            builder.muted_text_style()
        };
        let digit_prefix = match digit {
            Some(digit) => format!("({digit}) "),
            None => "    ".to_string(),
        };
        let mut spans = vec![
            (digit_prefix, detail_style),
            (row.label.clone(), label_style),
        ];
        let badge = match row.badge {
            Some(OptionBadge::Default) => Some("default"),
            Some(OptionBadge::Recent) => Some("recent"),
            Some(OptionBadge::Connected) => Some("connected"),
            None => None,
        };
        if let Some(badge) = badge {
            spans.push((format!("  ({badge})"), detail_style));
        }
        if let Some(reason) = &row.disabled_reason {
            spans.push((format!(" — {reason}"), detail_style));
        }
        TuiText::from_spans(spans).truncate().finish()
    }

    /// A generic single-span selectable virtual row (Retry / custom text).
    fn render_virtual_row(
        &self,
        text: String,
        digit: Option<usize>,
        is_highlighted: bool,
        style: TuiStyle,
        builder: &TuiUiBuilder,
    ) -> Box<dyn TuiElement> {
        let style = if is_highlighted {
            builder.orchestration_option_selected_style()
        } else {
            style
        };
        let digit_prefix = match digit {
            Some(digit) => format!("({digit}) "),
            None => "    ".to_string(),
        };
        TuiText::from_spans([(format!("{digit_prefix}{text}"), style)])
            .truncate()
            .finish()
    }

    /// The active custom-text editor row plus its validation error, if any.
    fn render_custom_text_editor(
        &self,
        editor: &CustomTextEditor,
        label: &str,
        builder: &TuiUiBuilder,
    ) -> Box<dyn TuiElement> {
        let mut column = TuiFlex::column();
        column.add_child(
            TuiText::from_spans([
                (format!("{label}: "), builder.primary_text_style()),
                (
                    format!("{}▏", editor.buffer),
                    builder.primary_text_style().add_modifier(Modifier::BOLD),
                ),
            ])
            .truncate()
            .finish(),
        );
        if let Some(error) = &editor.error {
            column.add_child(
                TuiText::new(error.clone())
                    .with_style(builder.error_text_style())
                    .truncate()
                    .finish(),
            );
        }
        column.finish()
    }

    /// The option list: visible window of items with digit prefixes, plus
    /// non-selectable status rows for Loading/Failed/Empty.
    fn render_list(&self, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
        let items = self.items();
        let mut column = TuiFlex::column();

        let visible_end = (self.scroll_offset + MAX_VISIBLE_OPTION_ROWS).min(items.len());
        let visible = self.scroll_offset..visible_end;
        if self.scroll_offset > 0 {
            column.add_child(
                TuiText::new("↑")
                    .with_style(builder.dim_text_style())
                    .truncate()
                    .finish(),
            );
        }
        for (position, index) in visible.clone().enumerate() {
            let item = items[index];
            let digit = (position < 9).then_some(position + 1);
            let is_highlighted =
                self.custom_text.is_none() && self.selection.selected_index() == Some(index);
            let element = match item {
                SelectorItem::Row(row_index) => {
                    let Some(row) = self.snapshot.rows.get(row_index) else {
                        continue;
                    };
                    self.render_row(row, digit, is_highlighted, builder)
                }
                SelectorItem::Retry => self.render_virtual_row(
                    "↻ Retry".to_string(),
                    digit,
                    is_highlighted,
                    builder.error_text_style(),
                    builder,
                ),
                SelectorItem::CustomText => match (&self.snapshot.footer, &self.custom_text) {
                    (Some(OptionFooter::CustomText { label }), Some(editor)) => {
                        self.render_custom_text_editor(editor, label, builder)
                    }
                    (Some(OptionFooter::CustomText { label }), None) => self.render_virtual_row(
                        self.custom_text_value
                            .clone()
                            .unwrap_or_else(|| label.clone()),
                        digit,
                        is_highlighted,
                        builder.primary_text_style(),
                        builder,
                    ),
                    (Some(OptionFooter::CreateNewAuthSecret) | None, _) => continue,
                },
            };
            // Each visible row is clickable through its own persistent
            // mouse-state handle.
            let element = match self.item_mouse_states.get(index) {
                Some(mouse_state) => TuiHoverable::new(mouse_state.clone(), element)
                    .on_click(move |event_ctx, _| {
                        event_ctx.dispatch_typed_action(TuiOptionSelectorAction::SelectItem(index));
                    })
                    .finish(),
                None => element,
            };
            column.add_child(element);
        }
        if visible_end < items.len() {
            column.add_child(
                TuiText::new("↓")
                    .with_style(builder.dim_text_style())
                    .truncate()
                    .finish(),
            );
        }

        match &self.snapshot.status {
            OptionSourceStatus::Ready => {}
            OptionSourceStatus::Loading => {
                column.add_child(
                    TuiText::new("Loading…")
                        .with_style(builder.dim_text_style())
                        .truncate()
                        .finish(),
                );
            }
            OptionSourceStatus::Failed { message } => {
                column.add_child(
                    TuiText::new(message.clone())
                        .with_style(builder.error_text_style())
                        .truncate()
                        .finish(),
                );
            }
            OptionSourceStatus::Empty { message } => {
                column.add_child(
                    TuiText::new(message.clone())
                        .with_style(builder.dim_text_style())
                        .truncate()
                        .finish(),
                );
            }
        }
        column.finish()
    }
}

/// A custom-text selection is encoded as a selected id that is not one of
/// the snapshot's fixed rows.
fn custom_text_value(snapshot: &OptionSnapshot) -> Option<String> {
    if !matches!(snapshot.footer, Some(OptionFooter::CustomText { .. })) {
        return None;
    }
    snapshot
        .selected_id
        .as_ref()
        .filter(|selected| !snapshot.rows.iter().any(|row| &row.id == *selected))
        .cloned()
}
impl Entity for TuiOptionSelector {
    type Event = TuiOptionSelectorEvent;
}

impl TuiView for TuiOptionSelector {
    fn ui_name() -> &'static str {
        "TuiOptionSelector"
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        let content = TuiFlex::column()
            .child(self.render_header(&builder))
            .child(self.render_list(&builder))
            .finish();
        SelectorInputElement {
            child: content,
            editing_custom_text: self.custom_text.is_some(),
        }
        .finish()
    }
}

impl TypedActionView for TuiOptionSelector {
    fn handle_action(&mut self, action: &TuiOptionSelectorAction, ctx: &mut ViewContext<Self>) {
        match action {
            TuiOptionSelectorAction::MoveUp => self.move_highlight(false, ctx),
            TuiOptionSelectorAction::MoveDown => self.move_highlight(true, ctx),
            TuiOptionSelectorAction::SelectNumberedOption(digit) => {
                let index = self.scroll_offset + usize::from(*digit) - 1;
                self.confirm_item(index, ctx);
            }
            TuiOptionSelectorAction::SelectItem(index) => self.confirm_item(*index, ctx),
            TuiOptionSelectorAction::ScrollBy(rows) => self.scroll_by(*rows, ctx),
            TuiOptionSelectorAction::InsertChar(c) => {
                if let Some(editor) = &mut self.custom_text {
                    editor.buffer.push(*c);
                    editor.error = None;
                    ctx.notify();
                }
            }
            TuiOptionSelectorAction::Backspace => {
                if let Some(editor) = &mut self.custom_text {
                    editor.buffer.pop();
                    editor.error = None;
                    ctx.notify();
                }
            }
            TuiOptionSelectorAction::Dismiss => {
                if !self.handle_back(ctx) {
                    ctx.emit(TuiOptionSelectorEvent::Dismissed);
                }
            }
        }
    }

    type Action = TuiOptionSelectorAction;
}

/// Wraps the selector's rendered content and translates element-level input
/// (arrows, digits, custom-text characters, wheel scrolling) into
/// [`TuiOptionSelectorAction`]s.
struct SelectorInputElement {
    child: Box<dyn TuiElement>,
    editing_custom_text: bool,
}

impl TuiElement for SelectorInputElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        self.child.layout(constraint, ctx, app)
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.child.render(origin, surface, ctx);
    }

    fn size(&self) -> Option<TuiSize> {
        self.child.size()
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.child.origin()
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        if self.child.dispatch_event(event, event_ctx, app) {
            return true;
        }
        match event {
            TuiEvent::KeyDown {
                keystroke, chars, ..
            } => {
                if keystroke.ctrl || keystroke.alt || keystroke.cmd || keystroke.meta {
                    return false;
                }
                if self.editing_custom_text {
                    if keystroke.key == "backspace" {
                        event_ctx.dispatch_typed_action(TuiOptionSelectorAction::Backspace);
                        return true;
                    }
                    if keystroke.key == "escape" {
                        event_ctx.dispatch_typed_action(TuiOptionSelectorAction::Dismiss);
                        return true;
                    }
                    if let Some(c) = chars.chars().next().filter(|c| !c.is_control()) {
                        event_ctx.dispatch_typed_action(TuiOptionSelectorAction::InsertChar(c));
                        return true;
                    }
                    return false;
                }
                match keystroke.key.as_str() {
                    "escape" => {
                        // Escape fallback for hosts without their own
                        // Escape keymap binding; the embedding card's
                        // `escape` binding normally consumes the key first.
                        event_ctx.dispatch_typed_action(TuiOptionSelectorAction::Dismiss);
                        true
                    }
                    "up" => {
                        event_ctx.dispatch_typed_action(TuiOptionSelectorAction::MoveUp);
                        true
                    }
                    "down" => {
                        event_ctx.dispatch_typed_action(TuiOptionSelectorAction::MoveDown);
                        true
                    }
                    key => match key.parse::<u8>() {
                        Ok(digit @ 1..=9) => {
                            event_ctx.dispatch_typed_action(
                                TuiOptionSelectorAction::SelectNumberedOption(digit),
                            );
                            true
                        }
                        Ok(_) | Err(_) => false,
                    },
                }
            }
            TuiEvent::ScrollWheel {
                position, delta, ..
            } => {
                let Some((origin, size)) = self.origin().zip(self.size()) else {
                    return false;
                };
                if !event_ctx.hit_test(origin, size, *position) {
                    return false;
                }
                let (_, rows) = *delta;
                if rows == 0 {
                    return false;
                }
                // Positive wheel delta scrolls the content up (toward the
                // start of the list), matching the transcript's scrollable.
                event_ctx.dispatch_typed_action(TuiOptionSelectorAction::ScrollBy(-rows));
                true
            }
            TuiEvent::Paste { text } => {
                if !self.editing_custom_text {
                    return false;
                }
                // The custom-text editor is single-line (host slugs), so only
                // the first line's printable characters are inserted.
                let mut handled = false;
                for c in text
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .chars()
                    .filter(|c| !c.is_control())
                {
                    event_ctx.dispatch_typed_action(TuiOptionSelectorAction::InsertChar(c));
                    handled = true;
                }
                handled
            }
            TuiEvent::LeftMouseDown { .. }
            | TuiEvent::LeftMouseUp { .. }
            | TuiEvent::LeftMouseDragged { .. }
            | TuiEvent::MiddleMouseDown { .. }
            | TuiEvent::RightMouseDown { .. }
            | TuiEvent::MouseMoved { .. } => false,
        }
    }
}

#[cfg(test)]
#[path = "option_selector_tests.rs"]
mod tests;

//! Generic focusable TUI text field over the shared editor model and element.
//!
//! Unlike [`crate::input::TuiInputView`], this view owns no prompt submission,
//! input-mode, inline-menu, or form policy. Embedding views provide that chrome
//! and behavior while reusing model-backed editing and focus handling.

use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp_editor::model::{CoreEditorModel, PlainTextEditorModel};
use warp_editor::selection::{TextDirection, TextUnit};
use warpui_core::elements::tui::{TuiElement, TuiHoverable};
use warpui_core::elements::MouseStateHandle;
use warpui_core::text::word_boundaries::WordBoundariesPolicy;
use warpui_core::{
    AppContext, BlurContext, Entity, FocusContext, ModelHandle, TuiView, TypedActionView,
    ViewContext,
};

use crate::editor_element::{TuiEditorAction, TuiEditorElement};
use crate::keybindings::TuiEditorCommand;

/// Events emitted when the editor content changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TuiEditorViewEvent {
    Changed(String),
}

/// Actions raised by the shared editor element or editor chrome.
#[derive(Clone, Debug)]
pub(crate) enum TuiEditorViewAction {
    FocusRequested,
    Editor(TuiEditorAction),
    Command(TuiEditorCommand),
}

/// A reusable single-line editor view.
pub(crate) struct TuiEditorView {
    model: ModelHandle<CodeEditorModel>,
    focused: bool,
    /// Target text of an in-flight programmatic replacement. Content events
    /// are suppressed until the model reaches this value (including an
    /// intermediate clear event).
    suppressed_text_target: Option<String>,
    mouse_state: MouseStateHandle,
}

impl TuiEditorView {
    /// Creates an empty single-line editor backed by a char-cell model.
    pub(crate) fn single_line(ctx: &mut ViewContext<Self>) -> Self {
        let model = ctx.add_model(|ctx| CodeEditorModel::new_tui(1, ctx));
        ctx.subscribe_to_model(&model, |editor, _, event, ctx| {
            if !matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                return;
            }
            let text = editor.text(ctx);
            if let Some(target) = &editor.suppressed_text_target {
                if &text == target {
                    editor.suppressed_text_target = None;
                }
            } else {
                ctx.emit(TuiEditorViewEvent::Changed(text));
            }
            ctx.notify();
        });
        Self {
            model,
            focused: false,
            suppressed_text_target: None,
            mouse_state: MouseStateHandle::default(),
        }
    }

    /// Returns the current editor text.
    pub(crate) fn text(&self, ctx: &AppContext) -> String {
        let model = self.model.as_ref(ctx);
        let buffer = model.content().as_ref(ctx);
        if buffer.is_empty() {
            String::new()
        } else {
            buffer.text().into_string()
        }
    }

    /// Returns whether the editor owns focus.
    pub(crate) fn is_focused(&self) -> bool {
        self.focused
    }

    /// Replaces editor content without emitting `Changed`.
    pub(crate) fn set_text(&mut self, text: impl Into<String>, ctx: &mut ViewContext<Self>) {
        let text = text.into();
        if self.text(ctx) == text {
            return;
        }
        self.suppressed_text_target = Some(text.clone());
        self.model.update(ctx, |model, ctx| {
            model.clear_buffer(ctx);
            model.user_insert(&text, ctx);
        });
        ctx.notify();
    }

    /// Renders the shared editor configured as a one-row field.
    fn render_editor(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        TuiEditorElement::new(&self.model, ctx)
            .editable()
            .with_view_focused(self.focused)
            .with_viewport_rows(1)
            .on_action(|action, event_ctx| {
                event_ctx.dispatch_typed_action(TuiEditorViewAction::Editor(action));
            })
            .finish()
    }

    /// Applies an editor action using the same model operations as `TuiInputView`.
    fn handle_editor_action(&mut self, action: &TuiEditorAction, ctx: &mut ViewContext<Self>) {
        if matches!(
            action,
            TuiEditorAction::SelectionStartAt { .. }
                | TuiEditorAction::SelectionExtendTo { .. }
                | TuiEditorAction::SelectWordAt { .. }
                | TuiEditorAction::SelectLineAt { .. }
        ) {
            ctx.focus_self();
        }
        match action {
            TuiEditorAction::InsertChar(c) => {
                self.model
                    .update(ctx, |model, ctx| model.user_insert(&c.to_string(), ctx));
            }
            TuiEditorAction::InsertText(text) => {
                let first_line = text.lines().next().unwrap_or_default();
                self.model
                    .update(ctx, |model, ctx| model.user_insert(first_line, ctx));
            }
            TuiEditorAction::SelectionStartAt { offset } => {
                self.model
                    .update(ctx, |model, ctx| model.select_at(*offset, false, ctx));
            }
            TuiEditorAction::SelectionExtendTo { offset } => {
                self.model.update(ctx, |model, ctx| {
                    model.set_last_selection_head(*offset, ctx)
                });
            }
            TuiEditorAction::SelectWordAt { offset } => {
                self.model
                    .update(ctx, |model, ctx| model.select_word_at(*offset, false, ctx));
            }
            TuiEditorAction::SelectLineAt { offset } => {
                self.model
                    .update(ctx, |model, ctx| model.select_line_at(*offset, false, ctx));
            }
            TuiEditorAction::SelectionUpdateTo { offset } => {
                self.model.update(ctx, |model, ctx| {
                    model.update_pending_selection(*offset, ctx)
                });
            }
            TuiEditorAction::SelectionEnd => {
                self.model
                    .update(ctx, |model, ctx| model.end_selection(ctx));
            }
            TuiEditorAction::Scroll { .. } => {}
        }
    }

    /// Applies a keybound editor command to the shared editor model.
    fn handle_command(&mut self, command: TuiEditorCommand, ctx: &mut ViewContext<Self>) {
        match command {
            TuiEditorCommand::Backspace => {
                self.model.update(ctx, |model, ctx| model.backspace(ctx));
            }
            TuiEditorCommand::DeleteForward => {
                self.model.update(ctx, |model, ctx| {
                    model.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
                });
            }
            TuiEditorCommand::DeleteWordBackward => {
                self.model.update(ctx, |model, ctx| {
                    model.delete(
                        TextDirection::Backwards,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        false,
                        ctx,
                    );
                });
            }
            TuiEditorCommand::DeleteWordForward => {
                self.model.update(ctx, |model, ctx| {
                    model.delete(
                        TextDirection::Forwards,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        false,
                        ctx,
                    );
                });
            }
            TuiEditorCommand::MoveLeft => {
                self.model.update(ctx, |model, ctx| model.move_left(ctx));
            }
            TuiEditorCommand::MoveRight => {
                self.model.update(ctx, |model, ctx| model.move_right(ctx));
            }
            TuiEditorCommand::MoveWordLeft => {
                self.model.update(ctx, |model, ctx| {
                    model.backward_word_with_unit(
                        false,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        ctx,
                    );
                });
            }
            TuiEditorCommand::MoveWordRight => {
                self.model.update(ctx, |model, ctx| {
                    model.forward_word_with_unit(
                        false,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        ctx,
                    );
                });
            }
            TuiEditorCommand::MoveToLineStart => {
                self.model
                    .update(ctx, |model, ctx| model.move_to_line_start(ctx));
            }
            TuiEditorCommand::MoveToLineEnd => {
                self.model
                    .update(ctx, |model, ctx| model.move_to_line_end(ctx));
            }
            TuiEditorCommand::SelectLeft => {
                self.model.update(ctx, |model, ctx| model.select_left(ctx));
            }
            TuiEditorCommand::SelectRight => {
                self.model.update(ctx, |model, ctx| model.select_right(ctx));
            }
            TuiEditorCommand::SelectWordLeft => {
                self.model.update(ctx, |model, ctx| {
                    model.backward_word_with_unit(
                        true,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        ctx,
                    );
                });
            }
            TuiEditorCommand::SelectWordRight => {
                self.model.update(ctx, |model, ctx| {
                    model.forward_word_with_unit(
                        true,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        ctx,
                    );
                });
            }
            TuiEditorCommand::SelectAll => {
                self.model.update(ctx, |model, ctx| model.select_all(ctx));
            }
            TuiEditorCommand::Undo => {
                self.model.update(ctx, |model, ctx| model.undo(ctx));
            }
            TuiEditorCommand::Redo => {
                self.model.update(ctx, |model, ctx| model.redo(ctx));
            }
        }
    }
}

impl Entity for TuiEditorView {
    type Event = TuiEditorViewEvent;
}

impl TuiView for TuiEditorView {
    fn ui_name() -> &'static str {
        "TuiEditorView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        TuiHoverable::new(self.mouse_state.clone(), self.render_editor(app))
            .on_click(|event_ctx, _| {
                event_ctx.dispatch_typed_action(TuiEditorViewAction::FocusRequested);
            })
            .finish()
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focused = true;
            ctx.notify();
        }
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.focused = false;
            ctx.notify();
        }
    }
}

impl TypedActionView for TuiEditorView {
    type Action = TuiEditorViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TuiEditorViewAction::FocusRequested => ctx.focus_self(),
            TuiEditorViewAction::Editor(action) => self.handle_editor_action(action, ctx),
            TuiEditorViewAction::Command(command) => self.handle_command(*command, ctx),
        }
        ctx.notify();
    }
}

#[cfg(test)]
#[path = "editor_view_tests.rs"]
mod tests;

//! TUI keybinding registration and cross-surface validation.
//!
//! Mirrors the GUI convention: each TUI view module exposes a top-level
//! `init(app)` that registers its keybindings, aggregated here and called once
//! at TUI startup (from [`crate::session`]'s mount). Fixed bindings are
//! reserved keys (ctrl-c); editable bindings are named `tui:*` so they are
//! user-remappable by name via `keybindings.yaml` (loading overrides in the
//! TUI process is a follow-up — the names registered here are the stable
//! contract).
//!
//! # Cross-surface isolation
//! GUI bindings cannot fire in the TUI even though the TUI process registers
//! them all: predicate-scoped bindings never match TUI keymap contexts, and
//! even a predicate-less binding dispatches an action type that no TUI view
//! handles, so the keystroke falls through to the element pass unharmed. The
//! debug-time validators below enforce the remaining convention: any
//! *keystroke* binding that matches a TUI view's context must be TUI-owned.
//! This catches GUI bindings registered without a context predicate — which
//! would otherwise match everywhere and, for multi-keystroke chords, swallow
//! prefix keys via a pending match.

use warpui_core::keymap::macros::*;
use warpui_core::keymap::{
    BindingLens, ContextPredicate, EditableBinding, IsBindingValid, Trigger,
};
use warpui_core::{Action, AppContext};

use crate::editor_view::{TuiEditorView, TuiEditorViewAction};
use crate::input::TuiInputView;
use crate::option_selector::TuiOptionSelector;
use crate::root_view::RootTuiView;
use crate::run_agents_card_view::TuiRunAgentsCardView;
use crate::terminal_session_view::TuiTerminalSessionView;
use crate::transcript_view::TuiTranscriptView;

/// Group tag set on every TUI-registered binding. The validators treat it (or
/// a `tui:` name prefix) as proof of TUI ownership.
pub(crate) const TUI_BINDING_GROUP: &str = "tui";

/// Editing commands whose key definitions are shared by TUI text fields.
#[derive(Clone, Copy, Debug)]
pub(crate) enum TuiEditorCommand {
    Backspace,
    DeleteForward,
    DeleteWordBackward,
    DeleteWordForward,
    MoveLeft,
    MoveRight,
    MoveWordLeft,
    MoveWordRight,
    MoveToLineStart,
    MoveToLineEnd,
    SelectLeft,
    SelectRight,
    SelectWordLeft,
    SelectWordRight,
    SelectAll,
    Undo,
    Redo,
}

/// Selects stable user-configurable names for each binding consumer.
#[derive(Clone, Copy)]
pub(crate) enum TuiEditorBindingTarget {
    Input,
    Editor,
}

struct EditorBindingSpec {
    command: TuiEditorCommand,
    input_name: &'static str,
    editor_name: &'static str,
    description: &'static str,
    keys: &'static [&'static str],
}

const SHARED_EDITOR_BINDINGS: &[EditorBindingSpec] = &[
    EditorBindingSpec {
        command: TuiEditorCommand::Backspace,
        input_name: "tui:input:backspace",
        editor_name: "tui:editor:backspace",
        description: "Delete the previous character",
        keys: &["backspace", "shift-backspace", "ctrl-h"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::DeleteForward,
        input_name: "tui:input:delete_forward",
        editor_name: "tui:editor:delete_forward",
        description: "Delete the next character",
        keys: &["delete", "ctrl-d"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::DeleteWordBackward,
        input_name: "tui:input:delete_word_backward",
        editor_name: "tui:editor:delete_word_backward",
        description: "Delete the previous word",
        keys: &["ctrl-w", "ctrl-backspace", "alt-backspace"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::DeleteWordForward,
        input_name: "tui:input:delete_word_forward",
        editor_name: "tui:editor:delete_word_forward",
        description: "Delete the next word",
        keys: &["alt-d", "alt-delete", "ctrl-delete"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::MoveLeft,
        input_name: "tui:input:move_left",
        editor_name: "tui:editor:move_left",
        description: "Move cursor left",
        keys: &["left", "ctrl-b"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::MoveRight,
        input_name: "tui:input:move_right",
        editor_name: "tui:editor:move_right",
        description: "Move cursor right",
        keys: &["right", "ctrl-f"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::MoveWordLeft,
        input_name: "tui:input:move_word_left",
        editor_name: "tui:editor:move_word_left",
        description: "Move cursor one word left",
        keys: &["alt-left", "alt-b", "ctrl-left"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::MoveWordRight,
        input_name: "tui:input:move_word_right",
        editor_name: "tui:editor:move_word_right",
        description: "Move cursor one word right",
        keys: &["alt-right", "alt-f", "ctrl-right"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::MoveToLineStart,
        input_name: "tui:input:move_to_line_start",
        editor_name: "tui:editor:move_to_line_start",
        description: "Move cursor to start of line",
        keys: &["home", "ctrl-a"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::MoveToLineEnd,
        input_name: "tui:input:move_to_line_end",
        editor_name: "tui:editor:move_to_line_end",
        description: "Move cursor to end of line",
        keys: &["end", "ctrl-e"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::SelectLeft,
        input_name: "tui:input:select_left",
        editor_name: "tui:editor:select_left",
        description: "Extend selection left",
        keys: &["shift-left"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::SelectRight,
        input_name: "tui:input:select_right",
        editor_name: "tui:editor:select_right",
        description: "Extend selection right",
        keys: &["shift-right"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::SelectWordLeft,
        input_name: "tui:input:select_word_left",
        editor_name: "tui:editor:select_word_left",
        description: "Extend selection one word left",
        keys: &["ctrl-shift-left", "alt-shift-left"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::SelectWordRight,
        input_name: "tui:input:select_word_right",
        editor_name: "tui:editor:select_word_right",
        description: "Extend selection one word right",
        keys: &["ctrl-shift-right", "alt-shift-right"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::SelectAll,
        input_name: "tui:input:select_all",
        editor_name: "tui:editor:select_all",
        description: "Select all text",
        keys: &["ctrl-shift-A"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::Undo,
        input_name: "tui:input:undo",
        editor_name: "tui:editor:undo",
        description: "Undo",
        keys: &["ctrl-z"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::Redo,
        input_name: "tui:input:redo",
        editor_name: "tui:editor:redo",
        description: "Redo",
        keys: &["ctrl-shift-Z"],
    },
];

/// Registers the common editor binding table for one concrete view action.
pub(crate) fn register_shared_editor_bindings<A>(
    app: &mut AppContext,
    target: TuiEditorBindingTarget,
    context: ContextPredicate,
    action_for: impl Fn(TuiEditorCommand) -> A,
) where
    A: Action,
{
    let bindings = SHARED_EDITOR_BINDINGS.iter().flat_map(|spec| {
        spec.keys.iter().map(|key| {
            let name = match target {
                TuiEditorBindingTarget::Input => spec.input_name,
                TuiEditorBindingTarget::Editor => spec.editor_name,
            };
            EditableBinding::new(name, spec.description, action_for(spec.command))
                .with_context_predicate(context.clone())
                .with_group(TUI_BINDING_GROUP)
                .with_key_binding(key)
        })
    });
    app.register_editable_bindings(bindings);
}

/// Registers all TUI view keybindings and the cross-surface binding
/// validators. Called once at TUI startup, before the driver starts.
pub(crate) fn init(app: &mut AppContext) {
    crate::root_view::init(app);
    crate::terminal_session_view::init(app);
    crate::input::init(app);
    register_shared_editor_bindings(
        app,
        TuiEditorBindingTarget::Editor,
        id!("TuiEditorView"),
        TuiEditorViewAction::Command,
    );
    crate::run_agents_card_view::init(app);

    register_binding_validators(app);
}

/// Debug-time guard (no-op in release): every keystroke binding that matches a
/// TUI view's default keymap context must be TUI-owned.
fn register_binding_validators(app: &mut AppContext) {
    app.register_tui_binding_validator::<RootTuiView>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiTerminalSessionView>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiInputView>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiEditorView>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiTranscriptView>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiRunAgentsCardView>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiOptionSelector>(is_tui_owned_binding);
}

fn is_tui_owned_binding(binding: BindingLens) -> IsBindingValid {
    // Non-keystroke triggers (palette-only `Empty`, `Standard`, `Custom`)
    // can never fire from TUI keyboard input, so they are exempt.
    if !matches!(binding.trigger, Trigger::Keystrokes(_)) {
        return IsBindingValid::Yes;
    }
    if is_tui_owned(binding.name, binding.group) {
        IsBindingValid::Yes
    } else {
        IsBindingValid::No
    }
}

/// Whether a binding's identity marks it as TUI-owned: a `tui:`-prefixed name
/// (editable bindings) or the [`TUI_BINDING_GROUP`] group (fixed bindings).
fn is_tui_owned(name: &str, group: Option<&str>) -> bool {
    name.starts_with("tui:") || group == Some(TUI_BINDING_GROUP)
}

#[cfg(test)]
#[path = "keybindings_tests.rs"]
mod tests;

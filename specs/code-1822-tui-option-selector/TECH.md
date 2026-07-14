# TECH: Reusable TUI option selector over shared option snapshots

## Context

This slice builds on the frontend-neutral orchestration option snapshots
(base commit `aaf62142`; see `specs/code-1822-option-snapshots/TECH.md` for the data
contract). At that base, `app/src/tui_export.rs` re-exports `OptionSnapshot`,
`OptionRow`, `OptionBadge`, `OptionSourceStatus`, and `OptionFooter`, but nothing in
`crates/warp_tui` renders them: the TUI has no single-select list primitive.

This PR adds that primitive — `TuiOptionSelector` — plus the generic
`TuiEditorView::single_line` it uses for searchable lists and custom-text entries.
The next slice (the TUI orchestration permission/configuration card) embeds the
selector to render its per-field configuration pages; the same primitive is intended
for future AskUserQuestion and permission prompts, which is why it is snapshot-driven
and knows nothing about orchestration edit state.

## Proposed changes

### `crates/warp_tui/src/option_selector.rs`

A `TuiView` + `TypedActionView` (`TuiOptionSelector`) rendering one page:

- Header (`OptionSelectorHeader`): title on the left, right-aligned `← n of m →`
  navigation state (boundary arrows muted), a blank separator row, and the page's
  bold question.
- Option list rendered from an `OptionSnapshot` (`warp::tui_export`): up to
  `MAX_VISIBLE_OPTION_ROWS` (4) rows visible at once with `↑` / `↓` overflow markers.
  Rows show a viewport-relative `(1)`-style number, the label, an optional badge
  suffix (`(default)` / `(recent)` / `(connected)`), and — for disabled rows — the
  `disabled_reason`. The selected row is bold magenta without an extra marker or
  background.
- Optional search: `set_page(..., searchable, ctx)` renders a pinned `Search:` row
  between the question and the scroll viewport. Search is not a `SelectorItem`; the
  list starts focused on `selected_id` (or its first item) so digits remain immediate
  shortcuts. Up from the top item focuses search, Down from search returns to the
  first filtered item, and typing a non-digit from the list focuses and seeds search.
  Filtering is case-insensitive substring matching over row labels; an empty result
  renders `No matches`. The pinned search editor remains visible while rows scroll.
- Status rows appended after the list per `OptionSourceStatus`: `Loading…` (dim),
  `Failed { message }` (error style, plus a selectable `↻ Retry` virtual row that
  emits `RetryRequested`), and `Empty { message }` (dim). Status rows are not
  navigable.
- Footer: `OptionFooter::CustomText { label }` appends a selectable entry that, when
  confirmed, embeds a one-line `TuiEditorView` in place of the entry.
  Submitting a value replaces the generic footer label with that value, keeps the
  footer highlighted, and pre-fills the value when it is edited again. A selected id
  not present in the fixed rows restores this custom value when a page is rebuilt.
  `OptionFooter::CreateNewAuthSecret` is ignored (resource creation is out of scope
  in the TUI).

State/API surface for the embedding host:

- `new(ctx)` then `set_page(header, snapshot, searchable, ctx)` — resets the
  search query and highlight to the
  snapshot's `selected_id` (falling back to the first item) and discards any
  in-progress custom-text editing.
- `refresh_snapshot(snapshot, ctx)` — in-place catalog refresh preserving the
  highlighted row when it still exists, else falling back to `selected_id`.
- `confirm_highlighted(ctx)` — the host's Enter path: enabled rows emit
  `TuiOptionSelectorEvent::Confirmed { id }`; disabled rows stay highlighted so their
  reason remains visible; while the custom-text editor is active it validates
  (trimmed, non-empty — else an inline "Enter a value to continue." error) and emits
  `CustomTextSubmitted { value }`.
- `handle_back(ctx) -> bool` — the host's Escape path: cancels active custom-text
  editing and reports whether the key was consumed, so the host only leaves the page
  when the selector had nothing to unwind.
- `is_editing_custom_text()` — lets the host suppress its own keymap while typing.

Focus and element-level input (via the private `SelectorInputElement` wrapper, active only
while the selector is rendered as the blocking interaction):
- The list and embedded editors are real focus zones. `set_page` focuses the selector;
  boundary arrows move focus between the selector and search editor. Card keybindings
  continue to resolve through the editor's ancestor responder chain.
- Up/Down move the highlight, scrolling to keep it visible; Up from the top row focuses
  search and Down from search restores the first filtered row.
- Digits 1-9 confirm the corresponding visible row — viewport-relative, so digit 1 is
  always the top visible row after scrolling. While search owns focus, digits are
  editor input instead.
- Row clicks confirm (or highlight, when disabled) via per-item persistent
  `MouseStateHandle`s (owned by the view, per the mouse-state ownership rule).
- Wheel scrolling moves the viewport without moving the highlight.
- Search and custom text use the shared `TuiEditorView`; printable characters, cursor,
  selection, paste, and model-backed editing come from `CodeEditorModel` and
  `TuiEditorElement`. Single-line paste inserts only its first line. Escape remains
  selector/card policy: it clears a non-empty search first, cancels custom editing,
  or leaves the page.
- An element-level Escape fallback emits `Dismissed` for hosts without their own
  Escape binding; the embedding card's keymap normally consumes Escape first.

Selection reuses `InlineMenuSelection` and `keep_selected_visible` from
`crates/warp_tui/src/inline_menu.rs`.

### `crates/warp_tui/src/editor_view.rs`

`TuiEditorView::single_line` is the TUI analogue of the GUI's
`EditorView::single_line`, used by `FilterableDropdown`:

- Owns a char-cell `CodeEditorModel` and renders the existing `TuiEditorElement`.
- Tracks `focused` via `TuiView::on_focus` / `on_blur` and snapshots it into the
  editor element, following GUI `EditorView::focused` → `view_snapshot.is_focused`
  prior art.
- Exposes `text`, `set_text`, `is_focused`, and `Changed` events without owning
  form-specific labels, errors, or filtering behavior.
- Applies the same editor actions as `TuiInputView` for insertion, paste, mouse
  selection, and Backspace. The selector owns `Search:` / custom-host labels and
  validation chrome around the generic child view.
- Defines one shared editor-command binding table used by both `TuiInputView` and
  `TuiEditorView`, preserving each consumer's stable `tui:input:*` /
  `tui:editor:*` names and concrete action type. Common horizontal/word/line
  movement, deletion, selection, undo, and redo keys are specified once. Vertical
  movement, Enter, Escape, Tab, and kill/yank remain input/host policy so search
  continues to propagate Up/Down to the selector.
- Mouse-originated selection actions focus the editor before applying cursor or
  selection changes, matching GUI editor mouse-down behavior; the outer hoverable
  remains as the first-mouse fallback.

### `crates/warp_tui/src/tui_builder.rs`

Adds `orchestration_option_selected_style()`: bold, full-strength magenta text for
the selected option. The card slice adds its orchestration surface background and
remaining recipes (title glyph, selected metadata values, identity palette) itself.

### `crates/warp_tui/src/lib.rs`

Declares `mod option_selector` with a narrowly-scoped, commented
`#[allow(dead_code)]` on the module declaration, since nothing consumes the selector
until the card slice; that slice removes the allow.

## Testing and validation

- `crates/warp_tui/src/option_selector_tests.rs` covers: header/position/question
  rendering and initial highlight from `selected_id`; Up/Down + Enter confirmation;
  digit confirmation, including viewport-relative digits in scrolled lists; scrolling
  to keep the highlight visible with overflow markers; disabled rows being
  highlightable but not confirmable via Enter, digit, or click; Loading/Empty status
  rows being non-selectable; the Failed state's keyboard-reachable Retry row;
  custom-text trim/validate/submit, submitted-value rendering/re-editing/restoration,
  and Backspace; Back cancelling custom-text editing before leaving the page; the
  ignored `CreateNewAuthSecret` footer; snapshot-refresh
  highlight preservation and selected-value fallback; badge rendering; and paste being
  consumed only while the custom-text editor is active (first line only);
  searchable pages starting on the selected row; boundary focus handoff; numeric
  shortcuts remaining active from the list; digit-containing queries; filtering,
  no-match rendering, first-match confirmation, and clear-on-Escape.
- `crates/warp_tui/src/editor_view_tests.rs` covers single-line model editing and
  view-owned focus transitions, shared Ctrl+A/Home registration for both editor
  consumers, line-start command behavior, and mouse-selection focus independently
  of selector chrome.
- Tests host the selector under `test_fixtures::TestHostView` in a headless TUI
  window and render to lines (see the `tui-testing` conventions).
- Commands: `cargo check -p warp_tui`,
  `cargo nextest run -p warp_tui -E 'test(option_selector)'`, plus `./script/format`.

## Follow-ups

The TUI orchestration card slice embeds `TuiOptionSelector` for its configuration
pages (host, environment, harness, model, API key, location), adds the remaining
orchestration theming recipes, and removes the module-level `allow(dead_code)`.

# TECH: Reusable TUI option selector over shared option snapshots

## Context

This slice builds on the frontend-neutral orchestration option snapshots
(base commit `aaf62142`; see `specs/code-1822-option-snapshots/TECH.md` for the data
contract). At that base, `app/src/tui_export.rs` re-exports `OptionSnapshot`,
`OptionRow`, `OptionBadge`, `OptionSourceStatus`, and `OptionFooter`, but nothing in
`crates/warp_tui` renders them: the TUI has no single-select list primitive.

This PR adds exactly that primitive — `TuiOptionSelector` — with no consumer yet. The
next slice (the TUI orchestration permission/configuration card) embeds it to render
its per-field configuration pages; the same primitive is intended for future
AskUserQuestion and permission prompts, which is why it is snapshot-driven and knows
nothing about orchestration edit state.

## Proposed changes

### `crates/warp_tui/src/option_selector.rs`

A `TuiView` + `TypedActionView` (`TuiOptionSelector`) rendering one page:

- Header (`OptionSelectorHeader`): bold title, a muted one-based "n of m" position in
  the host's page sequence, and the page's question.
- Option list rendered from an `OptionSnapshot` (`warp::tui_export`): up to
  `MAX_VISIBLE_OPTION_ROWS` (8) rows visible at once with `↑ more` / `↓ more` overflow
  markers. Rows show a `❯` highlight marker, a viewport-relative digit prefix (1-9),
  the label, an optional badge suffix (`(default)` / `(recent)` / `(connected)`), and
  — for disabled rows — the `disabled_reason`. The highlighted row is tinted with
  `TuiUiBuilder::orchestration_surface_background`.
- Status rows appended after the list per `OptionSourceStatus`: `Loading…` (dim),
  `Failed { message }` (error style, plus a selectable `↻ Retry` virtual row that
  emits `RetryRequested`), and `Empty { message }` (dim). Status rows are not
  navigable.
- Footer: `OptionFooter::CustomText { label }` appends a selectable entry that, when
  confirmed, opens a one-line custom-text editor rendered in place of the entry.
  `OptionFooter::CreateNewAuthSecret` is ignored (resource creation is out of scope
  in the TUI).

State/API surface for the embedding host:

- `new()` then `set_page(header, snapshot, ctx)` — resets the highlight to the
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

Element-level input (via the private `SelectorInputElement` wrapper, active only
while the selector is rendered as the blocking interaction):

- Up/Down move the highlight, scrolling to keep it visible.
- Digits 1-9 confirm the corresponding visible row — viewport-relative, so digit 1 is
  always the top visible row after scrolling.
- Row clicks confirm (or highlight, when disabled) via per-item persistent
  `MouseStateHandle`s (owned by the view, per the mouse-state ownership rule).
- Wheel scrolling moves the viewport without moving the highlight.
- While the custom-text editor is active: printable characters append, Backspace
  deletes, Escape cancels, and `TuiEvent::Paste` inserts the first line's printable
  characters (the editor is single-line).
- An element-level Escape fallback emits `Dismissed` for hosts without their own
  Escape binding; the embedding card's keymap normally consumes Escape first.

Selection reuses `InlineMenuSelection` and `keep_selected_visible` from
`crates/warp_tui/src/inline_menu.rs`.

### `crates/warp_tui/src/tui_builder.rs`

Adds one style recipe the selector renders with:
`orchestration_surface_background()` — the terminal palette's normal magenta at 2×10%
opacity pre-blended over the probed base background, mirroring `input_background`'s
accent recipe. The card slice adds its remaining orchestration recipes (title style,
selected-value style, identity palette) itself; only the method the selector calls
lands here, keeping this PR self-contained.

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
  custom-text trim/validate/submit and Backspace; Back cancelling custom-text editing
  before leaving the page; the ignored `CreateNewAuthSecret` footer; snapshot-refresh
  highlight preservation and selected-value fallback; badge rendering; and paste being
  consumed only while the custom-text editor is active (first line only).
- Tests host the selector under `test_fixtures::TestHostView` in a headless TUI
  window and render to lines (see the `tui-testing` conventions).
- Commands: `cargo check -p warp_tui`,
  `cargo nextest run -p warp_tui -E 'test(option_selector)'`, plus `./script/format`.

## Follow-ups

The TUI orchestration card slice embeds `TuiOptionSelector` for its configuration
pages (host, environment, harness, model, API key, location), adds the remaining
orchestration theming recipes, and removes the module-level `allow(dead_code)`.

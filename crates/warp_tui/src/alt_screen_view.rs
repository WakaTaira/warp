//! Full-screen alt-screen rendering + raw input forwarding for the TUI.
//!
//! When a PTY app switches to the alternate screen (vim, htop, less, …), the
//! terminal model flips [`TerminalModel::is_alt_screen_active`] and populates a
//! dedicated alt-screen grid. [`TuiTerminalSessionView`] then renders this
//! element full-area instead of the block/transcript UI, and forwards
//! input straight to the PTY as escape sequences — mirroring the GUI's
//! `AltScreenElement` (`app/src/terminal/alt_screen/alt_screen_element.rs`).
//!
//! Covers rendering, the cursor, and keyboard and SGR mouse forwarding. Mouse
//! forwarding is gated by the same policy the GUI uses
//! (`should_intercept_mouse` / `should_intercept_scroll`), so terminal modes,
//! reporting settings, and shared-session state behave identically across
//! front-ends. PTY sizing is handled by the session view's
//! `TuiTerminalSizeElement` wrapper, which publishes this element's laid-out
//! dimensions after every layout.
//!
//! [`TuiTerminalSessionView`]: crate::terminal_session_view::TuiTerminalSessionView
//! [`TerminalModel::is_alt_screen_active`]: warp::tui_export::TerminalModel

use std::ops::Deref as _;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    should_intercept_mouse, should_intercept_scroll, KeystrokeWithDetails, TermMode, TerminalModel,
    ToEscapeSequence as _,
};
use warp_terminal::model::escape_sequences::{alt_screen_scroll_to_pty_bytes, ModeProvider};
use warp_terminal::model::grid::Dimensions as _;
use warp_terminal::model::mouse::{MouseAction, MouseButton, MouseState};
use warp_terminal::model::Point;
use warpui_core::elements::tui::{
    TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiPoint, TuiScreenPoint, TuiScreenPosition, TuiScreenRect, TuiSize,
};
use warpui_core::AppContext;

use crate::terminal_block::render_grid_handler;
use crate::terminal_session_view::TuiTerminalSessionAction;

/// Renders the terminal's alt-screen grid full-area and forwards input to the
/// PTY while a full-screen app is active.
pub(crate) struct AltScreenElement {
    model: Arc<FairMutex<TerminalModel>>,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl AltScreenElement {
    pub(crate) fn new(model: Arc<FairMutex<TerminalModel>>) -> Self {
        Self {
            model,
            size: None,
            origin: None,
        }
    }
}

/// Which alt-screen mouse reports the active app and user settings allow.
#[derive(Clone, Copy)]
pub(crate) struct MouseReportPolicy {
    /// Clicks, releases, and drags.
    report_buttons: bool,
    /// Hover motion.
    report_motion: bool,
    /// Wheel events as SGR reports (arrow-key fallback otherwise).
    report_scroll: bool,
}

impl MouseReportPolicy {
    /// Derives the policy from the same gating the GUI's `AltScreenElement`
    /// applies: `should_intercept_*` for buttons and scroll (per-event shift
    /// is checked at the call site), and `MOUSE_MOTION` for hover motion.
    fn current(model: &TerminalModel, app: &AppContext) -> Self {
        Self {
            report_buttons: !should_intercept_mouse(model, false, app),
            report_motion: model.is_term_mode_set(TermMode::MOUSE_MOTION),
            report_scroll: !should_intercept_scroll(model, app),
        }
    }
}

/// Converts an in-bounds screen position to alt-screen grid coordinates.
fn cell_point(position: TuiPoint, bounds: TuiScreenRect) -> Option<Point> {
    if !bounds.contains(position) {
        return None;
    }
    Some(Point::new(
        usize::try_from(i32::from(position.y) - bounds.origin.y).ok()?,
        usize::try_from(i32::from(position.x) - bounds.origin.x).ok()?,
    ))
}

/// Encodes a supported pointer event for the active alt-screen application.
fn mouse_event_to_pty_bytes<T: ModeProvider>(
    event: &TuiEvent,
    bounds: TuiScreenRect,
    policy: MouseReportPolicy,
    mode_provider: &T,
) -> Option<Vec<u8>> {
    let point = cell_point(event.position()?, bounds)?;
    let state = match event {
        TuiEvent::ScrollWheel {
            delta: (_, rows), ..
        } => {
            return alt_screen_scroll_to_pty_bytes(
                i32::try_from(*rows).ok()?,
                point,
                policy.report_scroll,
                mode_provider,
            );
        }
        TuiEvent::LeftMouseDown { modifiers, .. } if policy.report_buttons && !modifiers.shift => {
            MouseState::new(MouseButton::Left, MouseAction::Pressed, *modifiers)
        }
        TuiEvent::RightMouseDown { modifiers, .. } if policy.report_buttons && !modifiers.shift => {
            MouseState::new(MouseButton::Right, MouseAction::Pressed, *modifiers)
        }
        TuiEvent::LeftMouseUp { modifiers, .. } if policy.report_buttons && !modifiers.shift => {
            MouseState::new(MouseButton::Left, MouseAction::Released, *modifiers)
        }
        TuiEvent::LeftMouseDragged { modifiers, .. }
            if policy.report_buttons && !modifiers.shift =>
        {
            MouseState::new(MouseButton::LeftDrag, MouseAction::Pressed, *modifiers)
        }
        TuiEvent::MouseMoved {
            modifiers,
            is_synthetic: false,
            ..
        } if policy.report_motion => {
            MouseState::new(MouseButton::Move, MouseAction::Pressed, *modifiers)
        }
        _ => return None,
    };
    state.set_point(point).to_escape_sequence(mode_provider)
}

impl TuiElement for AltScreenElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        // The alt-screen app owns the whole pane.
        let size = constraint.max;
        self.size = Some(size);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.origin = Some(ctx.scene_point(origin));
        let Some(size) = self.size else {
            return;
        };
        let model = self.model.lock();
        let colors = model.colors();
        let alt = model.alt_screen();
        render_grid_handler(alt.grid_handler(), origin, size, surface, &colors);

        // Submit the hardware cursor if the alt-screen app is showing it. The
        // alt screen has no scrollback, but subtract history defensively so the
        // cursor maps to a visible (screen-relative) row.
        let cursor = if alt.is_mode_set(TermMode::SHOW_CURSOR) {
            let grid = alt.grid_handler();
            let point = grid.cursor_render_point();
            point.row.checked_sub(grid.history_size()).and_then(|row| {
                let col = u16::try_from(point.col).ok()?;
                let row = u16::try_from(row).ok()?;
                (col < size.width && row < size.height).then_some((col, row))
            })
        } else {
            None
        };
        drop(model);
        if let Some((col, row)) = cursor {
            let cursor_point = ctx.scene_point(origin.offset(i32::from(col), i32::from(row)));
            ctx.set_terminal_cursor(cursor_point);
        }
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        // Forward the event to the app. Keys go through `to_pty_bytes`, which
        // layers the fallbacks a single-`KeyDown` frontend needs —
        // `Ctrl+<letter>` → C0, printable `chars`, and named control keys — on
        // top of the shared `to_escape_sequence` encoder in `warp_terminal`.
        // (ctrl-c never reaches here: the session view's interrupt handler
        // forwards it to the app.) Pointer events are translated to SGR mouse
        // reports when the GUI-shared reporting policy allows it.
        let bytes = {
            let model = self.model.lock();
            match event {
                TuiEvent::KeyDown {
                    keystroke,
                    chars,
                    details,
                    is_composing: false,
                } => KeystrokeWithDetails {
                    keystroke,
                    key_without_modifiers: details.key_without_modifiers.as_deref(),
                    chars: Some(chars.as_str()),
                }
                .to_pty_bytes(model.deref()),
                TuiEvent::KeyDown {
                    is_composing: true, ..
                } => None,
                _ => self.origin.zip(self.size).and_then(|(origin, size)| {
                    mouse_event_to_pty_bytes(
                        event,
                        TuiScreenRect::new(origin, size),
                        MouseReportPolicy::current(model.deref(), app),
                        model.deref(),
                    )
                }),
            }
        };
        let Some(bytes) = bytes else {
            return false;
        };
        event_ctx.dispatch_typed_action(TuiTerminalSessionAction::ForwardToPty(bytes));
        true
    }
}

#[cfg(test)]
#[path = "alt_screen_view_tests.rs"]
mod tests;

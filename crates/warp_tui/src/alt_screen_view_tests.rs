use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{TermMode, TerminalModel};
use warp_terminal::model::escape_sequences::ModeProvider;
use warpui::EntityIdMap;
use warpui_core::elements::tui::{
    TuiConstraint, TuiElement, TuiEvent, TuiLayoutContext, TuiPoint, TuiScreenPoint, TuiScreenRect,
    TuiSize, TuiZIndex,
};
use warpui_core::event::ModifiersState;
use warpui_core::App;

use super::{mouse_event_to_pty_bytes, AltScreenElement, MouseReportPolicy};

/// Builds retained screen bounds anchored at `(x, y)`.
fn bounds(x: i32, y: i32, width: u16, height: u16) -> TuiScreenRect {
    TuiScreenRect::new(
        TuiScreenPoint::new(x, y, TuiZIndex::Normal(0)),
        TuiSize::new(width, height),
    )
}

/// Builds a reporting policy with the given per-category flags.
fn policy(buttons: bool, motion: bool, scroll: bool) -> MouseReportPolicy {
    MouseReportPolicy {
        report_buttons: buttons,
        report_motion: motion,
        report_scroll: scroll,
    }
}

/// A policy allowing every report category.
fn report_all() -> MouseReportPolicy {
    policy(true, true, true)
}

/// Supplies no terminal modes; mouse SGR encoding does not consult them.
struct MouseModeProvider;

impl ModeProvider for MouseModeProvider {
    fn is_term_mode_set(&self, _mode: TermMode) -> bool {
        false
    }
}

/// Encodes `event` using the production TUI mouse-event adapter.
fn mouse_bytes(
    event: &TuiEvent,
    area: TuiScreenRect,
    policy: MouseReportPolicy,
) -> Option<Vec<u8>> {
    mouse_event_to_pty_bytes(event, area, policy, &MouseModeProvider)
}

#[test]
fn layout_fills_the_allocated_pane() {
    App::test((), |app| async move {
        app.read(|app| {
            let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
            let mut element = AltScreenElement::new(model);
            let expected_size = TuiSize::new(42, 8);
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            let size = element.layout(TuiConstraint::loose(expected_size), &mut layout_ctx, app);
            assert_eq!(size, expected_size);
            // The laid-out size is retained for the size wrapper and mouse
            // hit-testing to read back.
            assert_eq!(element.size(), Some(expected_size));
        });
    });
}

#[test]
fn sgr_mouse_events_use_area_relative_coordinates() {
    let area = bounds(10, 5, 20, 10);
    let position = TuiPoint::new(12, 6);
    let modifiers = ModifiersState::default();
    let cases = [
        (
            TuiEvent::LeftMouseDown {
                position,
                modifiers,
                click_count: 1,
                is_first_mouse: false,
            },
            b"\x1b[<0;3;2M".as_slice(),
        ),
        (
            TuiEvent::RightMouseDown {
                position,
                modifiers,
                click_count: 1,
            },
            b"\x1b[<2;3;2M".as_slice(),
        ),
        (
            TuiEvent::LeftMouseUp {
                position,
                modifiers,
            },
            b"\x1b[<0;3;2m".as_slice(),
        ),
        (
            TuiEvent::LeftMouseDragged {
                position,
                modifiers,
            },
            b"\x1b[<32;3;2M".as_slice(),
        ),
        (
            TuiEvent::MouseMoved {
                position,
                modifiers,
                is_synthetic: false,
            },
            b"\x1b[<35;3;2M".as_slice(),
        ),
        (
            TuiEvent::ScrollWheel {
                position,
                delta: (0, 1),
                precise: false,
                modifiers,
            },
            b"\x1b[<64;3;2M".as_slice(),
        ),
        (
            TuiEvent::ScrollWheel {
                position,
                delta: (0, -1),
                precise: false,
                modifiers,
            },
            b"\x1b[<65;3;2M".as_slice(),
        ),
    ];

    for (event, expected) in cases {
        assert_eq!(
            mouse_bytes(&event, area, report_all()).as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn events_are_gated_by_their_policy_category() {
    let area = bounds(0, 0, 10, 10);
    let position = TuiPoint::new(2, 3);
    let modifiers = ModifiersState::default();
    let left_down = TuiEvent::LeftMouseDown {
        position,
        modifiers,
        click_count: 1,
        is_first_mouse: false,
    };
    let left_dragged = TuiEvent::LeftMouseDragged {
        position,
        modifiers,
    };
    let moved = TuiEvent::MouseMoved {
        position,
        modifiers,
        is_synthetic: false,
    };

    // Buttons (clicks and drags) follow `report_buttons`; motion is
    // independent and follows `report_motion`.
    assert!(mouse_bytes(&left_down, area, policy(false, true, true)).is_none());
    assert!(mouse_bytes(&left_down, area, policy(true, false, false)).is_some());
    assert!(mouse_bytes(&left_dragged, area, policy(false, true, true)).is_none());
    assert!(mouse_bytes(&left_dragged, area, policy(true, false, false)).is_some());
    assert!(mouse_bytes(&moved, area, policy(true, false, true)).is_none());
    assert!(mouse_bytes(&moved, area, policy(false, true, false)).is_some());
}

#[test]
fn scroll_uses_sgr_when_allowed_and_arrows_otherwise() {
    let scroll = TuiEvent::ScrollWheel {
        position: TuiPoint::new(2, 3),
        delta: (0, 1),
        precise: false,
        modifiers: ModifiersState::default(),
    };
    let area = bounds(0, 0, 10, 10);

    assert_eq!(
        mouse_bytes(&scroll, area, policy(false, false, false)).as_deref(),
        Some(b"\x1bOA".as_slice())
    );
    assert_eq!(
        mouse_bytes(&scroll, area, policy(false, false, true)).as_deref(),
        Some(b"\x1b[<64;3;4M".as_slice())
    );
}

#[test]
fn unsupported_or_intercepted_mouse_events_are_not_forwarded() {
    let area = bounds(5, 5, 10, 10);
    let modifiers = ModifiersState::default();

    let outside = TuiEvent::LeftMouseDown {
        position: TuiPoint::new(4, 5),
        modifiers,
        click_count: 1,
        is_first_mouse: false,
    };
    let shifted = TuiEvent::LeftMouseDown {
        position: TuiPoint::new(6, 6),
        modifiers: ModifiersState {
            shift: true,
            ..Default::default()
        },
        click_count: 1,
        is_first_mouse: false,
    };
    let middle = TuiEvent::MiddleMouseDown {
        position: TuiPoint::new(6, 6),
        modifiers,
        click_count: 1,
    };
    let synthetic_move = TuiEvent::MouseMoved {
        position: TuiPoint::new(6, 6),
        modifiers,
        is_synthetic: true,
    };
    let horizontal_scroll = TuiEvent::ScrollWheel {
        position: TuiPoint::new(6, 6),
        delta: (1, 0),
        precise: false,
        modifiers,
    };

    assert!(mouse_bytes(&outside, area, report_all()).is_none());
    assert!(mouse_bytes(&shifted, area, report_all()).is_none());
    assert!(mouse_bytes(&middle, area, report_all()).is_none());
    assert!(mouse_bytes(&synthetic_move, area, report_all()).is_none());
    assert!(mouse_bytes(&horizontal_scroll, area, report_all()).is_none());
}

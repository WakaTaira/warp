use std::cell::RefCell;
use std::rc::Rc;

use warp::tui_export::{
    Appearance, OptionBadge, OptionFooter, OptionRow, OptionSnapshot, OptionSourceStatus,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, EntityId, EntityIdMap};
use warpui_core::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPaintContext, TuiRect, TuiSize,
};
use warpui_core::{App, TuiView as _, TypedActionView as _, ViewHandle};

use super::{
    OptionSelectorHeader, TuiOptionSelector, TuiOptionSelectorAction, TuiOptionSelectorEvent,
};
use crate::test_fixtures::TestHostView;

/// Builds an enabled row with `id` used as the label.
fn row(id: &str) -> OptionRow {
    OptionRow {
        id: id.to_string(),
        label: id.to_string(),
        harness: None,
        badge: None,
        disabled_reason: None,
    }
}

/// Builds a disabled row carrying `reason`.
fn disabled_row(id: &str, reason: &str) -> OptionRow {
    OptionRow {
        disabled_reason: Some(reason.to_string()),
        ..row(id)
    }
}

/// Builds a `Ready` snapshot over `ids` with `selected` preselected.
fn snapshot(ids: &[&str], selected: Option<&str>) -> OptionSnapshot {
    snapshot_of(ids.iter().map(|id| row(id)).collect(), selected)
}

/// Builds a `Ready` snapshot from explicit rows.
fn snapshot_of(rows: Vec<OptionRow>, selected: Option<&str>) -> OptionSnapshot {
    OptionSnapshot {
        rows,
        selected_id: selected.map(str::to_string),
        status: OptionSourceStatus::Ready,
        footer: None,
    }
}

/// A page header used across tests.
fn header() -> OptionSelectorHeader {
    OptionSelectorHeader {
        title: "Host".to_string(),
        position: (4, 6),
        question: "Which host should run the agents?".to_string(),
    }
}

type CapturedEvents = Rc<RefCell<Vec<TuiOptionSelectorEvent>>>;

/// Adds a selector in a fresh TUI window and captures its emitted events.
fn add_selector(app: &mut App) -> (ViewHandle<TuiOptionSelector>, CapturedEvents) {
    app.add_singleton_model(|_| Appearance::mock());
    let selector = app.update(|ctx| {
        let (window_id, _) = ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| TestHostView,
        );
        ctx.add_typed_action_tui_view(window_id, |_| TuiOptionSelector::new())
    });
    let events: CapturedEvents = Rc::new(RefCell::new(Vec::new()));
    let events_for_subscription = events.clone();
    app.update(|ctx| {
        ctx.subscribe_to_view(&selector, move |_, event, _| {
            events_for_subscription.borrow_mut().push(event.clone());
        });
    });
    (selector, events)
}

/// Sets the page under the shared test header.
fn set_page(app: &mut App, selector: &ViewHandle<TuiOptionSelector>, snapshot: OptionSnapshot) {
    selector.update(app, |selector, ctx| {
        selector.set_page(header(), snapshot, ctx);
    });
}

#[test]
fn set_page_recovers_a_selected_custom_text_value() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        let mut with_custom_selection = snapshot(&["warp"], Some("my-host"));
        with_custom_selection.footer = Some(OptionFooter::CustomText {
            label: "Custom host…".to_string(),
        });

        set_page(&mut app, &selector, with_custom_selection);

        let line = highlighted_line(&app, &selector);
        assert!(line.contains("my-host"));
        assert!(!line.contains("Custom host…"));
    });
}

/// Dispatches a selector action directly to the view.
fn act(app: &mut App, selector: &ViewHandle<TuiOptionSelector>, action: TuiOptionSelectorAction) {
    selector.update(app, |selector, ctx| selector.handle_action(&action, ctx));
}

/// Confirms the highlighted item (the card's Enter path).
fn confirm(app: &mut App, selector: &ViewHandle<TuiOptionSelector>) {
    selector.update(app, |selector, ctx| selector.confirm_highlighted(ctx));
}

/// Renders the selector to trimmed lines at `width`.
fn render_lines(app: &App, selector: &ViewHandle<TuiOptionSelector>, width: u16) -> Vec<String> {
    app.read(|app| {
        let mut rendered_views = EntityIdMap::default();
        let mut layout_ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        let mut element = selector.as_ref(app).render(app);
        let size = element.layout(
            TuiConstraint::loose(TuiSize::new(width, u16::MAX)),
            &mut layout_ctx,
            app,
        );
        let area = TuiRect::new(0, 0, size.width.max(1), size.height.max(1));
        let mut buffer = TuiBuffer::empty(area);
        let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
        element.render(area, &mut buffer, &mut paint_ctx);
        buffer
            .to_lines()
            .into_iter()
            .map(|line| line.trim_end().to_owned())
            .collect()
    })
}

/// The rendered line containing the `❯` highlight marker.
fn highlighted_line(app: &App, selector: &ViewHandle<TuiOptionSelector>) -> String {
    render_lines(app, selector, 60)
        .into_iter()
        .find(|line| line.contains('❯'))
        .expect("a highlighted row")
}

#[test]
fn renders_header_position_question_and_initial_highlight() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        set_page(&mut app, &selector, snapshot(&["a", "b", "c"], Some("b")));
        let lines = render_lines(&app, &selector, 60);
        // Header: title, position in the current sequence, and the question.
        assert!(lines[0].contains("Host"));
        assert!(lines[0].contains("4 of 6"));
        assert!(lines[1].contains("Which host should run the agents?"));
        // The highlight starts on the snapshot's current value.
        assert!(highlighted_line(&app, &selector).contains('b'));
    });
}

#[test]
fn up_and_down_move_the_highlight_and_enter_confirms_it() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        set_page(&mut app, &selector, snapshot(&["a", "b", "c"], Some("a")));
        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        assert!(highlighted_line(&app, &selector).contains('b'));
        act(&mut app, &selector, TuiOptionSelectorAction::MoveUp);
        assert!(highlighted_line(&app, &selector).contains('a'));
        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        confirm(&mut app, &selector);
        assert_eq!(
            events.borrow().as_slice(),
            [TuiOptionSelectorEvent::Confirmed {
                id: "b".to_string()
            }],
        );
    });
}

#[test]
fn digits_confirm_the_corresponding_visible_row() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        set_page(&mut app, &selector, snapshot(&["a", "b", "c"], Some("a")));
        act(
            &mut app,
            &selector,
            TuiOptionSelectorAction::SelectNumberedOption(3),
        );
        assert_eq!(
            events.borrow().as_slice(),
            [TuiOptionSelectorEvent::Confirmed {
                id: "c".to_string()
            }],
        );
    });
}

#[test]
fn digits_are_viewport_relative_in_scrolled_lists() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        let ids: Vec<String> = (0..12).map(|i| format!("row-{i}")).collect();
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        set_page(&mut app, &selector, snapshot(&id_refs, Some("row-0")));
        // Scroll two rows down; digit 1 now confirms the third row
        //, and the clipped top renders an overflow marker.
        act(&mut app, &selector, TuiOptionSelectorAction::ScrollBy(2));
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .any(|line| line.contains("↑ more")));
        act(
            &mut app,
            &selector,
            TuiOptionSelectorAction::SelectNumberedOption(1),
        );
        assert_eq!(
            events.borrow().as_slice(),
            [TuiOptionSelectorEvent::Confirmed {
                id: "row-2".to_string()
            }],
        );
    });
}

#[test]
fn navigation_scrolls_to_keep_the_highlight_visible() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        let ids: Vec<String> = (0..12).map(|i| format!("row-{i}")).collect();
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        set_page(&mut app, &selector, snapshot(&id_refs, Some("row-0")));
        for _ in 0..9 {
            act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        }
        // The highlight scrolled beyond the first viewport.
        assert!(highlighted_line(&app, &selector).contains("row-9"));
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .any(|line| line.contains("↑ more")));
    });
}

#[test]
fn disabled_rows_are_highlightable_but_not_confirmable() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        set_page(
            &mut app,
            &selector,
            snapshot_of(
                vec![
                    row("a"),
                    disabled_row("b", "Disabled by your administrator"),
                ],
                Some("a"),
            ),
        );
        // The disabled row can be highlighted and shows its reason
        // …
        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        let line = highlighted_line(&app, &selector);
        assert!(line.contains('b'));
        assert!(line.contains("Disabled by your administrator"));
        // … but neither Enter, its digit, nor a click confirms it.
        confirm(&mut app, &selector);
        act(
            &mut app,
            &selector,
            TuiOptionSelectorAction::SelectNumberedOption(2),
        );
        act(&mut app, &selector, TuiOptionSelectorAction::SelectItem(1));
        assert!(events.borrow().is_empty());
    });
}

#[test]
fn loading_and_empty_states_render_non_selectable_status_rows() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        let mut loading = snapshot(&["Skip (advanced)"], None);
        loading.status = OptionSourceStatus::Loading;
        set_page(&mut app, &selector, loading);
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .any(|line| line.contains("Loading…")));

        let mut empty = snapshot_of(Vec::new(), None);
        empty.status = OptionSourceStatus::Empty {
            message: "No harnesses available".to_string(),
        };
        set_page(&mut app, &selector, empty);
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .any(|line| line.contains("No harnesses available")));
        // Nothing is confirmable in an empty list.
        confirm(&mut app, &selector);
        assert!(events.borrow().is_empty());
    });
}

#[test]
fn failed_state_offers_a_retry_row_that_emits_retry_requested() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        let mut failed = snapshot(&["Skip (advanced)"], None);
        failed.status = OptionSourceStatus::Failed {
            message: "Unable to load secrets".to_string(),
        };
        set_page(&mut app, &selector, failed);
        let lines = render_lines(&app, &selector, 60);
        assert!(lines
            .iter()
            .any(|line| line.contains("Unable to load secrets")));
        assert!(lines.iter().any(|line| line.contains("Retry")));
        // The Retry row is reachable by keyboard.
        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        confirm(&mut app, &selector);
        assert_eq!(
            events.borrow().as_slice(),
            [TuiOptionSelectorEvent::RetryRequested],
        );
    });
}

#[test]
fn custom_text_editor_trims_validates_and_submits() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        let mut with_footer = snapshot(&["warp"], Some("warp"));
        with_footer.footer = Some(OptionFooter::CustomText {
            label: "Custom host…".to_string(),
        });
        set_page(&mut app, &selector, with_footer);
        // The footer renders and confirming it opens the one-line editor.
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .any(|line| line.contains("Custom host…")));
        act(&mut app, &selector, TuiOptionSelectorAction::SelectItem(1));
        assert!(app.read(|app| selector.as_ref(app).is_editing_custom_text()));

        // Whitespace-only input stays editable with a concise error
        //.
        act(
            &mut app,
            &selector,
            TuiOptionSelectorAction::InsertChar(' '),
        );
        confirm(&mut app, &selector);
        assert!(events.borrow().is_empty());
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .any(|line| line.contains("Enter a value to continue.")));

        // Valid input is trimmed and submitted.
        for c in "my-host ".chars() {
            act(&mut app, &selector, TuiOptionSelectorAction::InsertChar(c));
        }
        act(&mut app, &selector, TuiOptionSelectorAction::Backspace);
        confirm(&mut app, &selector);
        assert_eq!(
            events.borrow().as_slice(),
            [TuiOptionSelectorEvent::CustomTextSubmitted {
                value: "my-host".to_string()
            }],
        );
        assert!(app.read(|app| !selector.as_ref(app).is_editing_custom_text()));
        let line = highlighted_line(&app, &selector);
        assert!(line.contains("my-host"));
        assert!(!line.contains("Custom host…"));

        // Editing the custom option again starts from the submitted value.
        confirm(&mut app, &selector);
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .any(|line| line.contains("Custom host…: my-host▏")));
    });
}

#[test]
fn back_cancels_custom_text_editing_before_leaving_the_page() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        let mut with_footer = snapshot(&["warp"], Some("warp"));
        with_footer.footer = Some(OptionFooter::CustomText {
            label: "Custom host…".to_string(),
        });
        set_page(&mut app, &selector, with_footer);
        act(&mut app, &selector, TuiOptionSelectorAction::SelectItem(1));
        assert!(app.read(|app| selector.as_ref(app).is_editing_custom_text()));
        // The first Back unwinds editing and is consumed; the next one isn't.
        let consumed = selector.update(&mut app, |selector, ctx| selector.handle_back(ctx));
        assert!(consumed);
        assert!(app.read(|app| !selector.as_ref(app).is_editing_custom_text()));
        let consumed = selector.update(&mut app, |selector, ctx| selector.handle_back(ctx));
        assert!(!consumed);
    });
}

#[test]
fn create_new_auth_secret_footer_is_ignored() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        let mut with_footer = snapshot(&["Skip (advanced)"], None);
        with_footer.footer = Some(OptionFooter::CreateNewAuthSecret);
        set_page(&mut app, &selector, with_footer);
        // Resource creation is out of scope in the TUI: the
        // footer contributes no navigable item.
        assert!(render_lines(&app, &selector, 60)
            .iter()
            .all(|line| !line.contains("New API key")));
    });
}

#[test]
fn snapshot_refresh_preserves_the_highlighted_row() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        set_page(&mut app, &selector, snapshot(&["a", "b", "c"], Some("a")));
        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        // The highlighted row survives a catalog refresh that reorders rows
        //.
        selector.update(&mut app, |selector, ctx| {
            selector.refresh_snapshot(snapshot(&["c", "a"], Some("a")), ctx);
        });
        confirm(&mut app, &selector);
        assert_eq!(
            events.borrow().as_slice(),
            [TuiOptionSelectorEvent::Confirmed {
                id: "c".to_string()
            }],
        );
    });
}

#[test]
fn snapshot_refresh_falls_back_to_the_selected_value_when_the_highlight_vanishes() {
    App::test((), |mut app| async move {
        let (selector, events) = add_selector(&mut app);
        set_page(&mut app, &selector, snapshot(&["a", "b"], Some("a")));
        act(&mut app, &selector, TuiOptionSelectorAction::MoveDown);
        // "b" disappears from the catalog; the highlight falls back to the
        // snapshot's current value rather than silently confirming anything
        //.
        selector.update(&mut app, |selector, ctx| {
            selector.refresh_snapshot(snapshot(&["a", "x"], Some("a")), ctx);
        });
        confirm(&mut app, &selector);
        assert_eq!(
            events.borrow().as_slice(),
            [TuiOptionSelectorEvent::Confirmed {
                id: "a".to_string()
            }],
        );
    });
}

/// Dispatches `event` to the selector's freshly rendered element tree,
/// returning whether it was handled.
fn dispatch(app: &App, selector: &ViewHandle<TuiOptionSelector>, event: &TuiEvent) -> bool {
    app.read(|app| {
        let mut rendered_views = EntityIdMap::default();
        let mut layout_ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        let mut element = selector.as_ref(app).render(app);
        let size = element.layout(
            TuiConstraint::loose(TuiSize::new(60, u16::MAX)),
            &mut layout_ctx,
            app,
        );
        let area = TuiRect::new(0, 0, size.width.max(1), size.height.max(1));
        let mut event_ctx = TuiEventContext::default();
        event_ctx.set_origin_view(Some(EntityId::new()));
        element.dispatch_event(event, area, &mut event_ctx, &mut layout_ctx, app)
    })
}

#[test]
fn paste_is_consumed_only_while_editing_custom_text() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        let mut with_footer = snapshot(&["warp"], Some("warp"));
        with_footer.footer = Some(OptionFooter::CustomText {
            label: "Custom host…".to_string(),
        });
        set_page(&mut app, &selector, with_footer);
        let paste = TuiEvent::Paste {
            text: "my-host\nsecond line".to_string(),
        };
        // Ignored while the list (not the editor) is active.
        assert!(!dispatch(&app, &selector, &paste));

        act(&mut app, &selector, TuiOptionSelectorAction::SelectItem(1));
        assert!(app.read(|app| selector.as_ref(app).is_editing_custom_text()));
        // The editor consumes the paste (only the first line's printable
        // characters are inserted; the editor is single-line).
        assert!(dispatch(&app, &selector, &paste));
        // A paste with no printable first-line characters is not consumed.
        let control_only = TuiEvent::Paste {
            text: "\nsecond line".to_string(),
        };
        assert!(!dispatch(&app, &selector, &control_only));
    });
}

#[test]
fn badges_render_next_to_their_rows() {
    App::test((), |mut app| async move {
        let (selector, _) = add_selector(&mut app);
        let rows = vec![
            OptionRow {
                badge: Some(OptionBadge::Default),
                ..row("team-host")
            },
            OptionRow {
                badge: Some(OptionBadge::Connected),
                ..row("worker-1")
            },
            OptionRow {
                badge: Some(OptionBadge::Recent),
                ..row("old-host")
            },
        ];
        set_page(&mut app, &selector, snapshot_of(rows, Some("team-host")));
        let lines = render_lines(&app, &selector, 60);
        assert!(lines.iter().any(|line| line.contains("(default)")));
        assert!(lines.iter().any(|line| line.contains("(connected)")));
        assert!(lines.iter().any(|line| line.contains("(recent)")));
    });
}

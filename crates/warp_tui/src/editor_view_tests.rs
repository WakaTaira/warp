use std::collections::HashSet;

use string_offset::CharOffset;
use warp::tui_export::Appearance;
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, EntityIdMap};
use warpui_core::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiLayoutContext, TuiPaintContext, TuiPaintSurface,
    TuiRect, TuiScreenPosition, TuiSize,
};
use warpui_core::keymap::Trigger;
use warpui_core::{App, TuiView as _, TypedActionView as _};

use super::{TuiEditorCommand, TuiEditorView, TuiEditorViewAction};
use crate::editor_element::TuiEditorAction;
use crate::test_fixtures::TestHostView;

/// Renders an editor view to trimmed lines.
fn render_lines(app: &App, editor: &warpui_core::ViewHandle<TuiEditorView>) -> Vec<String> {
    app.read(|ctx| {
        let mut element = editor.as_ref(ctx).render(ctx);
        let mut rendered_views = EntityIdMap::default();
        let mut layout_ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        let size = element.layout(
            TuiConstraint::loose(TuiSize::new(30, 4)),
            &mut layout_ctx,
            ctx,
        );
        let area = TuiRect::new(0, 0, size.width, size.height);
        let mut buffer = TuiBuffer::empty(area);
        let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
        let mut surface = TuiPaintSurface::new(&mut buffer);
        element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
        buffer
            .to_lines()
            .into_iter()
            .map(|line| line.trim_end().to_string())
            .collect()
    })
}

#[test]
fn focus_hooks_update_editor_focus_without_changing_text() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (window_id, editor) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                TuiEditorView::single_line,
            )
        });
        editor.update(&mut app, |editor, ctx| {
            editor.set_text("gen", ctx);
            ctx.focus_self();
        });
        assert!(editor.read(&app, |editor, _| editor.is_focused()));
        assert_eq!(render_lines(&app, &editor)[0], "gen");

        let other = app.update(|ctx| ctx.add_tui_view(window_id, |_| TestHostView));
        other.update(&mut app, |_, ctx| ctx.focus_self());
        assert!(!editor.read(&app, |editor, _| editor.is_focused()));
        assert_eq!(render_lines(&app, &editor)[0], "gen");
    });
}

#[test]
fn shared_initializer_registers_line_start_for_input_and_editor() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            crate::input::init(ctx);
            super::init(ctx);
        });

        let triggers_for = |name: &str| {
            app.read(|ctx| {
                ctx.get_key_bindings()
                    .filter(|binding| binding.name == name)
                    .filter_map(|binding| match binding.trigger {
                        Trigger::Keystrokes(keys) => keys.first().map(|key| key.normalized()),
                        Trigger::Empty | Trigger::Standard(_) | Trigger::Custom(_) => None,
                    })
                    .collect::<HashSet<_>>()
            })
        };
        let expected = HashSet::from(["home".to_string(), "ctrl-a".to_string()]);
        assert_eq!(triggers_for("tui:input:move_to_line_start"), expected);
        assert_eq!(triggers_for("tui:editor:move_to_line_start"), expected);
        assert!(app.read(|ctx| ctx.get_binding_by_name("tui:editor:move_up").is_none()));
    });
}

#[test]
fn mouse_selection_action_focuses_the_editor() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (window_id, editor) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                TuiEditorView::single_line,
            )
        });
        let other = app.update(|ctx| ctx.add_tui_view(window_id, |_| TestHostView));
        other.update(&mut app, |_, ctx| ctx.focus_self());
        assert!(!editor.read(&app, |editor, _| editor.is_focused()));

        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &TuiEditorViewAction::Editor(TuiEditorAction::SelectionStartAt {
                    offset: CharOffset::from(1),
                }),
                ctx,
            );
        });
        assert!(editor.read(&app, |editor, _| editor.is_focused()));
    });
}

#[test]
fn actions_edit_the_single_line_buffer() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (_, editor) = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                TuiEditorView::single_line,
            )
        });
        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &TuiEditorViewAction::Editor(TuiEditorAction::InsertText("gen".to_string())),
                ctx,
            );
            editor.handle_action(
                &TuiEditorViewAction::Command(TuiEditorCommand::Backspace),
                ctx,
            );
        });
        // Line navigation is visual-row-aware; layout establishes the real
        // terminal width before the command runs.
        render_lines(&app, &editor);
        editor.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &TuiEditorViewAction::Command(TuiEditorCommand::MoveToLineStart),
                ctx,
            );
            editor.handle_action(
                &TuiEditorViewAction::Editor(TuiEditorAction::InsertChar('X')),
                ctx,
            );
        });
        assert_eq!(editor.read(&app, |editor, ctx| editor.text(ctx)), "Xge");
    });
}

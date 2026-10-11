#![allow(clippy::unwrap_used)]
use crate::{
    HerdrWindow,
    keymap::{Binding, Keymap},
};
use gpui::{Entity, VisualTestContext};

fn search(view: &Entity<HerdrWindow>, query: &str, cx: &mut VisualTestContext) {
    view.update(cx, |view, cx| {
        view.menu
            .keybinds_search
            .as_ref()
            .unwrap()
            .update(cx, |input, cx| input.set_text_selected(query, cx));
        view.menu.keybinds_scroll.set_offset(Default::default());
        cx.notify();
    });
    cx.update(|window, cx| crate::sidebar::layout_tests::full_draw(window, cx).clear(cx));
}

#[gpui::test]
fn editor_launch_rows_follow_resolved_bindings_in_the_shortcuts_pane(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = cx.add_window_view(crate::sidebar::layout_tests::fixture_window);
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_keybinds(window, cx)));
    search(&view, "Open VS Code", cx);
    assert!(cx.debug_bounds("shortcut-Open VS Code").is_some());
    assert!(cx.debug_bounds("shortcut-Open VS Code On").is_some());
    for (key, row) in [
        ("cmd-alt-e", "shortcut-Open VS Code"),
        ("cmd-alt-shift-e", "shortcut-Open VS Code On"),
    ] {
        search(&view, key, cx);
        assert!(cx.debug_bounds(row).is_some(), "missing default {key}");
    }
    view.update(cx, |view, _| {
        let overrides = [
            ("open_code".into(), Binding::One("cmd-alt-r".into())),
            (
                "open_code_on".into(),
                Binding::One("cmd-alt-shift-r".into()),
            ),
        ]
        .into_iter()
        .collect();
        view.config.keybindings =
            Keymap::with_overrides(&overrides, &Default::default(), &Default::default()).unwrap();
    });
    for (key, row) in [
        ("cmd-alt-r", "shortcut-Open VS Code"),
        ("cmd-alt-shift-r", "shortcut-Open VS Code On"),
    ] {
        search(&view, key, cx);
        assert!(cx.debug_bounds(row).is_some(), "missing resolved {key}");
    }
    search(&view, "cmd-alt-e", cx);
    assert!(cx.debug_bounds("shortcut-Open VS Code").is_none());
    assert!(cx.debug_bounds("shortcut-Open VS Code On").is_none());
    view.update(cx, |view, _| {
        let overrides = [
            ("open_code".into(), Binding::Many(Vec::new())),
            ("open_code_on".into(), Binding::One(String::new())),
        ]
        .into_iter()
        .collect();
        view.config.keybindings =
            Keymap::with_overrides(&overrides, &Default::default(), &Default::default()).unwrap();
    });
    search(&view, "Open VS Code", cx);
    assert!(cx.debug_bounds("shortcut-Open VS Code").is_none());
    assert!(cx.debug_bounds("shortcut-Open VS Code On").is_none());
    view.update(cx, |view, _| view.config.keybindings = Keymap::default());
    search(&view, "Open VS Code", cx);
    assert!(cx.debug_bounds("shortcut-Open VS Code").is_some());
    assert!(cx.debug_bounds("shortcut-Open VS Code On").is_some());
}

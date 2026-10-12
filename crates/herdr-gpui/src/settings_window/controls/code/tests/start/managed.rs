use super::*;

#[gpui::test]
fn automatic_mode_can_be_selected_without_a_discovered_cli(cx: &mut TestAppContext) {
    for cli in [Cli::Missing, Cli::Finding] {
        let (view, cx, edits) = start_page(cx, cli);
        view.read_with(cx, |view, cx| {
            assert_eq!(view.code_mode(cx), Some(CodeMode::Start));
        });
        view.update(cx, |view, cx| {
            view.config.code.mode = Some(CodeMode::Address);
            cx.notify();
        });
        draw(cx);
        assert!(cx.debug_bounds("settings-code-url").is_some());
        click(cx, "settings-code-start");
        assert_eq!(
            edits.lock().unwrap().as_slice(),
            [CodeEdit::Mode(CodeMode::Start)]
        );
    }
}

use super::*;

#[test]
fn editor_launch_bindings_rebind_unbind_and_reload() {
    let defaults = with(&[]).unwrap();
    assert!(defaults.triggers(Command::OpenCode, &keystroke("alt-cmd-e"), false));
    assert!(defaults.triggers(Command::OpenCodeOn, &keystroke("shift-alt-cmd-e"), false));
    let edited = with(&[("open_code", one("cmd-alt-r")), ("open_code_on", one(""))]).unwrap();
    assert!(edited.triggers(Command::OpenCode, &keystroke("alt-cmd-r"), false));
    assert!(!edited.triggers(Command::OpenCode, &keystroke("cmd-alt-e"), false));
    assert!(!edited.triggers(Command::OpenCodeOn, &keystroke("cmd-alt-shift-e"), false));
    assert_eq!(with(&[]).unwrap(), defaults);
    let mut seen = HashSet::new();
    for (_, shortcut) in defaults.bindings() {
        let parsed = keystroke(shortcut);
        assert!(seen.insert((parsed.modifiers, parsed.key)));
    }
}

#[test]
fn focused_editor_only_dispatches_resolved_launch_commands_and_preserves_clipboard() {
    let defaults = with(&[]).unwrap();
    assert_eq!(
        defaults.editor_launch(&keystroke("cmd-alt-e")),
        Some(Command::OpenCode)
    );
    assert_eq!(
        defaults.editor_launch(&keystroke("cmd-alt-shift-e")),
        Some(Command::OpenCodeOn)
    );
    for key in [
        "cmd-v",
        "cmd-alt-shift-v",
        "cmd-c",
        "cmd-x",
        "cmd-o",
        "cmd-shift-p",
        "a",
        "left",
    ] {
        assert_eq!(defaults.editor_launch(&keystroke(key)), None, "{key}");
    }
    let changed = with(&[("open_code", one("cmd-alt-r")), ("open_code_on", one(""))]).unwrap();
    assert_eq!(changed.editor_launch(&keystroke("cmd-alt-e")), None);
    assert_eq!(
        changed.editor_launch(&keystroke("cmd-alt-r")),
        Some(Command::OpenCode)
    );
    let clipboard = with(&[("open_code", one("cmd-v"))]).unwrap();
    assert_eq!(clipboard.editor_launch(&keystroke("cmd-v")), None);
}

//! Match AppKit's native characters to GPUI's resolved direct key bindings.
use gpui::{Keystroke, Modifiers};

pub(super) fn from_native(character: &str, mut modifiers: Modifiers) -> Keystroke {
    let first = character.chars().next();
    // AppKit sets Function for navigation and function keys even without Fn.
    if matches!(first, Some('\u{f700}'..='\u{f747}')) {
        modifiers.function = false;
    }
    let key = match character {
        " " => "space".into(),
        "\t" | "\u{19}" => "tab".into(),
        "\r" | "\u{3}" => "enter".into(),
        "\u{7f}" => "backspace".into(),
        "\u{1b}" => "escape".into(),
        "\u{f700}" => "up".into(),
        "\u{f701}" => "down".into(),
        "\u{f702}" => "left".into(),
        "\u{f703}" => "right".into(),
        "\u{f728}" => "delete".into(),
        "\u{f729}" => "home".into(),
        "\u{f72b}" => "end".into(),
        "\u{f72c}" => "pageup".into(),
        "\u{f72d}" => "pagedown".into(),
        "\u{f746}" => "insert".into(),
        _ if matches!(first, Some('\u{f704}'..='\u{f726}')) => {
            format!("f{}", first.map_or(0, |key| key as u32 - 0xf704 + 1))
        }
        _ if character.len() == 1 && first.is_some_and(|key| key.is_ascii_alphabetic()) => {
            character.to_ascii_lowercase()
        }
        _ => {
            // GPUI represents shifted punctuation by its resulting character.
            modifiers.shift = false;
            character.to_owned()
        }
    };
    Keystroke {
        modifiers,
        key,
        key_char: None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::{
        controls::Command,
        keymap::{Binding, Keymap},
    };

    #[test]
    fn native_editor_defaults_and_nonprinting_rebinds_use_resolved_keys() {
        let modifiers = Modifiers {
            platform: true,
            alt: true,
            ..Default::default()
        };
        let defaults = Keymap::default();
        assert_eq!(
            defaults.editor_launch(&from_native("e", modifiers)),
            Some(Command::OpenCode)
        );
        assert_eq!(
            defaults.editor_launch(&from_native(
                "E",
                Modifiers {
                    shift: true,
                    ..modifiers
                }
            )),
            Some(Command::OpenCodeOn)
        );
        for (native, binding) in [
            ("\u{f702}", "cmd-alt-left"),
            ("\u{f709}", "cmd-alt-f6"),
            ("!", "cmd-alt-!"),
        ] {
            let keymap = Keymap::with_overrides(
                &[("open_code".into(), Binding::One(binding.into()))].into(),
                &Default::default(),
                &Default::default(),
            )
            .unwrap();
            let typed = from_native(
                native,
                Modifiers {
                    shift: native == "!",
                    function: native != "!",
                    ..modifiers
                },
            );
            assert_eq!(
                keymap.editor_launch(&typed),
                Some(Command::OpenCode),
                "{binding}"
            );
        }
        assert!(
            from_native(
                "r",
                Modifiers {
                    function: true,
                    ..modifiers
                }
            )
            .modifiers
            .function
        );
    }
}

use super::*;
use crate::code_server::Target;

#[gpui::test]
fn configured_target_preserves_legacy_platform_and_fixture_behavior(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        Launcher::fixture(cx, found());
        let code = accepted();
        assert_eq!(
            format!(
                "{:?}",
                Launcher::startup_target(cx, &code, &Target::Configured)
            ),
            format!("{:?}", Launcher::startup(cx, &code))
        );
        Launcher::want_target(cx, &code, &Target::Configured, &[]).unwrap();
        assert_eq!(
            Launcher::alive_target(cx, &Target::Configured),
            Launcher::alive(cx)
        );
        Launcher::retry_target(cx, &Target::Configured);
        assert!(!Launcher::running(cx));
    });
}

#[cfg(not(target_os = "macos"))]
#[gpui::test]
fn unsupported_clients_return_a_typed_error_for_new_machine_demand(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        Launcher::fixture(cx, found());
        assert!(matches!(
            Launcher::want_target(cx, &accepted(), &Target::Local, &[]),
            Err(crate::Error::Code(
                crate::code_server::Error::UnsupportedTarget
            ))
        ));
    });
}

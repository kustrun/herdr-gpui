#![allow(clippy::unwrap_used)]
use super::*;
use core::prelude::v1::test;

#[gpui::test]
fn picker_owner_survives_focus_changes_but_not_group_or_workspace_closure(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(crate::sidebar::layout_tests::fixture_window);
    cx.update(|_, cx| {
        view.update(cx, |view, _| {
            let mut snapshot = crate::sidebar::layout_tests::snapshot(40);
            snapshot.focused_workspace_id = Some("w0".into());
            snapshot.focused_tab_id = Some("t0".into());
            view.live.snapshot = Some(std::sync::Arc::new(snapshot));
            let owner = Owner::capture(view, None).unwrap();
            assert!(owner.current(view));
            let mut changed = (**view.live.snapshot.as_ref().unwrap()).clone();
            changed.focused_workspace_id = Some("w1".into());
            view.live.snapshot = Some(std::sync::Arc::new(changed.clone()));
            assert!(owner.current(view), "focus must not retarget the operation");
            let mut closed = changed.clone();
            closed
                .workspaces
                .retain(|workspace| workspace.workspace_id != owner.workspace);
            view.live.snapshot = Some(std::sync::Arc::new(closed));
            assert!(
                !owner.current(view),
                "closed workspace cancels the operation"
            );
            changed.boot_id = "replacement".into();
            view.live.snapshot = Some(std::sync::Arc::new(changed.clone()));
            assert!(
                !owner.current(view),
                "daemon replacement cancels the operation"
            );
            changed.boot_id = owner.boot.clone();
            view.live.snapshot = Some(std::sync::Arc::new(changed));
            view.browser
                .layouts
                .remove(&(owner.scope.clone(), owner.workspace.clone()));
            assert!(!owner.current(view), "closed group cancels the operation");
        })
    });
}

#[gpui::test]
fn saved_offline_machines_are_choices_and_labels_ignore_page_titles(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(crate::sidebar::layout_tests::fixture_window);
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            let host = SavedHost {
                id: "a".repeat(32),
                label: "Server A".into(),
                target: "server-a.invalid".into(),
                session: "default".into(),
                enabled: true,
            };
            let target = Target::saved(&host).unwrap();
            view.reconcile_catalog(vec![host], Vec::new(), cx);
            let choices = view.code_choices(cx);
            assert!(
                choices
                    .iter()
                    .any(|choice| choice.target.as_ref() == Some(&target)
                        && choice.detail.contains("offline"))
            );
            assert_eq!(view.code_label(&target), "VS Code · Server A");
            view.reconcile_catalog(Vec::new(), Vec::new(), cx);
            assert_eq!(
                view.code_label(&target),
                "VS Code · Unavailable SSH machine"
            );
            assert!(target.resolve(&view.code_hosts()).is_err());
        })
    });
}

#[gpui::test]
fn same_machine_folder_is_encoded_and_cross_machine_is_blank(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(crate::sidebar::layout_tests::fixture_window);
    cx.update(|_, cx| {
        view.update(cx, |view, _| {
            let mut snapshot = crate::sidebar::layout_tests::snapshot(40);
            snapshot.focused_workspace_id = Some("w0".into());
            snapshot.focused_tab_id = Some("t0".into());
            snapshot.panes = serde_json::from_value(serde_json::json!([{
                "pane_id": "p0", "workspace_id": "w0", "tab_id": "t0",
                "label": "shell", "cwd": "/projects/example", "foreground_cwd": null,
                "focused": true, "right_click_passthrough": false
            }]))
            .unwrap();
            view.live.snapshot = Some(std::sync::Arc::new(snapshot));
            let owner = Owner::capture(view, None).unwrap();
            let server =
                super::super::WebUrl::try_from("http://127.0.0.1:8000/?tkn=fictional-token")
                    .unwrap();
            assert_eq!(
                view.code_start_for(&owner, &Target::Local, &server)
                    .as_str(),
                "http://127.0.0.1:8000/?folder=%2Fprojects%2Fexample"
            );
            let other = Target::Ssh {
                id: "a".repeat(32),
                fingerprint: "b".repeat(64),
            };
            assert_eq!(
                view.code_start_for(&owner, &other, &server).as_str(),
                "http://127.0.0.1:8000/"
            );
            view.endpoints[0].generation += 1;
            assert!(
                !owner.current(view),
                "connection replacement fences the owner"
            );
        })
    });
}

#[gpui::test]
fn disabled_edited_and_unsupported_choices_never_select_local(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(crate::sidebar::layout_tests::fixture_window);
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            let mut host = SavedHost {
                id: "a".repeat(32),
                label: "Server A".into(),
                target: "server-a.invalid".into(),
                session: "default".into(),
                enabled: true,
            };
            let old = Target::saved(&host).unwrap();
            host.target = "server-b.invalid".into();
            view.reconcile_catalog(
                vec![host.clone()],
                vec![herdr_client::WslHost {
                    distro: "Example".into(),
                    session: "default".into(),
                }],
                cx,
            );
            assert!(old.resolve(&view.code_hosts()).is_err());
            let choices = view.code_choices(cx);
            assert!(choices.iter().any(|choice| choice.label == "Example"
                && choice.target.is_none()
                && choice.detail.contains("Unsupported")));
            let current = Target::saved(&host).unwrap();
            assert!(
                choices
                    .iter()
                    .any(|choice| choice.target.as_ref() == Some(&current))
            );
            host.enabled = false;
            view.reconcile_catalog(vec![host], Vec::new(), cx);
            assert!(
                view.code_choices(cx)
                    .iter()
                    .any(|choice| choice.label == "Server A"
                        && choice.target.is_none()
                        && choice.detail == "Machine disabled")
            );
            assert!(current.resolve(&view.code_hosts()).is_err());
        })
    });
}

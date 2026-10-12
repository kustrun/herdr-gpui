use super::*;
use crate::browser::{code_launch::Owner, groups::Pick};
use gpui::{Entity, TestAppContext, VisualTestContext};

fn fixture(cx: &mut TestAppContext) -> (Entity<HerdrWindow>, &mut VisualTestContext) {
    cx.update(|cx| Store::default().install(cx));
    cx.add_window_view(|window, cx| {
        let mut view = crate::sidebar::layout_tests::fixture_window(window, cx);
        let mut snapshot = crate::sidebar::layout_tests::snapshot(40);
        snapshot.focused_workspace_id = Some("w0".into());
        snapshot.focused_tab_id = Some("t0".into());
        view.live.snapshot = Some(std::sync::Arc::new(snapshot));
        view
    })
}

fn ready(_: &WebUrl) -> crate::Result<Server> {
    Ok(Server::from_version("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap())
}

fn refused(_: &WebUrl) -> crate::Result<Server> {
    Err(crate::code_server::Error::TokenRefused.into())
}

#[gpui::test]
fn stale_probe_cannot_remove_replacement_readiness_after_owner_group_changes(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let id = cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            let old = Owner::capture(view, None).unwrap();
            let id = Store::update(cx, |store| {
                store.open_code_tab_for(old.scope.clone(), &old.workspace, Target::Configured, None)
            })
            .unwrap();
            let tab = store(cx).unwrap().get(id).unwrap().clone();
            view.browser.code_probe = refused;
            assert!(!view.code_server_ready(
                &tab,
                old.clone(),
                &url("http://127.0.0.1:8000/"),
                false,
                cx
            ));
            view.browser
                .layouts
                .remove(&(old.scope.clone(), old.workspace.clone()));
            let replacement = Owner::capture(view, None).unwrap();
            assert_ne!(old.group, replacement.group);
            view.browser.code_probe = ready;
            assert!(!view.code_server_ready(
                &tab,
                replacement,
                &url("http://127.0.0.1:8000/"),
                false,
                cx
            ));
            id
        })
    });
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert!(view.code_page_current(id, cx));
        assert!(matches!(
            view.browser
                .code_servers
                .get(&id)
                .map(|server| &server.state),
            Some(Reach::Ready { .. })
        ))
    });
    view.update(cx, |view, cx| {
        view.browser.layouts.clear();
        assert!(
            !view.code_page_current(id, cx),
            "late native navigation is fenced too"
        );
    });
}

#[gpui::test]
fn displayed_groups_have_independent_target_probes_and_retry(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let ids = cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            let host = herdr_client::SavedHost {
                id: "a".repeat(32),
                label: "Server A".into(),
                target: "server-a.invalid".into(),
                session: "default".into(),
                enabled: true,
            };
            let target = Target::saved(&host).unwrap();
            view.reconcile_catalog(vec![host], Vec::new(), cx);
            let owner = Owner::capture(view, None).unwrap();
            let second = view.browser.group_ids.next();
            view.browser
                .layouts
                .get_mut(&(owner.scope.clone(), owner.workspace.clone()))
                .unwrap()
                .split(owner.group, second, None);
            let ids: Vec<_> = [Target::Local, target]
                .into_iter()
                .map(|target| {
                    Store::update(cx, |store| {
                        store.open_code_tab_for(owner.scope.clone(), &owner.workspace, target, None)
                    })
                    .unwrap()
                })
                .collect();
            for (group, id, probe) in [
                (
                    owner.group,
                    ids[0],
                    ready as fn(&WebUrl) -> crate::Result<Server>,
                ),
                (
                    second,
                    ids[1],
                    refused as fn(&WebUrl) -> crate::Result<Server>,
                ),
            ] {
                view.browser
                    .layouts
                    .get_mut(&(owner.scope.clone(), owner.workspace.clone()))
                    .unwrap()
                    .choose(group, Pick::Page(id));
                let captured = Owner::capture(view, Some(group)).unwrap();
                let tab = store(cx).unwrap().get(id).unwrap().clone();
                view.browser.code_probe = probe;
                assert!(!view.code_server_ready(
                    &tab,
                    captured,
                    &url("http://127.0.0.1:8000/"),
                    false,
                    cx
                ));
            }
            assert_eq!(
                view.displayed_code_tabs(cx)
                    .into_iter()
                    .map(|(_, tab)| tab.id)
                    .collect::<Vec<_>>(),
                ids,
                "every displayed group contributes demand"
            );
            ids
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            assert!(matches!(
                view.browser.code_servers[&ids[0]].state,
                Reach::Ready { .. }
            ));
            assert!(matches!(
                view.browser.code_servers[&ids[1]].state,
                Reach::Failed { .. }
            ));
            view.browser.failed.insert(ids[0], "Local failure".into());
            view.browser
                .failed
                .insert(ids[1], "Server A failure".into());
            let target = store(cx)
                .unwrap()
                .get(ids[1])
                .unwrap()
                .code_target()
                .unwrap()
                .clone();
            view.retry_code_target(&target, cx);
            assert!(!view.browser.code_servers.contains_key(&ids[1]));
            assert!(!view.browser.failed.contains_key(&ids[1]));
            assert!(view.browser.code_servers.contains_key(&ids[0]));
            assert!(view.browser.failed.contains_key(&ids[0]));
        })
    });
}

#[gpui::test]
fn closed_tabs_cannot_receive_late_probe_results(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            let owner = Owner::capture(view, None).unwrap();
            let id = Store::update(cx, |store| {
                store.open_code_tab_for(
                    owner.scope.clone(),
                    &owner.workspace,
                    Target::Configured,
                    None,
                )
            })
            .unwrap();
            let tab = store(cx).unwrap().get(id).unwrap().clone();
            view.browser.code_probe = ready;
            assert!(!view.code_server_ready(
                &tab,
                owner,
                &url("http://127.0.0.1:8000/"),
                false,
                cx
            ));
            Store::update(cx, |store| store.close(id));
            view.forget_browser_tabs(|closed| closed == id);
            id
        })
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(!view.browser.code_servers.contains_key(&id))
    });
}

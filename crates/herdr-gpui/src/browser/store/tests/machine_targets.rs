use super::*;
use crate::browser::{
    groups::{GroupIds, Layout, SavedLayout},
    native_profile::identifier,
};
use herdr_client::SavedHost;
use serde_json::json;

fn host(id: &str, label: &str) -> SavedHost {
    SavedHost {
        id: id.into(),
        label: label.into(),
        target: id.into(),
        session: "default".into(),
        enabled: true,
    }
}

fn machines() -> [Target; 3] {
    [
        Target::Local,
        Target::saved(&host("server-a", "Server A")).unwrap(),
        Target::saved(&host("server-b", "Server B")).unwrap(),
    ]
}

fn saved(store: &Store) -> Vec<u8> {
    serde_json::to_vec(&Saved {
        tabs: store.tabs.clone(),
    })
    .unwrap()
}

#[test]
fn local_and_two_servers_coexist_and_reopen_in_their_own_workspace() {
    let mut store = Store::default();
    let owner = Scope::endpoint("server-a");
    let targets = machines();
    let ids: Vec<_> = targets
        .iter()
        .map(|target| {
            store
                .open_code_tab_for(owner.clone(), "w_1", target.clone(), None)
                .unwrap()
        })
        .collect();
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        3
    );
    assert!(store.code_tab(&owner, "w_1").is_none());
    for (target, id) in targets.iter().zip(&ids) {
        assert!(store.visited_code(
            *id,
            target,
            url("http://127.0.0.1:8000/?folder=/projects/example"),
            Some("Server B")
        ));
        let before = store.get(*id).unwrap().clone();
        assert_eq!(
            store.open_code_tab_for(
                owner.clone(),
                "w_1",
                target.clone(),
                url("http://127.0.0.1:9000/")
            ),
            Some(*id)
        );
        assert_eq!(store.get(*id), Some(&before));
        assert_eq!(store.code_tab_for(&owner, "w_1", target).unwrap().id, *id);
    }
    let other_workspace = store
        .open_code_tab_for(owner.clone(), "w_2", targets[1].clone(), None)
        .unwrap();
    let other_owner = store
        .open_code_tab_for(Scope::endpoint("server-b"), "w_1", targets[1].clone(), None)
        .unwrap();
    assert!(!ids.contains(&other_workspace) && !ids.contains(&other_owner));
    assert_eq!(
        store
            .code_tabs(&targets[1])
            .map(|tab| tab.id)
            .collect::<Vec<_>>(),
        [ids[1], other_workspace, other_owner]
    );
    let restored = Store::with_tabs(parse(&saved(&store)).unwrap(), None);
    assert_eq!(restored.tabs, store.tabs);
    assert!(store.forget_workspaces(&owner, &["w_1".into()]));
    assert!(ids.iter().all(|id| store.get(*id).is_none()));
    assert!(store.get(other_workspace).is_some() && store.get(other_owner).is_some());
}

#[test]
fn legacy_aliases_keep_ids_folders_duplicate_bindings_and_layouts() {
    let input = json!({"tabs": [
        {"id": 7, "scope": "server-a", "workspace_id": "w_1", "location": {"kind": "web", "url": "http://127.0.0.1:8000/?folder=/projects/example"}, "title": "Server A", "place": "code"},
        {"id": 9, "scope": "server-a", "workspace_id": "w_1", "location": null, "title": "Server B", "place": "code_group"},
        {"id": 11, "scope": "server-a", "workspace_id": "w_1", "location": null, "title": ""}
    ]});
    let mut store = Store::with_tabs(parse(&serde_json::to_vec(&input).unwrap()).unwrap(), None);
    assert_eq!(
        store.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>(),
        [TabId::test(7), TabId::test(9), TabId::test(11)]
    );
    for tab in &store.tabs[..2] {
        assert_eq!(tab.code_target(), Some(&Target::Configured));
        assert_eq!(identifier(tab), None);
    }
    assert_eq!(
        store.tabs[0].location,
        url("http://127.0.0.1:8000/?folder=/projects/example")
    );
    assert_eq!(store.tabs[1].location, None);
    assert_eq!(store.tabs[2].code_target(), None);
    assert_eq!(
        store.open_code_tab(Scope::endpoint("server-a"), "w_1", None),
        Some(TabId::test(7))
    );
    assert_eq!(store.tabs.len(), 3);
    assert_eq!(parse(&saved(&store)).unwrap(), store.tabs);
    let layout: SavedLayout = serde_json::from_value(json!({
        "groups": [
            {"pick": {"kind": "page", "id": 7}, "share": 0.5},
            {"pick": {"kind": "page", "id": 9}, "share": 0.5}
        ], "active": 1
    }))
    .unwrap();
    assert!(layout.valid());
    assert_eq!(
        Layout::restore(&layout, &mut GroupIds::default()).saved(),
        Some(layout)
    );
}

#[test]
fn target_navigation_follow_and_reconnect_selection_leave_others_alone() {
    let mut store = Store::default();
    let owner = Scope::endpoint("server-a");
    let [local, a, b] = machines();
    let page = url("http://127.0.0.1:8000/editor?folder=/projects/example&view=one");
    let ids: Vec<_> = [Target::Configured, local, a.clone(), b.clone()]
        .into_iter()
        .map(|target| {
            store
                .open_code_tab_for(owner.clone(), "w_1", target, page.clone())
                .unwrap()
        })
        .collect();
    let ordinary = store.open(owner.clone(), "w_1", page, None).unwrap();
    let another_a = store
        .open_code_tab_for(
            owner.clone(),
            "w_2",
            a.clone(),
            url("http://127.0.0.1:8000/other?folder=/projects/example&view=two"),
        )
        .unwrap();
    let waiting_a = store
        .open_code_tab_for(owner, "w_3", a.clone(), None)
        .unwrap();
    let before = store.tabs.clone();
    assert!(!store.visited_code(ids[2], &b, url("http://127.0.0.1:9999/"), None));
    assert!(!store.visited_code(ordinary, &a, None, Some("Server A")));
    assert_eq!(store.tabs, before);
    assert!(store.visited_code(
        ids[2],
        &a,
        url("http://127.0.0.1:8000/editor?folder=/projects/example&view=three"),
        Some("Server B")
    ));
    let server = WebUrl::try_from("http://127.0.0.1:9000/").unwrap();
    let retarget = |page: &WebUrl| {
        let mut changed = page.0.clone();
        changed.set_port(Some(9000)).unwrap();
        WebUrl::try_from(changed.as_str()).unwrap()
    };
    assert!(store.follow_code_target(&a, &server, retarget));
    assert!(!store.follow_code_target(&a, &server, retarget));
    assert_eq!(
        store.get(ids[2]).unwrap().location,
        url("http://127.0.0.1:9000/editor?folder=/projects/example&view=three")
    );
    assert_eq!(
        store.get(another_a).unwrap().location,
        url("http://127.0.0.1:9000/other?folder=/projects/example&view=two")
    );
    assert_eq!(store.get(waiting_a).unwrap().location, None);
    for id in [ids[0], ids[1], ids[3], ordinary] {
        assert_eq!(store.get(id), before.iter().find(|tab| tab.id == id));
    }
    let managed: Vec<_> = store
        .tabs
        .iter()
        .filter(|tab| tab.code_target() != Some(&Target::Configured))
        .cloned()
        .collect();
    assert!(store.follow_code_server(&server, retarget));
    for tab in managed {
        assert_eq!(store.get(tab.id), Some(&tab));
    }
}

#[test]
fn missing_or_edited_saved_targets_keep_their_persisted_binding() {
    let mut store = Store::default();
    let mut host = host("server-a", "Server A");
    let old = Target::saved(&host).unwrap();
    let owner = Scope::endpoint("server-a");
    let id = store
        .open_code_tab_for(owner.clone(), "w_1", old.clone(), None)
        .unwrap();
    let profile = identifier(store.get(id).unwrap());
    assert!(old.resolve(&[]).is_err());
    host.target = "server-b".into();
    assert!(old.resolve(std::slice::from_ref(&host)).is_err());
    let new = Target::saved(&host).unwrap();
    let new_id = store.open_code_tab_for(owner, "w_1", new, None).unwrap();
    assert_ne!(new_id, id);
    assert_ne!(identifier(store.get(new_id).unwrap()), profile);
    let restored = Store::with_tabs(parse(&saved(&store)).unwrap(), None);
    assert_eq!(restored.get(id).unwrap().code_target(), Some(&old));
    assert_eq!(identifier(restored.get(id).unwrap()), profile);
}

#[test]
fn malformed_bindings_and_persistence_bounds_are_rejected() {
    let base = json!({"id": 0, "scope": "server-a", "workspace_id": "w_1", "location": null, "title": "", "place": "code"});
    for target in [
        json!({"kind": "ssh", "id": "", "fingerprint": "a".repeat(64)}),
        json!({"kind": "ssh", "id": "a".repeat(257), "fingerprint": "a".repeat(64)}),
        json!({"kind": "ssh", "id": "server-a\n", "fingerprint": "a".repeat(64)}),
        json!({"kind": "ssh", "id": "server-a", "fingerprint": "A".repeat(64)}),
        json!({"kind": "ssh", "id": "server-a", "fingerprint": "a".repeat(63)}),
        json!({"kind": "ssh", "id": "server-a"}),
        json!({"kind": "local", "unexpected": true}),
        json!({"kind": "configured", "unexpected": true}),
        json!({"kind": "unknown"}),
    ] {
        let mut tab = base.clone();
        tab["target"] = target;
        assert!(
            parse(&serde_json::to_vec(&json!({"tabs": [tab.clone()]})).unwrap()).is_err(),
            "{tab}"
        );
    }
    for target in [
        r#"{"kind":"local","kind":"configured"}"#.to_owned(),
        format!(
            r#"{{"kind":"ssh","id":"server-a","id":"server-b","fingerprint":"{}"}}"#,
            "a".repeat(64)
        ),
        format!(
            r#"{{"kind":"ssh","id":"server-a","fingerprint":"{}","fingerprint":"{}"}}"#,
            "a".repeat(64),
            "b".repeat(64)
        ),
    ] {
        let input = format!(
            r#"{{"tabs":[{{"id":0,"scope":"server-a","workspace_id":"w_1","location":null,"title":"","place":"code","target":{target}}}]}}"#
        );
        assert!(parse(input.as_bytes()).is_err());
    }
    for target in [json!({"kind": "local"}), json!({"kind": "configured"})] {
        let mut tab = base.clone();
        tab["place"] = json!("group");
        tab["target"] = target;
        assert!(parse(&serde_json::to_vec(&json!({"tabs": [tab]})).unwrap()).is_err());
    }
    let oversized = json!({"tabs": vec![base; MAX_TABS + 1]});
    assert!(parse(&serde_json::to_vec(&oversized).unwrap()).is_err());
    let mut oversized_bytes = br#"{"tabs":[]}"#.to_vec();
    oversized_bytes.resize(MAX_FILE_BYTES as usize + 1, b' ');
    assert!(parse(&oversized_bytes).is_err());
    let mut store = Store::default();
    assert!(
        store
            .open_code_tab_for(
                Scope::endpoint("server-a"),
                "w_1",
                Target::Ssh {
                    id: "server-a".into(),
                    fingerprint: "invalid".into()
                },
                None
            )
            .is_none()
    );
    for index in 0..MAX_TABS {
        assert!(
            store
                .open_code_tab_for(
                    Scope::endpoint("server-a"),
                    &format!("w_{index}"),
                    Target::Local,
                    None
                )
                .is_some()
        );
    }
    assert_eq!(
        store.open_code_tab_for(Scope::endpoint("server-a"), "w_0", Target::Local, None),
        Some(TabId::test(0))
    );
    assert!(
        store
            .open_code_tab_for(Scope::endpoint("server-a"), "w_next", Target::Local, None)
            .is_none()
    );
}

#[test]
fn native_profiles_are_stable_per_target_across_windows_workspaces_and_urls() {
    let mut store = Store::default();
    let profiles: Vec<_> = machines()
        .into_iter()
        .map(|target| {
            let first = store
                .open_code_tab_for(
                    Scope::endpoint("server-a"),
                    "w_1",
                    target.clone(),
                    url("http://127.0.0.1:8000/?folder=/projects/example"),
                )
                .unwrap();
            let second = store
                .open_code_tab_for(
                    Scope::endpoint("server-b"),
                    "w_2",
                    target,
                    url("http://127.0.0.1:9000/"),
                )
                .unwrap();
            let profile = identifier(store.get(first).unwrap()).unwrap();
            assert_eq!(identifier(store.get(second).unwrap()), Some(profile));
            assert_eq!(
                identifier(&store.get(first).unwrap().clone()),
                Some(profile)
            );
            assert_eq!(uuid::Uuid::from_bytes(profile).get_version_num(), 5);
            profile
        })
        .collect();
    assert_eq!(
        profiles
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        3
    );
    let configured = store
        .open_code_tab(Scope::endpoint("server-a"), "w_1", None)
        .unwrap();
    let ordinary = store
        .open(Scope::endpoint("server-a"), "w_1", None, None)
        .unwrap();
    assert_eq!(identifier(store.get(configured).unwrap()), None);
    assert_eq!(identifier(store.get(ordinary).unwrap()), None);
    let restored = Store::with_tabs(parse(&saved(&store)).unwrap(), None);
    assert_eq!(
        restored.tabs.iter().map(identifier).collect::<Vec<_>>(),
        store.tabs.iter().map(identifier).collect::<Vec<_>>()
    );
}

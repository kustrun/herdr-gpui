use super::*;

#[test]
fn code_tokens_and_page_titles_never_enter_saved_navigation() {
    let mut store = Store::default();
    let target = Target::Local;
    let owner = Scope::endpoint("server-a");
    let dirty = "http://127.0.0.1:8000/workbench?%74kn=fictional-secret&folder=%2Fprojects%2Fexample&t%6bn=fictional-secret#file";
    let clean = url("http://127.0.0.1:8000/workbench?folder=%2Fprojects%2Fexample#file");
    let id = store
        .open_code_tab_for(owner.clone(), "w_1", target.clone(), url(dirty))
        .unwrap();
    assert_eq!(store.get(id).unwrap().location, clean);
    assert!(!store.visited_code(id, &target, url(dirty), Some("fictional-secret")));
    // Generic Loaded/address navigation callers share the same privacy boundary.
    assert!(!store.visited(id, url(dirty), Some("fictional-secret")));
    let saved = serde_json::to_vec(&Saved {
        tabs: store.tabs.clone(),
    })
    .unwrap();
    assert!(!String::from_utf8_lossy(&saved).contains("fictional-secret"));
    let restored = Store::with_tabs(parse(&saved).unwrap(), None);
    assert_eq!(restored.get(id), store.get(id));
    let mut legacy: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    legacy["tabs"][0]["location"]["url"] = dirty.into();
    legacy["tabs"][0]["title"] = "fictional-secret".into();
    let restored = parse(&serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert_eq!(restored[0].location, clean);
    assert_eq!(restored[0].title, "VS Code");
    assert!(store.follow_code_target(&target, &WebUrl::try_from("http://127.0.0.1:9000/?tkn=fictional-new").unwrap(), |_| WebUrl::try_from("http://127.0.0.1:9000/workbench?folder=%2Fprojects%2Fexample&tkn=fictional-secret#file").unwrap()));
    assert!(
        !serde_json::to_string(&Saved {
            tabs: store.tabs.clone()
        })
        .unwrap()
        .contains("fictional-secret")
    );
    assert_eq!(
        store.open_code_tab_for(owner, "w_1", target, None),
        Some(id)
    );
    assert_eq!(
        store.get(id).unwrap().location,
        url("http://127.0.0.1:9000/workbench?folder=%2Fprojects%2Fexample#file"),
        "reopening preserves the existing navigation"
    );
}

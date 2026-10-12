use super::*;
use std::{os::unix::fs::PermissionsExt, path::Path, process::Command};

fn repository(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    for args in [
        vec!["init", "-q"],
        vec![
            "remote",
            "add",
            "origin",
            "https://github.com/example/project.git",
        ],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
}

#[test]
fn discovers_beads_in_the_parent_workspace_without_github_access() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    let checkout = root.join("project");
    std::fs::create_dir_all(root.join(".beads")).unwrap();
    repository(&checkout);
    let host = Host::new(&ConnectTarget::Local).unwrap();
    let info = repo::detect(
        &ConnectTarget::Local,
        &host,
        checkout.to_str().unwrap(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(info.main_root, checkout.to_str().unwrap());
    assert_eq!(
        info.beads_source().unwrap().repository,
        root.to_str().unwrap()
    );
    assert_eq!(sources(&info)[0].0.provider, Provider::Beads);
}

#[test]
fn local_beads_sync_and_survive_github_credential_failure() {
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("project");
    repository(&checkout);
    std::fs::create_dir_all(checkout.join(".beads")).unwrap();
    let home = dir.path().join("home");
    let bin = home.join(".local/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let bd = bin.join("bd");
    std::fs::write(&bd, r#"#!/bin/sh
set -eu
[ "$BEADS_DIR" = "$EXPECTED_BEADS_DIR" ]
case "$*" in
  --help) printf '%s\n' '--readonly --sandbox' ;;
  '--readonly --sandbox list --json --all --limit 0') printf '%s\n' '[{"id":"example-1","title":"Local task","status":"open"}]' ;;
  '--readonly --sandbox blocked --json') printf '%s\n' '[]' ;;
  *) exit 1 ;;
esac
"#).unwrap();
    std::fs::set_permissions(&bd, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut host = Host::new(&ConnectTarget::Local).unwrap();
    host.env = vec![
        ("HOME".into(), home.to_str().unwrap().into()),
        (
            "EXPECTED_BEADS_DIR".into(),
            checkout.join(".beads").to_str().unwrap().into(),
        ),
    ];
    let cancelled = Arc::new(AtomicBool::new(false));
    let info = repo::detect(
        &ConnectTarget::Local,
        &host,
        checkout.to_str().unwrap(),
        &cancelled,
    )
    .unwrap();
    let keys = sources(&info);
    assert_eq!(
        keys[0].0.provider,
        Provider::Beads,
        "local data must precede network sync"
    );
    let (commands, _receiver) = mpsc::sync_channel(COMMANDS);
    let mut worker = Worker {
        request: Request {
            target: ConnectTarget::Local,
            checkout: checkout.to_str().unwrap().into(),
            workspace_id: "workspace-1".into(),
            token: None,
            data_root: Some(dir.path().join("data")),
            timing: Timing::default(),
        },
        mailbox: Arc::default(),
        notices: Arc::default(),
        commands,
        actions: Arc::new(AtomicUsize::new(0)),
        site: None,
        cancelled,
        snapshot: Snapshot {
            repo: Some(info),
            sources: keys
                .into_iter()
                .map(|(key, _)| SourceStatus {
                    key,
                    state: SyncState::Syncing,
                })
                .collect(),
            ..Snapshot::default()
        },
        store: Some(Store::open(&dir.path().join("store.sqlite3")).unwrap()),
        host: Some(host),
        data_version: None,
        retry: Vec::new(),
        uncached: HashMap::new(),
        cooldown: None,
    };
    worker.sync(None);
    assert_eq!(worker.snapshot.items.len(), 1);
    assert_eq!(worker.snapshot.items[0].identifier, "example-1");
    assert!(matches!(
        worker.snapshot.sources[1].state,
        SyncState::SignIn
    ));
    // A malformed synthetic credential is rejected before any network request.
    worker.sync(Some(&SecretString::from("invalid\nfixture")));
    assert!(matches!(&worker.snapshot.sources[1].state,
        SyncState::Failed(error) if matches!(error.as_ref(), Error::Github(source) if matches!(source.as_ref(), crate::Error::GitHubHeader(_)))));
    assert!(matches!(
        worker.snapshot.sources[0].state,
        SyncState::Synced { count: 1, .. }
    ));
    assert_eq!(worker.snapshot.items[0].identifier, "example-1");
    assert!(worker.snapshot.error.is_none());
    let updated = std::fs::read_to_string(&bd)
        .unwrap()
        .replace("Local task", "Updated local task");
    std::fs::write(&bd, updated).unwrap();
    worker.sync(None);
    assert!(
        matches!(worker.snapshot.sources[1].state, SyncState::Failed(_)),
        "GitHub stays in backoff"
    );
    assert_eq!(worker.snapshot.items.len(), 1);
    assert_eq!(worker.snapshot.items[0].title, "Updated local task");
}

#[test]
fn worktrees_fall_back_to_main_beads_but_prefer_their_own() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main");
    repository(&main);
    std::fs::create_dir_all(main.join(".beads")).unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&main)
            .args([
                "-c",
                "user.name=Example",
                "-c",
                "user.email=example@example.invalid",
                "-c",
                "commit.gpgSign=false",
                "commit",
                "--allow-empty",
                "-qm",
                "fixture"
            ])
            .status()
            .unwrap()
            .success()
    );
    let checkout = dir.path().join("worktree");
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&main)
            .args(["worktree", "add", "--detach", "-q"])
            .arg(&checkout)
            .status()
            .unwrap()
            .success()
    );
    let host = Host::new(&ConnectTarget::Local).unwrap();
    let detect = |path: &Path| {
        repo::detect(
            &ConnectTarget::Local,
            &host,
            path.to_str().unwrap(),
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let info = detect(&checkout);
    assert_eq!(info.main_root, main.to_str().unwrap());
    assert_eq!(
        info.beads_source().unwrap().repository,
        main.to_str().unwrap()
    );
    let nested = checkout.join("nested");
    std::fs::create_dir_all(nested.join(".beads")).unwrap();
    let info = detect(&nested);
    assert_eq!(info.main_root, main.to_str().unwrap());
    assert_eq!(
        info.beads_source().unwrap().repository,
        nested.to_str().unwrap()
    );
}

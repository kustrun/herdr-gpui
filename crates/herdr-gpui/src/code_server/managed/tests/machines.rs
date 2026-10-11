use super::*;
use crate::browser::WebUrl;
use crate::code_server::supervisor::Address;

fn entry(generation: u64) -> Entry {
    Entry {
        shared: Arc::default(),
        generation,
        configured: false,
        independent: true,
        requested_port: None,
    }
}
fn target(id: &str) -> Target {
    Target::Ssh {
        id: id.into(),
        fingerprint: "a".repeat(64),
    }
}
fn ready(port: u16) -> Address {
    Address {
        port: NonZeroU16::new(port).unwrap(),
        url: WebUrl::try_from(format!("http://127.0.0.1:{port}/?tkn=secret")).unwrap(),
    }
}

#[test]
fn targets_keep_independent_reports_generations_and_retry() {
    let mut manager = Manager::default();
    let a = target("server-a");
    let b = target("server-b");
    manager.entries.insert(a.clone(), entry(1));
    manager.entries.insert(b.clone(), entry(2));
    manager.entries[&a]
        .shared
        .publish(Status::Ready, Some(ready(51001)), false);
    manager.entries[&b]
        .shared
        .publish(Status::Ready, Some(ready(51002)), false);
    let code = CodeConfig {
        license_accepted: true,
        ..Default::default()
    };
    assert!(
        matches!(manager.startup(&a, &code), Startup::Ready { url } if url.address() == "127.0.0.1:51001")
    );
    manager.retry(&a);
    assert_eq!(manager.generation(&a), None);
    assert_eq!(manager.generation(&b), Some(2));
    assert!(matches!(manager.startup(&b, &code), Startup::Ready { .. }));
    assert!(matches!(
        manager.want(&a, &code, &[]),
        Err(Error::TargetUnavailable)
    ));
    assert!(manager.entries.contains_key(&b));
}

#[test]
fn configuration_does_not_stop_independent_local_or_remote_demand() {
    let mut manager = Manager::default();
    manager.entries.insert(Target::Local, entry(1));
    manager.entries.insert(target("server-a"), entry(2));
    manager.sync(&CodeConfig {
        mode: Some(crate::config::CodeMode::Address),
        license_accepted: true,
        ..Default::default()
    });
    assert_eq!(manager.entries.len(), 2);
    manager.entries.get_mut(&Target::Local).unwrap().independent = false;
    manager.stop_configured();
    assert_eq!(manager.entries.len(), 1);
    manager.sync(&CodeConfig::default());
    assert!(manager.entries.is_empty());
}

#[test]
fn consent_gates_all_filesystem_and_process_work() {
    let mut manager = Manager::default();
    manager
        .want(&Target::Local, &CodeConfig::default(), &[])
        .unwrap();
    assert!(manager.entries.is_empty());
    assert!(matches!(
        manager.startup(&Target::Local, &CodeConfig::default()),
        Startup::Consent
    ));
}

#[test]
fn persisted_origin_is_atomic_bounded_and_has_no_collision_fallback() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("origin");
    assert_eq!(origin::load(&path).unwrap(), None);
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = NonZeroU16::new(listener.local_addr().unwrap().port()).unwrap();
    origin::save(&path, port).unwrap();
    assert_eq!(origin::load(&path).unwrap(), Some(port));
    assert!(
        std::net::TcpListener::bind((
            std::net::Ipv4Addr::LOCALHOST,
            origin::load(&path).unwrap().unwrap().get()
        ))
        .is_err()
    );
    assert_eq!(origin::load(&path).unwrap(), Some(port));
    std::fs::write(&path, "999999999999").unwrap();
    assert!(origin::load(&path).is_err());
}

#[test]
fn quit_signals_all_entries_before_any_wait_and_shares_one_budget() {
    let mut manager = Manager::default();
    let mut signals = Vec::new();
    for n in 0..256 {
        let entry = entry(n);
        signals.push(entry.shared.clone());
        manager.entries.insert(target(&n.to_string()), entry);
    }
    let started = Instant::now();
    manager.quit(Duration::from_millis(150));
    assert!(started.elapsed() < Duration::from_millis(250));
    assert!(
        signals
            .iter()
            .all(|shared| shared.stopped.load(Ordering::Acquire))
    );
}

#[test]
fn failed_target_map_and_ui_liveness_are_bounded() {
    let mut manager = Manager::default();
    for n in 0..300 {
        manager.fail(&target(&n.to_string()), Error::TargetUnavailable.into());
    }
    assert_eq!(manager.entries.len(), MAX_ENTRIES);
    let locked = manager.entries[&target("0")].shared.running.lock().unwrap();
    let start = Instant::now();
    assert!(!manager.alive(&target("0")));
    assert!(start.elapsed() < Duration::from_millis(50));
    drop(locked);
    let code = CodeConfig {
        license_accepted: true,
        ..Default::default()
    };
    assert!(matches!(
        manager.want(&Target::Local, &code, &[]),
        Err(Error::TargetLimit)
    ));
    manager.sync_hosts(&[]);
    assert!(manager.entries.is_empty());
}

#[test]
fn permanent_failures_require_retry_but_transport_loss_does_not() {
    for error in [
        Error::Digest,
        Error::Archive,
        Error::UnsafeCache,
        Error::Capability,
        Error::UnsupportedTarget,
        Error::RemoteTools,
        Error::Ssh(herdr_client::SshFailure::Auth),
        Error::Ssh(herdr_client::SshFailure::HostKey),
    ] {
        assert!(error.permanent());
    }
    for error in [
        Error::RemoteExited,
        Error::Ssh(herdr_client::SshFailure::Unreachable),
        Error::RemoteIo(std::io::ErrorKind::BrokenPipe.into()),
    ] {
        assert!(!error.permanent());
    }
    let shared = Shared::default();
    shared.stopped.store(true, Ordering::Release);
    shared.publish(Status::Ready, Some(ready(51001)), false);
    assert!(
        shared.state.lock().unwrap().report.is_none(),
        "retired generation cannot publish"
    );
}

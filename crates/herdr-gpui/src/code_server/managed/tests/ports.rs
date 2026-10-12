use super::*;

fn local(port: Option<NonZeroU16>) -> Entry {
    Entry {
        shared: Arc::default(),
        generation: 1,
        configured: true,
        independent: true,
        requested_port: port,
    }
}

#[test]
fn explicit_port_changes_retire_only_local_and_preserve_both_owners() {
    let mut manager = Manager::default();
    let port = NonZeroU16::new(51001);
    let moved = NonZeroU16::new(51002);
    let entry = local(port);
    let retired = entry.shared.clone();
    manager.entries.insert(Target::Local, entry);
    let ssh = Target::Ssh {
        id: "server-a".into(),
        fingerprint: "a".repeat(64),
    };
    manager.entries.insert(ssh.clone(), local(None));
    assert_eq!(manager.retire_moved_local(None), None);
    assert_eq!(manager.retire_moved_local(port), None);
    assert_eq!(manager.generation(&Target::Configured), Some(1));
    assert_eq!(manager.retire_moved_local(moved), Some((true, true)));
    assert!(retired.stopped.load(Ordering::Acquire));
    assert_eq!(manager.generation(&Target::Local), None);
    assert_eq!(manager.generation(&ssh), Some(1));
}

#[test]
fn acknowledged_port_is_visible_before_config_self_save_and_does_not_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("origin");
    let entry = local(None);
    let port = NonZeroU16::new(51001).unwrap();
    entry
        .shared
        .acknowledge(&path, port, |saved| {
            assert_eq!(saved, port);
            assert_eq!(origin::load(&path).unwrap(), Some(port));
            assert!(entry.keeps_port(Some(port)));
            Ok(())
        })
        .unwrap();
    let mut manager = Manager::default();
    manager.entries.insert(Target::Local, entry);
    assert_eq!(manager.retire_moved_local(Some(port)), None);
    assert_eq!(manager.generation(&Target::Configured), Some(1));
}

#[test]
fn cancelled_acknowledgement_cannot_publish_origin_or_run_config_save() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("origin");
    let shared = Shared::default();
    shared.stopped.store(true, Ordering::Release);
    assert!(
        shared
            .acknowledge(&path, NonZeroU16::new(51001).unwrap(), |_| {
                panic!("cancelled generation reached config save")
            })
            .is_err()
    );
    assert_eq!(origin::load(&path).unwrap(), None);
}

#[test]
fn a_cancelled_old_origin_writer_cannot_overwrite_the_new_generation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("origin");
    let old = NonZeroU16::new(51001).unwrap();
    let new = NonZeroU16::new(51002).unwrap();
    origin::save(&path, old).unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))
        .unwrap();
    lock.lock().unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = cancelled.clone();
    let old_path = path.clone();
    let started = Arc::new(std::sync::Barrier::new(2));
    let running = started.clone();
    let writer = thread::spawn(move || {
        running.wait();
        origin::save_active(&old_path, old, &signal)
    });
    started.wait();
    cancelled.store(true, Ordering::Release);
    drop(lock);
    origin::save(&path, new).unwrap();
    assert!(matches!(writer.join().unwrap(), Err(Error::Cancelled)));
    assert_eq!(origin::load(&path).unwrap(), Some(new));
}

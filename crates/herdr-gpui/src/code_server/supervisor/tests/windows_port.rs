use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn legacy_preparation_keeps_its_port_when_saving_fails() {
    let root = tempfile::tempdir().unwrap();
    let saves = AtomicUsize::new(0);
    let mut worker = Worker {
        shared: Arc::default(),
        plan: Plan {
            program: PathBuf::from("code"),
            port: None,
            token_file: root.path().join("token"),
        },
        port: None,
        command: serve_web,
        probe: |_: &WebUrl| -> crate::Result<Server> { panic!("preparation never probes") },
        save_port: |_: NonZeroU16| {
            saves.fetch_add(1, Ordering::Relaxed);
            Err(std::io::Error::from(ErrorKind::PermissionDenied).into())
        },
        timing: Timing::default(),
    };
    let first = worker.prepare().unwrap();
    let again = worker.prepare().unwrap();
    assert_eq!(first.port, again.port);
    assert_eq!(saves.load(Ordering::Relaxed), 1);
    assert!(matches!(
        worker.shared.lock().status,
        Some(Status::Starting)
    ));
}

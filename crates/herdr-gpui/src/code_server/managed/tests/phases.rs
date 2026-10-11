use super::*;

#[test]
fn target_phases_are_independent_observable_and_clear_on_ready() {
    let mut manager = Manager::default();
    for target in [
        Target::Local,
        Target::Ssh {
            id: "server-a".into(),
            fingerprint: "a".repeat(64),
        },
    ] {
        let shared = Arc::new(Shared::default());
        shared.publish(Status::Starting, None, false);
        manager.entries.insert(
            target,
            Entry {
                shared,
                generation: 1,
                configured: false,
                independent: true,
                requested_port: None,
            },
        );
    }
    let local = &manager.entries[&Target::Local].shared;
    let revision = manager.revision();
    local.phase(Phase::Preparing);
    assert!(manager.revision() > revision);
    assert_eq!(manager.phase(&Target::Configured), Some(Phase::Preparing));
    local.phase(Phase::Installing);
    assert_eq!(manager.phase(&Target::Local), Some(Phase::Installing));
    assert!(
        manager
            .entries
            .iter()
            .filter(|(target, _)| **target != Target::Local)
            .all(|(target, _)| manager.phase(target).is_none())
    );
    local.phase(Phase::Starting);
    assert_eq!(manager.phase(&Target::Local), Some(Phase::Starting));
    local.publish(Status::Ready, None, false);
    assert_eq!(manager.phase(&Target::Local), None);
}

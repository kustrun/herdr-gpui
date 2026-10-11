#![allow(clippy::unwrap_used)]
use super::*;

fn host() -> SavedHost {
    SavedHost {
        id: "server-a".into(),
        label: "Server A".into(),
        target: "server-a".into(),
        session: "default".into(),
        enabled: true,
    }
}

#[test]
fn identity_ignores_label_session_and_rejects_destination_edits() {
    let mut host = host();
    let target = Target::saved(&host).unwrap();
    host.label = "Server B".into();
    host.session = "another".into();
    assert_eq!(target, Target::saved(&host).unwrap());
    assert_eq!(
        target.resolve(std::slice::from_ref(&host)).unwrap(),
        Some("server-a".into())
    );
    host.target = "server-b".into();
    assert_ne!(target, Target::saved(&host).unwrap());
    assert!(matches!(
        target.resolve(&[host]),
        Err(Error::TargetUnavailable)
    ));
    assert!(matches!(target.resolve(&[]), Err(Error::TargetUnavailable)));
    assert_eq!(
        target,
        serde_json::from_str(&serde_json::to_string(&target).unwrap()).unwrap()
    );
}

#[test]
fn automatic_default_comes_from_config_not_cli_discovery() {
    let host = host();
    let owner = ConnectTarget::Ssh {
        target: host.target.clone(),
        session: "another".into(),
    };
    assert_eq!(
        Target::default_for(
            &CodeConfig::default(),
            &owner,
            Some("server-a"),
            std::slice::from_ref(&host)
        )
        .unwrap(),
        Target::saved(&host).unwrap()
    );
    let explicit = CodeConfig {
        mode: Some(CodeMode::Address),
        ..Default::default()
    };
    assert_eq!(
        Target::default_for(&explicit, &owner, None, &[]).unwrap(),
        Target::Configured
    );
    for owner in [
        ConnectTarget::Local,
        ConnectTarget::Socket("/projects/example/socket".into()),
        ConnectTarget::Session {
            name: "default".into(),
            development: false,
        },
    ] {
        assert_eq!(Target::owner(&owner, None, &[]).unwrap(), Target::Local);
    }
    assert!(matches!(
        Target::owner(
            &ConnectTarget::Wsl {
                distro: "example".into(),
                session: "default".into()
            },
            None,
            &[]
        ),
        Err(Error::UnsupportedTarget)
    ));
}

#[test]
fn duplicate_destinations_retain_selected_saved_identity_and_invalid_payloads_fail() {
    let a = host();
    let mut b = a.clone();
    b.id = "server-b".into();
    b.label = "Server B".into();
    let owner = ConnectTarget::Ssh {
        target: "server-a".into(),
        session: "default".into(),
    };
    assert_eq!(
        Target::owner(&owner, Some("server-b"), &[a.clone(), b.clone()]).unwrap(),
        Target::saved(&b).unwrap()
    );
    assert_ne!(
        Target::saved(&a).unwrap().key(),
        Target::saved(&b).unwrap().key()
    );
    assert!(Target::owner(&owner, None, &[a, b]).is_err());
    for target in [
        Target::Ssh {
            id: "server-a".into(),
            fingerprint: "not-a-fingerprint".into(),
        },
        Target::Ssh {
            id: "a".repeat(257),
            fingerprint: "a".repeat(64),
        },
        Target::Ssh {
            id: "server-a\n".into(),
            fingerprint: "a".repeat(64),
        },
    ] {
        assert!(!target.valid());
    }
}

use super::*;

#[test]
fn stdin_owned_forward_reuses_the_exact_validated_trust_and_listener_policy() {
    let legacy = forward_command("server-a", 51001, 51002).unwrap();
    let controlled = controlled_forward_command("server-a", 51001, 51002).unwrap();
    let legacy: Vec<_> = legacy.get_args().filter(|arg| *arg != "-N").collect();
    let args: Vec<_> = controlled.get_args().collect();
    assert_eq!(
        args.last().unwrap(),
        &"exec /bin/sh -c 'exec cat >/dev/null'"
    );
    assert!(args.contains(&std::ffi::OsStr::new("-T")));
    assert!(args.contains(&std::ffi::OsStr::new("-C")));
    assert!(!args.contains(&std::ffi::OsStr::new("-N")));
    assert_eq!(args[..args.len() - 1], legacy);
    assert!(controlled_forward_command("-oProxyCommand=bad", 1, 2).is_err());
}

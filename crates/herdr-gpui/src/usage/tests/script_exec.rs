#[test]
fn usage_shell_keeps_one_owned_remote_shell_and_its_stdin_protocol() {
    let command = herdr_client::script_command("server-a", "exec /bin/sh -s").unwrap();
    let args: Vec<_> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        args.last().map(String::as_str),
        Some("exec /bin/sh -c 'exec /bin/sh -s'")
    );
    assert!(args.iter().any(|arg| arg == "StrictHostKeyChecking=yes"));
    assert!(args.iter().any(|arg| arg == "ControlMaster=no"));
}

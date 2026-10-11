use super::*;

fn server() -> Remote {
    let mut wrapper = Owned(
        Command::new("sleep")
            .arg("60")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let control = wrapper.0.stdin.take().unwrap();
    let mut tunnel = Owned(
        Command::new("sleep")
            .arg("60")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let tunnel_control = tunnel.0.stdin.take().unwrap();
    Remote {
        wrapper,
        tunnel,
        _tunnel_control: tunnel_control,
        control,
        output: Arc::default(),
        generation: 7,
        ping: Instant::now(),
        address: Address {
            port: NonZeroU16::new(51001).unwrap(),
            url: WebUrl::try_from("http://127.0.0.1:51001/?tkn=fixture-token").unwrap(),
        },
    }
}

#[test]
fn a_successful_probe_requires_both_owned_children_to_outlive_its_answer() {
    for wrapper in [true, false] {
        let mut server = server();
        let child = if wrapper {
            &server.wrapper.0
        } else {
            &server.tunnel.0
        };
        let pid = rustix::process::Pid::from_child(child);
        let result = server.readiness(|_| {
            rustix::process::kill_process(pid, rustix::process::Signal::KILL).unwrap();
            // Acknowledge exit without reaping: the real owner retains its child.
            ownership::until("owned child exited during probe", || {
                rustix::process::waitid(
                    rustix::process::WaitId::Pid(pid),
                    rustix::process::WaitIdOptions::EXITED
                        | rustix::process::WaitIdOptions::NOHANG
                        | rustix::process::WaitIdOptions::NOWAIT,
                )
                .unwrap()
                .is_some()
            });
            Ok(super::super::super::Server::from_version(&"a".repeat(40)).unwrap())
        });
        assert!(
            result.is_err(),
            "a successful answer cannot restore lost ownership"
        );
    }
    let mut healthy = server();
    assert!(
        healthy
            .readiness(|_| Ok(super::super::super::Server::from_version(&"a".repeat(40)).unwrap()))
            .unwrap()
    );
}

#[test]
fn quick_stop_and_drop_reap_only_the_owned_transport_children() {
    let mut sentinel = Owned(Command::new("sleep").arg("60").spawn().unwrap());
    let mut server = server();
    let owned = [server.wrapper.0.id() as i32, server.tunnel.0.id() as i32];
    let started = Instant::now();
    server.stop();
    assert!(started.elapsed() < Duration::from_millis(50));
    drop(server);
    assert!(owned.into_iter().all(ownership::dead));
    assert!(sentinel.0.try_wait().unwrap().is_none());
}

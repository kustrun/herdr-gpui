//! Real transport fixture: only generated keys, private config and loopback.
use super::ownership::{DESCENDANT, Fixture, dead, no_runtime, pids, until};
use super::*;
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

/// SIGQUIT asks PRoot to kill and reap its registered tracees. SIGKILL would
/// detach them, so the ordinary exact-child guard is unsuitable for the tracer.
struct Tracer(Child);
impl Drop for Tracer {
    fn drop(&mut self) {
        if !crate::code_server::supervisor::process::exited(&mut self.0).unwrap_or(false) {
            let pid = rustix::process::Pid::from_child(&self.0);
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::QUIT);
        }
        let _ = self.0.wait();
    }
}

fn isolated_daemon(root: &Path, config: &Path) -> Tracer {
    let Some(proot) = std::env::var_os("HERDR_TEST_PROOT") else {
        panic!("set HERDR_TEST_PROOT to an explicit PRoot executable");
    };
    let Some(sshd) = std::env::var_os("HERDR_TEST_SSHD") else {
        panic!("set HERDR_TEST_SSHD to an explicit sshd executable");
    };
    assert!(Path::new(&proot).is_absolute() && Path::new(&sshd).is_absolute());
    let private = root.join("account");
    fs::create_dir(&private).unwrap();
    fs::create_dir(private.join("ssh")).unwrap();
    fs::create_dir(private.join("nscd")).unwrap();
    let uid = rustix::process::getuid().as_raw();
    let gid = rustix::process::getgid().as_raw();
    fs::write(
        private.join("passwd"),
        format!(
            "server-a::{uid}:{gid}:Server A:{}:/bin/sh\nsshd:*:65534:65534:Server B:{}:/bin/false\n",
            root.display(), root.display()
        ),
    )
    .unwrap();
    fs::write(private.join("group"), format!("server-a:x:{gid}:\n")).unwrap();
    for file in ["shadow", "gshadow"] {
        fs::write(private.join(file), "").unwrap();
    }
    fs::write(
        private.join("nsswitch.conf"),
        "passwd: files\ngroup: files\nshadow: files\nhosts: files\n",
    )
    .unwrap();
    fs::write(private.join("hosts"), "127.0.0.1 localhost\n").unwrap();
    let mut command = Command::new(proot);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", root)
        .env("ENV", "/dev/null")
        .env("BASH_ENV", "/dev/null")
        .env("PROOT_NO_SECCOMP", "1")
        .arg("-w")
        .arg(root);
    if let Some(libraries) = std::env::var_os("LD_LIBRARY_PATH") {
        command.env("LD_LIBRARY_PATH", libraries);
    }
    // All account and automatic SSH startup lookups resolve inside the fixture.
    // NSS cannot fall back to an existing account cache or a real home/shell.
    for (source, destination) in [
        ("ssh", "/etc/ssh"),
        ("passwd", "/etc/passwd"),
        ("group", "/etc/group"),
        ("shadow", "/etc/shadow"),
        ("gshadow", "/etc/gshadow"),
        ("nsswitch.conf", "/etc/nsswitch.conf"),
        ("hosts", "/etc/hosts"),
        ("nscd", "/var/run/nscd"),
    ] {
        command
            .arg("-b")
            .arg(format!("{}:{destination}", private.join(source).display()));
    }
    let start = format!(
        "printf '%s\\n' $$ > {}\nexec {} -D -e -f {}",
        herdr_client::shell_quote(&root.join("daemon-pid").to_string_lossy()),
        herdr_client::shell_quote(&sshd.to_string_lossy()),
        herdr_client::shell_quote(&config.to_string_lossy())
    );
    command
        .args(["/bin/sh", "-c", &start])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let Ok(child) = command.spawn() else {
        panic!("could not start isolated PRoot sshd");
    };
    Tracer(child)
}

fn client(root: &Path, port: NonZeroU16) -> Command {
    let mut command = Command::new("ssh");
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", root)
        .env("SHELL", "/bin/sh")
        .env("ENV", "/dev/null")
        .env("BASH_ENV", "/dev/null")
        .args([
            "-F",
            "/dev/null",
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "ControlMaster=no",
            "-o",
            "ControlPath=none",
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "IdentityAgent=none",
            "-o",
            "GlobalKnownHostsFile=/dev/null",
            "-o",
            "PasswordAuthentication=no",
            "-o",
            "ConnectTimeout=3",
            "-o",
        ])
        .arg(format!(
            "UserKnownHostsFile={}",
            root.join("known_hosts").display()
        ))
        .arg("-i")
        .arg(root.join("client-key"))
        .arg("-p")
        .arg(port.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

fn tunnels_follow_owned_stdin(root: &Path, port: NonZeroU16) {
    for mode in ["startup", "ready", "drop"] {
        let local = free_port().unwrap();
        let policy =
            herdr_client::controlled_forward_command("server-a@127.0.0.1", local.get(), 51001)
                .unwrap();
        let mut command = client(root, port);
        command.args(policy.get_args()).stderr(Stdio::piped());
        let before = if root.join("cat-pids").exists() {
            pids(&root.join("cat-pids")).len()
        } else {
            0
        };
        let mut tunnel = Owned(command.spawn().unwrap());
        let control = tunnel.0.stdin.take().unwrap();
        let stdout = tunnel.0.stdout.take().unwrap();
        if mode != "startup" {
            let mut line = String::new();
            BufReader::new(stdout)
                .take(256)
                .read_line(&mut line)
                .unwrap();
            if line.trim() != herdr_client::FORWARD_READY {
                let mut diagnostic = Vec::new();
                tunnel
                    .0
                    .stderr
                    .take()
                    .unwrap()
                    .take(4096)
                    .read_to_end(&mut diagnostic)
                    .unwrap();
                let status = tunnel.0.wait().unwrap();
                panic!(
                    "forward readiness failed: mode={mode}, status={status}, kind={:?}",
                    herdr_client::SshFailure::classify(status.code(), &diagnostic),
                );
            }
            until("foreground tunnel command", || {
                root.join("cat-pids").exists() && pids(&root.join("cat-pids")).len() > before
            });
        }
        if mode == "drop" {
            drop(tunnel);
            drop(control);
        } else {
            drop(control);
            until("tunnel EOF exit", || tunnel.0.try_wait().unwrap().is_some());
            drop(tunnel);
        }
        if root.join("cat-pids").exists() {
            let owned = pids(&root.join("cat-pids"));
            until("foreground tunnel descendants", || {
                owned.iter().all(|pid| {
                    dead(*pid)
                        // Container PID1 can leave an exited peer-loss orphan unreaped.
                        || (mode == "drop"
                            && fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
                                stat.rsplit_once(')').is_some_and(|(_, fields)| {
                                    fields.split_whitespace().next() == Some("Z")
                                })
                            }))
                })
            });
        }
        assert!(std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, local.get())).is_ok());
    }
}

#[test]
#[ignore = "requires Linux, ssh, ssh-keygen, HERDR_TEST_PROOT and HERDR_TEST_SSHD; isolated loopback only"]
fn connection_drop_and_bootstrap_cancel() {
    let fixture = Fixture::new(DESCENDANT);
    let mut sentinel = Owned(Command::new("sleep").arg("60").spawn().unwrap());
    let root = fixture.root.path();
    fs::create_dir(root.join("bin")).unwrap();
    crate::test_executable::write(
        root.join("bin/cat"),
        "#!/bin/sh\nprintf '%s\\n' $$ >> \"$HOME/cat-pids\"\nexec /bin/cat \"$@\"\n",
        0o700,
    )
    .unwrap();
    let key = root.join("client-key");
    let host = root.join("host-key");
    for path in [&key, &host] {
        assert!(
            Command::new("ssh-keygen")
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", root)
                .args(["-q", "-t", "ed25519", "-N", "", "-C", "Server A", "-f"])
                .arg(path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
    }
    fs::copy(key.with_extension("pub"), root.join("authorized_keys")).unwrap();
    let port = free_port().unwrap();
    let username = "server-a";
    let config = root.join("sshd_config");
    // PRoot's traced sshd cannot establish compressed sessions in this fixture.
    // Keep the client's exact forwarding policy; this test proves ownership.
    fs::write(&config, format!("ListenAddress 127.0.0.1\nPort {port}\nHostKey {}\nPidFile {}\nAuthorizedKeysFile {}\nStrictModes no\nCompression no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nUseDNS no\nPermitUserRC no\nPermitUserEnvironment no\nSetEnv HOME={} BASH_ENV=/dev/null ENV=/dev/null PATH={}/bin:/usr/bin:/bin\nAllowUsers {username}\nLogLevel QUIET\n", host.display(), root.join("sshd.pid").display(), root.join("authorized_keys").display(), root.display(), root.display())).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let mut server = isolated_daemon(root, &config);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port.get())).is_ok() {
            break;
        }
        assert!(
            !crate::code_server::supervisor::process::exited(&mut server.0).unwrap(),
            "isolated sshd could not start"
        );
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(20));
    }
    let daemon = pids(&root.join("daemon-pid"));
    let known = root.join("known_hosts");
    let public = fs::read_to_string(host.with_extension("pub")).unwrap();
    fs::write(&known, format!("[127.0.0.1]:{port} {public}")).unwrap();
    tunnels_follow_owned_stdin(root, port);
    assert!(sentinel.0.try_wait().unwrap().is_none());
    for generation in [11, 12, 13] {
        let bootstrap_pid = root.join("bootstrap-pid");
        let bootstrap = if generation == 13 {
            format!(
                "bootstrap() {{ sleep 60 & cli=$!; printf '%s\n' \"$cli\" > {}; wait \"$cli\"; }}",
                herdr_client::shell_quote(&bootstrap_pid.to_string_lossy())
            )
        } else {
            fixture.bootstrap()
        };
        let remote_script = format!(
            "[ \"$HOME\" = {} ] && [ \"$ENV\" = /dev/null ] && [ \"$BASH_ENV\" = /dev/null ] || exit 1\nXDG_STATE_HOME=\"$HOME/.local/state\"\nXDG_CACHE_HOME=\"$HOME/.cache\"\nXDG_CONFIG_HOME=\"$HOME/.config\"\nexport XDG_STATE_HOME XDG_CACHE_HOME XDG_CONFIG_HOME\n{}",
            herdr_client::shell_quote(&root.to_string_lossy()),
            fixture.with_bootstrap(generation, 1, &bootstrap)
        );
        let mut command = client(root, port);
        command
            .arg("--")
            .arg(format!("{username}@127.0.0.1"))
            .arg(format!(
                "exec /bin/sh -c {}",
                herdr_client::shell_quote(&remote_script)
            ));
        let token = Token::load_or_create(&root.join("token")).unwrap();
        if generation == 13 {
            let mut wrapper = Owned(command.spawn().unwrap());
            let mut control = wrapper.0.stdin.take().unwrap();
            writeln!(control, "token {generation} {}", token.as_str()).unwrap();
            until("real SSH bootstrap child", || bootstrap_pid.exists());
            let owned = pids(&bootstrap_pid);
            wrapper.0.kill().unwrap();
            drop(control);
            drop(wrapper);
            until("real SSH bootstrap cleanup", || {
                owned.iter().all(|pid| dead(*pid)) && no_runtime(root)
            });
        } else {
            let cancel = AtomicBool::new(false);
            let (mut wrapper, control, _, _) =
                start_wrapper(command, generation, &token, &cancel, || Ok(())).unwrap();
            let owned = pids(&fixture.program.with_extension("pids"));
            if generation == 11 {
                wrapper.0.kill().unwrap();
            }
            drop(control);
            drop(wrapper);
            until("real SSH runtime cleanup", || {
                owned.iter().all(|pid| dead(*pid)) && no_runtime(root)
            });
        }
        assert!(sentinel.0.try_wait().unwrap().is_none());
    }
    drop(server);
    until("isolated daemon cleanup", || {
        daemon.iter().all(|pid| dead(*pid))
    });
    assert!(sentinel.0.try_wait().unwrap().is_none());
}

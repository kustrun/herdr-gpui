use super::*;
use std::{fs, os::unix::process::CommandExt, path::Path};

pub(super) struct Fixture {
    pub(super) root: tempfile::TempDir,
    pub(super) program: std::path::PathBuf,
}
impl Fixture {
    pub(super) fn new(body: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join("code");
        let body = format!(
            "#!/bin/sh\nif [ \"$2\" = --help ]; then exit 0; fi\nprintf '%s\\n' $$ > '{}'\n{body}\n",
            root.path().join("pid").display()
        );
        crate::test_executable::write(&program, body, 0o700).unwrap();
        Self { root, program }
    }
    pub(super) fn bootstrap(&self) -> String {
        format!(
            "bootstrap() {{ program={}; }}",
            herdr_client::shell_quote(&self.program.to_string_lossy())
        )
    }
    pub(super) fn with_bootstrap(&self, generation: u64, lease: u64, bootstrap: &str) -> String {
        let release = Release {
            commit: "a".repeat(40),
            platform: "cli-linux-x64",
            url: "https://vscode.download.prss.microsoft.com/fixture".into(),
            sha256: "a".repeat(64),
        };
        script(&release, generation, lease, 1, bootstrap, "server-a")
    }
    fn start(
        &self,
        generation: u64,
        lease: u64,
        bootstrap: &str,
        close_output: bool,
    ) -> (Owned, ChildStdin, Option<Arc<Mutex<Output>>>) {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", &self.with_bootstrap(generation, lease, bootstrap)])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.root.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0);
        let mut child = Owned(command.spawn().unwrap());
        let mut stdin = child.0.stdin.take().unwrap();
        writeln!(stdin, "token {generation} fixture-token").unwrap();
        let stdout = child.0.stdout.take().unwrap();
        let output = if close_output {
            // Keep stdout open through START so this exercises loss after launch.
            until("CLI before output closes", || {
                self.program.with_extension("pids").exists()
            });
            drop(stdout);
            None
        } else {
            Some(output(stdout, generation).unwrap())
        };
        (child, stdin, output)
    }
}

pub(super) const DESCENDANT: &str = r#"leaf=
cleanup_fixture() {
    trap '' HUP INT TERM
    if [ -n "$leaf" ]; then kill "$leaf" 2>/dev/null || :; wait "$leaf" 2>/dev/null || :; fi
    exit 0
}
trap cleanup_fixture HUP INT TERM
sleep 60 & leaf=$!
printf '%s %s\n' $$ "$leaf" > "$0.pids"
echo 'Web UI available at http://127.0.0.1:51001/?tkn=never-publish'
wait "$leaf" || :"#;

pub(super) fn until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(12);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "owned fixture did not finish: {what}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}
pub(super) fn pids(path: &Path) -> Vec<i32> {
    fs::read_to_string(path)
        .unwrap()
        .split_whitespace()
        .map(|pid| pid.parse().unwrap())
        .collect()
}
pub(super) fn dead(pid: i32) -> bool {
    let Ok(pid) = rustix::process::Pid::from_raw(pid).ok_or(()) else {
        return false;
    };
    rustix::process::test_kill_process(pid).is_err()
}
pub(super) fn no_runtime(root: &Path) -> bool {
    !root
        .join(".local/state/herdr-gpui")
        .read_dir()
        .unwrap()
        .any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("code-runtime.")
        })
}
fn cleanup(child: &mut Owned, root: &Path, owned: &[i32]) {
    until("wrapper exit", || child.0.try_wait().unwrap().is_some());
    until("all owned children reaped", || {
        owned.iter().all(|pid| dead(*pid))
    });
    assert!(no_runtime(root));
}

#[test]
fn stop_eof_lease_and_wrong_generation_clean_only_owned_processes() {
    let mut sentinel = Owned(Command::new("sleep").arg("60").spawn().unwrap());
    for mode in ["stop", "eof", "lease", "wrong"] {
        let fixture = Fixture::new(DESCENDANT);
        let (mut child, mut input, state) = fixture.start(7, 1, &fixture.bootstrap(), false);
        until("ready record", || {
            state.as_ref().unwrap().lock().unwrap().port.is_some()
        });
        assert!(state.as_ref().unwrap().lock().unwrap().starting);
        let owned = pids(&fixture.program.with_extension("pids"));
        match mode {
            "stop" => writeln!(input, "stop 7").unwrap(),
            "wrong" => {
                writeln!(input, "ping 8").unwrap();
                writeln!(input, "stop 8").unwrap();
            }
            _ => {}
        }
        if mode == "eof" {
            drop(input);
            cleanup(&mut child, fixture.root.path(), &owned);
        } else {
            cleanup(&mut child, fixture.root.path(), &owned);
            drop(input);
        }
        assert!(sentinel.0.try_wait().unwrap().is_none());
        assert!(
            fixture
                .root
                .path()
                .join(".local/state/herdr-gpui/code-data/server-a")
                .is_dir(),
            "persistent data survives owned runtime cleanup"
        );
    }
}

#[test]
fn unexpected_exit_bootstrap_cancel_and_closed_output_cleanup() {
    for mode in ["exit", "bootstrap", "closed-output"] {
        let fixture = Fixture::new(if mode == "exit" { "exit 1" } else { DESCENDANT });
        let bootstrap_pid = fixture.root.path().join("bootstrap-pid");
        let bootstrap = if mode == "bootstrap" {
            format!(
                "bootstrap() {{ sleep 60 & cli=$!; printf '%s\\n' \"$cli\" > {}; wait \"$cli\"; }}",
                herdr_client::shell_quote(&bootstrap_pid.to_string_lossy())
            )
        } else {
            fixture.bootstrap()
        };
        let (mut child, input, state) = fixture.start(9, 2, &bootstrap, mode == "closed-output");
        let owned = if mode == "bootstrap" {
            until("bootstrap child", || bootstrap_pid.exists());
            let owned = pids(&bootstrap_pid);
            rustix::process::kill_process(
                rustix::process::Pid::from_child(&child.0),
                rustix::process::Signal::TERM,
            )
            .unwrap();
            owned
        } else {
            until("CLI start", || fixture.root.path().join("pid").exists());
            if mode == "exit" {
                pids(&fixture.root.path().join("pid"))
            } else {
                until("CLI descendant", || {
                    fixture.program.with_extension("pids").exists()
                });
                pids(&fixture.program.with_extension("pids"))
            }
        };
        // Keep control open: output loss/CLI exit, not EOF, must initiate cleanup.
        cleanup(&mut child, fixture.root.path(), &owned);
        drop(input);
        if let Some(state) = state {
            until("output closed", || state.lock().unwrap().closed);
        }
    }
}

#[test]
fn two_wrappers_have_separate_groups_and_owned_data() {
    let a = Fixture::new(DESCENDANT);
    let b = Fixture::new(DESCENDANT);
    let (mut child_a, mut input_a, state_a) = a.start(1, 4, &a.bootstrap(), false);
    let (mut child_b, input_b, state_b) = b.start(2, 4, &b.bootstrap(), false);
    until("both ready", || {
        state_a.as_ref().unwrap().lock().unwrap().port.is_some()
            && state_b.as_ref().unwrap().lock().unwrap().port.is_some()
    });
    let owned_a = pids(&a.program.with_extension("pids"));
    let owned_b = pids(&b.program.with_extension("pids"));
    writeln!(input_a, "stop 1").unwrap();
    cleanup(&mut child_a, a.root.path(), &owned_a);
    assert!(owned_b.iter().all(|pid| !dead(*pid)));
    drop(input_b);
    cleanup(&mut child_b, b.root.path(), &owned_b);
}

#[test]
fn output_flood_after_ready_is_discarded_with_a_hard_disk_cap() {
    let fixture = Fixture::new(&DESCENDANT.replace(
        "wait \"$leaf\" || :",
        "kill \"$leaf\"; wait \"$leaf\" || :\nsleep 2\nyes never-publish-output & leaf=$!\nwait \"$leaf\" || :",
    ));
    let (mut child, mut input, state) = fixture.start(17, 10, &fixture.bootstrap(), false);
    until("ready before flood", || {
        state.as_ref().unwrap().lock().unwrap().port.is_some()
    });
    let base = fixture.root.path().join(".local/state/herdr-gpui");
    let runtime = base
        .read_dir()
        .unwrap()
        .find_map(|entry| {
            let entry = entry.unwrap();
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("code-runtime.")
                .then(|| entry.path())
        })
        .unwrap();
    until("capture full", || runtime.join("capture-full").exists());
    assert!(fs::metadata(runtime.join("output")).unwrap().len() <= 65536);
    assert!(child.0.try_wait().unwrap().is_none());
    writeln!(input, "stop 17").unwrap();
    cleanup(&mut child, fixture.root.path(), &[]);
}

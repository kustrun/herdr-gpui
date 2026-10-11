//! An owned foreground SSH wrapper plus an independently owned private tunnel.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]
use super::{
    Error,
    error::Result,
    install::Release,
    supervisor::{Address, free_port},
    token::Token,
};
use crate::browser::WebUrl;
use std::{
    io::{BufRead, BufReader, Read, Write},
    num::NonZeroU16,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const LEASE: u64 = 15;
const GRACE: u64 = 2;
const WRAPPER: &str = include_str!("remote/wrapper.sh");
const BOOTSTRAP: &str = include_str!("remote/bootstrap.sh");
const PRIVATE: &str = include_str!("remote/private.sh");
const PROBE: &str = r#"os=$(uname -s)
arch=$(uname -m)
printf '%s\n' "$os" "$arch"
candidate=$(command -v code 2>/dev/null || :)
case "$candidate" in /*) ;; *) candidate=;; esac
if [ -n "$candidate" ]; then
    sibling=${candidate%/*}/code-tunnel
    if [ -x "$sibling" ]; then candidate=$sibling; fi
    if "$candidate" --version 2>/dev/null | grep -Eq '[0-9a-f]{40}' && "$candidate" serve-web --help >/dev/null 2>&1; then echo installed; exit 0; fi
fi
case "$os/$arch" in
    Darwin/arm64) platform=cli-darwin-arm64;; Darwin/x86_64) platform=cli-darwin-x64;;
    Linux/aarch64) platform=cli-linux-arm64;; Linux/x86_64) platform=cli-linux-x64;;
    *) echo missing; exit 0;;
esac
cache=${XDG_CACHE_HOME:-"$HOME/.cache"}/herdr-gpui/vscode
if [ -e "$cache" ] || [ -L "$cache" ]; then
    private_path "$cache" 700 directory || { echo unsafe; exit 0; }
fi
pointer=$cache/current-$platform
if [ -e "$pointer" ] || [ -L "$pointer" ]; then
    private_path "$pointer" 600 file || { echo unsafe; exit 0; }
    commit=$(head -c 42 "$pointer") || { echo unsafe; exit 0; }
    case "$commit" in ''|*[!a-f0-9]*) ;; *)
        if [ "${#commit}" = 40 ]; then
            program=$cache/$commit-$platform
            if [ -e "$program" ] || [ -L "$program" ]; then
                private_path "$program" 700 file || { echo unsafe; exit 0; }
                if "$program" serve-web --help >/dev/null 2>&1; then echo installed; exit 0; fi
            fi
        fi;;
    esac
fi
echo missing"#;

pub(super) fn platform(target: &str, cancel: &AtomicBool) -> Result<(&'static str, bool)> {
    let mut output = Vec::new();
    let timed_out = AtomicBool::new(false);
    let done = AtomicBool::new(false);
    let result = thread::scope(|scope| {
        scope.spawn(|| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !done.load(Ordering::Acquire) {
                if cancel.load(Ordering::Acquire) || Instant::now() >= deadline {
                    timed_out.store(true, Ordering::Release);
                    return;
                }
                thread::sleep(Duration::from_millis(25));
            }
        });
        let result = herdr_client::run_script(
            herdr_client::ScriptHost::Ssh(target),
            &format!("{PRIVATE}\n{PROBE}"),
            std::io::empty(),
            &mut output,
            herdr_client::ScriptLimits {
                output: 256,
                idle: Duration::from_secs(15),
            },
            &timed_out,
        );
        done.store(true, Ordering::Release);
        result
    });
    super::install::check(cancel)?;
    result.map_err(|error| match error {
        herdr_client::Error::ScriptExit { status, stderr } => Error::Ssh(
            herdr_client::SshFailure::classify(status.code(), stderr.as_bytes()),
        ),
        error => Error::Probe(error),
    })?;
    let text = std::str::from_utf8(&output).map_err(|_| Error::UnsupportedTarget)?;
    let mut lines = text.lines();
    let platform =
        super::install::platform(lines.next().unwrap_or(""), lines.next().unwrap_or(""))?;
    match lines.next() {
        Some("installed") => Ok((platform, true)),
        Some("missing") => Ok((platform, false)),
        _ => Err(Error::RemoteInstall),
    }
}

fn script(
    release: &Release,
    generation: u64,
    lease: u64,
    grace: u64,
    bootstrap: &str,
    target_key: &str,
) -> String {
    let quote = herdr_client::shell_quote;
    format!(
        "target_key={}\ngeneration={generation}\nlease={lease}\ngrace={grace}\ncommit={}\nplatform={}\nartifact={}\nsha256={}\n{PRIVATE}\n{bootstrap}\n{WRAPPER}",
        quote(target_key),
        quote(&release.commit),
        quote(release.platform),
        quote(&release.url),
        quote(&release.sha256)
    )
}

struct Owned(Child);
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Only validated records are retained; remote output and stderr are discarded.
#[derive(Default)]
struct Output {
    starting: bool,
    port: Option<NonZeroU16>,
    failure: Option<u8>,
    closed: bool,
}

fn output(reader: impl Read + Send + 'static, generation: u64) -> Result<Arc<Mutex<Output>>> {
    let state = Arc::new(Mutex::new(Output::default()));
    let updated = state.clone();
    thread::Builder::new()
        .name("herdr-code-ssh-output".into())
        .spawn(move || {
            let mut reader = BufReader::new(reader);
            let mut bytes = Vec::new();
            loop {
                bytes.clear();
                let n = reader.by_ref().take(257).read_until(b'\n', &mut bytes);
                if !matches!(n, Ok(1..=256)) {
                    break;
                }
                let Ok(text) = std::str::from_utf8(&bytes) else {
                    continue;
                };
                let fields: Vec<_> = text.split_whitespace().collect();
                if fields.len() != 3 || fields[1].parse::<u64>().ok() != Some(generation) {
                    continue;
                }
                let mut state = updated.lock().unwrap_or_else(PoisonError::into_inner);
                match fields[0] {
                    "START" if fields[2] == "0" => state.starting = true,
                    "READY" => state.port = fields[2].parse().ok(),
                    "FAIL" => state.failure = fields[2].parse().ok(),
                    _ => {}
                }
            }
            updated
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .closed = true;
        })
        .map_err(Error::Worker)?;
    Ok(state)
}

fn failure(code: u8) -> Error {
    match code {
        1 => Error::RemoteTools,
        2 => Error::RemoteInstall,
        4 => Error::RemoteExited,
        5 => Error::Ssh(herdr_client::SshFailure::Unreachable),
        _ => Error::RemoteOwnership,
    }
}

pub(super) enum Event {
    Installing,
    Starting,
    Origin(NonZeroU16),
}

pub(super) struct Remote {
    wrapper: Owned,
    tunnel: Owned,
    _tunnel_control: ChildStdin,
    control: ChildStdin,
    output: Arc<Mutex<Output>>,
    generation: u64,
    ping: Instant,
    pub(super) address: Address,
}

impl Remote {
    pub(super) fn start(
        target: &str,
        target_key: &str,
        generation: u64,
        token: &Token,
        port: Option<NonZeroU16>,
        cancel: &AtomicBool,
        publish: impl Fn(Event) -> Result<()>,
    ) -> Result<Self> {
        let (platform, installed) = platform(target, cancel)?;
        let release = if installed {
            Release {
                commit: String::new(),
                platform,
                url: String::new(),
                sha256: String::new(),
            }
        } else {
            publish(Event::Installing)?;
            super::install::resolve(platform, cancel)?
        };
        let command = herdr_client::script_command(
            target,
            &script(&release, generation, LEASE, GRACE, BOOTSTRAP, target_key),
        )
        .map_err(Error::Target)?;
        let (wrapper, control, output, remote) =
            start_wrapper(command, generation, token, cancel, || {
                publish(Event::Starting)
            })?;
        let local = match port {
            Some(port) => port,
            None => free_port()?,
        };
        let mut command =
            herdr_client::controlled_forward_command(target, local.get(), remote.get())
                .map_err(Error::Target)?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut tunnel = Owned(command.spawn().map_err(Error::RemoteIo)?);
        let tunnel_control = tunnel
            .0
            .stdin
            .take()
            .ok_or_else(|| Error::RemoteIo(std::io::ErrorKind::BrokenPipe.into()))?;
        let ready = Arc::new(AtomicBool::new(false));
        let mark = ready.clone();
        let stdout = tunnel
            .0
            .stdout
            .take()
            .ok_or_else(|| Error::RemoteIo(std::io::ErrorKind::BrokenPipe.into()))?;
        thread::Builder::new()
            .name("herdr-code-tunnel".into())
            .spawn(move || {
                let mut text = String::new();
                if BufReader::new(stdout)
                    .take(256)
                    .read_line(&mut text)
                    .is_ok()
                    && text.trim() == herdr_client::FORWARD_READY
                {
                    mark.store(true, Ordering::Release);
                }
            })
            .map_err(Error::Worker)?;
        let deadline = Instant::now() + Duration::from_secs(15);
        while !ready.load(Ordering::Acquire) {
            super::install::check(cancel)?;
            if tunnel.0.try_wait().map_err(Error::RemoteIo)?.is_some() {
                return Err(Error::OriginTaken { port: local.get() });
            }
            if Instant::now() >= deadline {
                return Err(Error::Silent(Duration::from_secs(15)));
            }
            thread::sleep(Duration::from_millis(25));
        }
        // Ownership acknowledgement precedes persistence and every HTTP request.
        if port.is_none() {
            publish(Event::Origin(local))?;
        }
        let url = WebUrl::try_from(format!("http://127.0.0.1:{local}/?tkn={}", token.as_str()))
            .map_err(|_| Error::RemoteOwnership)?;
        Ok(Self {
            wrapper,
            tunnel,
            _tunnel_control: tunnel_control,
            control,
            output,
            generation,
            ping: Instant::now(),
            address: Address { port: local, url },
        })
    }

    pub(super) fn alive(&mut self) -> bool {
        self.wrapper.0.try_wait().is_ok_and(|exit| exit.is_none())
            && self.tunnel.0.try_wait().is_ok_and(|exit| exit.is_none())
            && !self
                .output
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .closed
    }

    /// Only nonblocking writes and exact-child signals; reaping stays on workers.
    pub(super) fn stop(&mut self) {
        let _ = self
            .control
            .write_all(format!("stop {}\n", self.generation).as_bytes());
        let _ = self.wrapper.0.kill();
        let _ = self.tunnel.0.kill();
    }

    /// A successful HTTP answer counts only while both owned children outlive it.
    pub(super) fn readiness(
        &mut self,
        probe: impl FnOnce(&WebUrl) -> crate::Result<super::Server>,
    ) -> Result<bool> {
        let answered = probe(&self.address.url).is_ok();
        self.tick()?;
        Ok(answered)
    }

    pub(super) fn tick(&mut self) -> Result<()> {
        if self.ping.elapsed() >= Duration::from_secs(3) {
            self.control
                .write_all(format!("ping {}\n", self.generation).as_bytes())
                .map_err(Error::RemoteIo)?;
            self.ping = Instant::now();
        }
        if let Some(code) = self
            .output
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .failure
        {
            return Err(failure(code));
        }
        if !self.alive() {
            return Err(Error::Ssh(herdr_client::SshFailure::Unreachable));
        }
        Ok(())
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        let _ = self
            .control
            .write_all(format!("stop {}\n", self.generation).as_bytes());
        // EOF/network loss invokes the same remote cleanup; lease also covers
        // transport failures that leave a half-open remote control channel.
    }
}

fn start_wrapper(
    mut command: Command,
    generation: u64,
    token: &Token,
    cancel: &AtomicBool,
    starting: impl FnOnce() -> Result<()>,
) -> Result<(Owned, ChildStdin, Arc<Mutex<Output>>, NonZeroU16)> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut wrapper = Owned(command.spawn().map_err(Error::RemoteIo)?);
    let mut control = wrapper
        .0
        .stdin
        .take()
        .ok_or_else(|| Error::RemoteIo(std::io::ErrorKind::BrokenPipe.into()))?;
    let flags = rustix::fs::fcntl_getfl(&control).map_err(|error| Error::RemoteIo(error.into()))?;
    rustix::fs::fcntl_setfl(&control, flags | rustix::fs::OFlags::NONBLOCK)
        .map_err(|error| Error::RemoteIo(error.into()))?;
    control
        .write_all(format!("token {generation} {}\n", token.as_str()).as_bytes())
        .map_err(Error::RemoteIo)?;
    let stdout = wrapper
        .0
        .stdout
        .take()
        .ok_or_else(|| Error::RemoteIo(std::io::ErrorKind::BrokenPipe.into()))?;
    let output = output(stdout, generation)?;
    let stderr = wrapper
        .0
        .stderr
        .take()
        .ok_or_else(|| Error::RemoteIo(std::io::ErrorKind::BrokenPipe.into()))?;
    let diagnostic = Arc::new(Mutex::new(Vec::new()));
    let recorded = diagnostic.clone();
    thread::Builder::new()
        .name("herdr-code-ssh-error".into())
        .spawn(move || {
            let mut reader = stderr;
            let mut buffer = [0; 1024];
            while let Ok(n) = reader.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                let mut tail = recorded.lock().unwrap_or_else(PoisonError::into_inner);
                if tail.len() + n > 4096 {
                    tail.clear();
                }
                tail.extend_from_slice(&buffer[..n]);
            }
        })
        .map_err(Error::Worker)?;
    let mut starting = Some(starting);
    let deadline = Instant::now() + Duration::from_secs(600);
    let mut ping = Instant::now();
    loop {
        super::install::check(cancel)?;
        if let Some(status) = wrapper.0.try_wait().map_err(Error::RemoteIo)? {
            let tail = diagnostic.lock().unwrap_or_else(PoisonError::into_inner);
            return Err(Error::Ssh(herdr_client::SshFailure::classify(
                status.code(),
                &tail,
            )));
        }
        {
            let state = output.lock().unwrap_or_else(PoisonError::into_inner);
            if state.starting
                && let Some(starting) = starting.take()
            {
                starting()?;
            }
            if let Some(code) = state.failure {
                return Err(failure(code));
            }
            if let Some(port) = state.port {
                return Ok((wrapper, control, output.clone(), port));
            }
            if state.closed {
                return Err(Error::RemoteOwnership);
            }
        }
        if Instant::now() >= deadline {
            return Err(Error::Silent(Duration::from_secs(600)));
        }
        if ping.elapsed() >= Duration::from_secs(3) {
            control
                .write_all(format!("ping {generation}\n").as_bytes())
                .map_err(Error::RemoteIo)?;
            ping = Instant::now();
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(test)]
mod tests;

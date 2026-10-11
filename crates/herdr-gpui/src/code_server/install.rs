//! Official immutable CLI releases, verified before exact-file extraction.
//! Staging and atomic publication follow the updater's private-cache policy.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]
use super::{Error, error::Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

const LIMIT: u64 = 64 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone, Debug)]
pub(super) struct Release {
    pub(super) commit: String,
    pub(super) platform: &'static str,
    pub(super) url: String,
    pub(super) sha256: String,
}

#[derive(Deserialize)]
struct Metadata {
    version: String,
    url: String,
    sha256hash: String,
}

pub(super) fn platform(os: &str, arch: &str) -> Result<&'static str> {
    match (os, arch) {
        ("Darwin", "arm64" | "aarch64") => Ok("cli-darwin-arm64"),
        ("Darwin", "x86_64") => Ok("cli-darwin-x64"),
        ("Linux", "aarch64" | "arm64") => Ok("cli-linux-arm64"),
        ("Linux", "x86_64") => Ok("cli-linux-x64"),
        _ => Err(Error::UnsupportedTarget),
    }
}

fn hex(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn official_url(text: &str) -> bool {
    url::Url::parse(text).is_ok_and(|url| {
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && url.host_str().is_some_and(|host| {
                matches!(
                    host,
                    "vscode.download.prss.microsoft.com"
                        | "az764295.vo.msecnd.net"
                        | "update.code.visualstudio.com"
                )
            })
    })
}

fn metadata(body: &[u8], platform: &'static str) -> Result<Release> {
    if body.len() > 16384 {
        return Err(Error::Metadata);
    }
    let data: Metadata = serde_json::from_slice(body).map_err(|_| Error::Metadata)?;
    if body.len() > 16384
        || !hex(&data.version, 40)
        || !hex(&data.sha256hash, 64)
        || !official_url(&data.url)
    {
        return Err(Error::Metadata);
    }
    Ok(Release {
        commit: data.version,
        platform,
        url: data.url,
        sha256: data.sha256hash,
    })
}

pub(super) fn resolve(platform: &'static str, cancelled: &AtomicBool) -> Result<Release> {
    check(cancelled)?;
    let mut response = agent()
        .get(format!(
            "https://update.code.visualstudio.com/api/update/{platform}/stable/latest"
        ))
        .call()
        .map_err(Error::MetadataRequest)?;
    let mut body = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(16385)
        .read_to_end(&mut body)
        .map_err(Error::Cache)?;
    check(cancelled)?;
    metadata(&body, platform)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .max_redirects(0)
        .build()
        .into()
}

pub(super) fn check(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

fn download(release: &Release, destination: &Path, cancelled: &AtomicBool) -> Result<()> {
    // Metadata names the immutable artifact directly. Redirects cannot downgrade TLS.
    let mut response = agent().get(&release.url).call().map_err(Error::Download)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(destination)
        .map_err(Error::Cache)?;
    verify_copy(
        response.body_mut().as_reader(),
        &mut file,
        &release.sha256,
        cancelled,
    )?;
    file.sync_all().map_err(Error::Cache)
}

fn verify_copy(
    mut input: impl Read,
    mut output: impl Write,
    expected: &str,
    cancelled: &AtomicBool,
) -> Result<()> {
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0; 65536];
    loop {
        check(cancelled)?;
        let n = input.read(&mut buffer).map_err(Error::Cache)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > LIMIT {
            return Err(Error::Digest);
        }
        hash.update(&buffer[..n]);
        output.write_all(&buffer[..n]).map_err(Error::Cache)?;
    }
    check(cancelled)?;
    if hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        != expected
    {
        return Err(Error::Digest);
    }
    Ok(())
}

fn extract(input: impl Read, destination: &Path, cancelled: &AtomicBool) -> Result<PathBuf> {
    let decoder = flate2::read::MultiGzDecoder::new(input);
    let mut archive = tar::Archive::new(decoder.take(LIMIT + 1));
    let program = destination.join("code");
    let mut found = false;
    for entry in archive.entries().map_err(Error::Cache)?.raw(true) {
        check(cancelled)?;
        let mut entry = entry.map_err(Error::Cache)?;
        if found
            || entry.path().map_err(Error::Cache)?.as_ref() != Path::new("code")
            || !entry.header().entry_type().is_file()
            || entry.size() > LIMIT
            || entry.header().mode().map_err(Error::Cache)? & 0o111 == 0
        {
            return Err(Error::Archive);
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&program)
            .map_err(Error::Cache)?;
        let mut buffer = [0; 65536];
        loop {
            check(cancelled)?;
            let n = entry.read(&mut buffer).map_err(Error::Cache)?;
            if n == 0 {
                break;
            }
            file.write_all(&buffer[..n]).map_err(Error::Cache)?;
        }
        file.sync_all().map_err(Error::Cache)?;
        found = true;
    }
    let mut rest = archive.into_inner();
    let mut buffer = [0; 65536];
    loop {
        check(cancelled)?;
        let n = rest.read(&mut buffer).map_err(Error::Cache)?;
        if n == 0 {
            break;
        }
        if buffer[..n].iter().any(|b| *b != 0) {
            return Err(Error::Archive);
        }
    }
    if !found || rest.limit() == 0 {
        return Err(Error::Archive);
    }
    Ok(program)
}

/// A bounded capability check, with output discarded because CLI diagnostics
/// can contain local paths. Tests supply only isolated synthetic executables.
fn command_output(program: &Path, args: &[&str], cancelled: &AtomicBool) -> Result<Option<String>> {
    check(cancelled)?;
    let mut command = Command::new(program);
    command.args(args);
    super::supervisor::process::isolate(&mut command);
    let spawn_deadline = Instant::now() + Duration::from_secs(1);
    let mut child = loop {
        check(cancelled)?;
        match command.spawn() {
            Ok(child) => break child,
            // Another thread's fork can briefly inherit a staging writer fd.
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && Instant::now() < spawn_deadline =>
            {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(Error::Cache(error)),
        }
    };
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Cache(std::io::ErrorKind::BrokenPipe.into()))?;
    let output = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(4097).read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut timed_out = false;
    let mut watch_error = None;
    loop {
        match super::supervisor::process::exited(&mut child) {
            Ok(true) => break,
            Ok(false) => {}
            Err(error) => {
                watch_error = Some(error);
                break;
            }
        }
        if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
            timed_out = true;
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    // Retain the leader until its entire owned group has been stopped.
    super::supervisor::process::kill(&mut child);
    let status = child.wait().map_err(Error::Cache)?;
    let bytes = output
        .join()
        .map_err(|_| Error::Capability)?
        .map_err(Error::Cache)?;
    check(cancelled)?;
    if let Some(error) = watch_error {
        return Err(Error::Cache(error));
    }
    if timed_out || !status.success() || bytes.len() > 4096 {
        return Ok(None);
    }
    Ok(String::from_utf8(bytes).ok())
}

pub(super) fn capable(program: &Path, cancelled: &AtomicBool) -> Result<bool> {
    let Some(version) = command_output(program, &["--version"], cancelled)? else {
        return Ok(false);
    };
    let valid = version
        .split(|c: char| !c.is_ascii_hexdigit())
        .any(|word| hex(word, 40));
    if !valid {
        return Ok(false);
    }
    Ok(command_output(program, &["serve-web", "--help"], cancelled)?.is_some())
}

fn cached(root: &Path, cancelled: &AtomicBool) -> Result<Option<PathBuf>> {
    super::private::directory(root)?;
    let pointer = root.join("current");
    let Some(file) = super::private::file(&pointer, 0o600)? else {
        return Ok(None);
    };
    let mut name = String::new();
    file.take(128)
        .read_to_string(&mut name)
        .map_err(Error::Cache)?;
    let Some((commit, platform)) = name.trim().split_once('-') else {
        return Ok(None);
    };
    if !hex(commit, 40)
        || !matches!(
            platform,
            "cli-darwin-arm64" | "cli-darwin-x64" | "cli-linux-arm64" | "cli-linux-x64"
        )
    {
        return Ok(None);
    }
    let destination = root.join(name.trim());
    let program = destination.join("code");
    if cached_program(&destination, &program, cancelled)? {
        Ok(Some(program))
    } else {
        Ok(None)
    }
}

fn cached_program(destination: &Path, program: &Path, cancelled: &AtomicBool) -> Result<bool> {
    Ok(super::private::existing_directory(destination)?
        && super::private::file(program, 0o700)?.is_some()
        && capable(program, cancelled)?)
}

fn remember(root: &Path, release: &Release) -> Result<()> {
    // Reject planted links instead of replacing them or reading their targets.
    super::private::file(&root.join("current"), 0o600)?;
    let mut pointer = tempfile::NamedTempFile::new_in(root).map_err(Error::Cache)?;
    write!(pointer, "{}-{}", release.commit, release.platform).map_err(Error::Cache)?;
    pointer.as_file().sync_all().map_err(Error::Cache)?;
    pointer
        .persist(root.join("current"))
        .map_err(|error| Error::Cache(error.error))?;
    Ok(())
}

pub(super) fn ensure(
    root: &Path,
    cancelled: &AtomicBool,
    installing: impl FnOnce(),
) -> Result<PathBuf> {
    if let Some(program) = super::cli::find() {
        match capable(&program, cancelled) {
            Ok(true) => return Ok(program),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            _ => {}
        }
    }
    if let Some(program) = cached(root, cancelled)? {
        return Ok(program);
    }
    let platform = platform(
        if cfg!(target_os = "macos") {
            "Darwin"
        } else {
            "Linux"
        },
        std::env::consts::ARCH,
    )?;
    installing();
    let release = resolve(platform, cancelled)?;
    publish(root, &release, cancelled, |path| {
        download(&release, path, cancelled)
    })
}

fn publish(
    root: &Path,
    release: &Release,
    cancelled: &AtomicBool,
    fetch: impl FnOnce(&Path) -> Result<()>,
) -> Result<PathBuf> {
    super::private::directory(root)?;
    let destination = root.join(format!("{}-{}", release.commit, release.platform));
    let program = destination.join("code");
    if cached_program(&destination, &program, cancelled)? {
        remember(root, release)?;
        return Ok(program);
    }
    let staging = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(root)
        .map_err(Error::Cache)?;
    let archive = staging.path().join("cli.tar.gz");
    fetch(&archive)?;
    check(cancelled)?;
    // Recheck immediately before extraction, following the updater's policy.
    verify_copy(
        File::open(&archive).map_err(Error::Cache)?,
        std::io::sink(),
        &release.sha256,
        cancelled,
    )?;
    let candidate = extract(
        File::open(&archive).map_err(Error::Cache)?,
        staging.path(),
        cancelled,
    )?;
    if !capable(&candidate, cancelled)?
        || !command_output(&candidate, &["--version"], cancelled)?
            .is_some_and(|text| text.contains(&release.commit))
    {
        return Err(Error::Capability);
    }
    fs::remove_file(archive).map_err(Error::Cache)?;
    check(cancelled)?;
    match fs::rename(staging.path(), &destination) {
        Ok(()) => {
            remember(root, release)?;
            Ok(program)
        }
        Err(_) if cached_program(&destination, &program, cancelled)? => {
            remember(root, release)?;
            Ok(program)
        }
        Err(source) => Err(Error::Cache(source)),
    }
}

#[cfg(test)]
mod tests;

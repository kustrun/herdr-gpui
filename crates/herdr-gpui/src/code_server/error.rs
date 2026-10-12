//! What goes wrong asking a VS Code server whether it is there, or starting
//! the one the app runs. No message names the connection token: addresses
//! are named by host and port alone.
use super::NoAnswer;
use std::{io, path::PathBuf, process::ExitStatus, time::Duration};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("This machine cannot run a managed VS Code editor.")]
    UnsupportedTarget,
    #[error("Invalid editor machine connection.")]
    Target(#[source] herdr_client::Error),
    #[error("VS Code installation was cancelled.")]
    Cancelled,
    #[error("Could not read official VS Code release metadata.")]
    MetadataRequest(#[source] ureq::Error),
    #[error("Official VS Code release metadata was invalid.")]
    Metadata,
    #[error("Could not download the VS Code command line.")]
    Download(#[source] ureq::Error),
    #[error("The VS Code archive failed its size or SHA256 check. Retry to download it again.")]
    Digest,
    #[error("The VS Code archive contains an unexpected path or file type.")]
    Archive,
    #[error(
        "Managed VS Code state must be private, owned directories and regular files. Check permissions and retry."
    )]
    UnsafeCache,
    #[error("Could not use the private VS Code cache.")]
    Cache(#[source] io::Error),
    #[error("The VS Code command line does not support serve-web.")]
    Capability,
    #[error("Could not run an editor machine probe.")]
    Probe(#[source] herdr_client::Error),
    #[error("The editor SSH connection failed: {0}.")]
    Ssh(herdr_client::SshFailure),
    #[error("The remote editor requires curl, tar, gzip and sha256sum or shasum.")]
    RemoteTools,
    #[error("The remote editor runtime failed its ownership or readiness check.")]
    RemoteOwnership,
    #[error("The remote VS Code process stopped unexpectedly.")]
    RemoteExited,
    #[error("The remote VS Code installation failed. Check platform compatibility and retry.")]
    RemoteInstall,
    #[error("Could not control the editor SSH connection.")]
    RemoteIo(#[source] io::Error),
    #[error(
        "Port {port} on 127.0.0.1 is in use. Close the program using it, then Retry; the editor keeps this address to preserve its settings."
    )]
    OriginTaken { port: u16 },
    /// Only the host and port are named: the address's query may hold the
    /// server's connection token.
    #[error("{reason} at {address}.")]
    Unreachable {
        address: String,
        reason: NoAnswer,
        #[source]
        source: ureq::Error,
    },
    #[error("This address is not a VS Code server (HTTP {status}).")]
    NotServer { status: u16 },
    #[error("The server refused the connection token. Use the address it prints, with its ?tkn=.")]
    TokenRefused,
    #[error("The VS Code server answered HTTP {0}.")]
    Status(u16),
    #[error("There is no folder for the app's data. Set HOME.")]
    NoDataDir,
    #[error("Could not use the VS Code token file {}.", .path.display())]
    TokenFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Could not make a VS Code connection token.")]
    TokenRandom(#[source] getrandom::Error),
    #[error("Could not find a free port for VS Code.")]
    NoFreePort(#[source] io::Error),
    #[error(
        "Port {port} on 127.0.0.1 is in use by another program. Quit it, or remove `port` \
         under [code] in config-gpui.local.toml to pick another one; VS Code then starts \
         without the settings it kept for this port."
    )]
    PortTaken { port: u16 },
    #[error("Could not check port {port} on 127.0.0.1.")]
    PortCheck {
        port: u16,
        #[source]
        source: io::Error,
    },
    #[error("Could not start {}.", .program.display())]
    Spawn {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("VS Code stopped ({0}).")]
    Exited(ExitStatus),
    #[error("Could not watch the VS Code process.")]
    Watch(#[source] io::Error),
    #[error("VS Code did not answer within {} seconds.", .0.as_secs())]
    Silent(Duration),
    #[error("Could not start the VS Code worker.")]
    Worker(#[source] io::Error),
}

pub(super) type Result<T> = std::result::Result<T, Error>;

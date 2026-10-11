//! Persisted backing machines. Labels and Herdr sessions are presentation,
//! while an edited SSH destination is a different trust and storage boundary.
use super::{Error, error::Result};
use crate::config::{CodeConfig, CodeMode};
use herdr_client::{ConnectTarget, SavedHost};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Target {
    #[default]
    Configured,
    Local,
    Ssh {
        id: String,
        fingerprint: String,
    },
}

impl Target {
    pub(crate) fn saved(host: &SavedHost) -> Result<Self> {
        // Validate exactly the descriptor the transport accepts, before hashing it.
        match herdr_client::forward_command(&host.target, 1, 1) {
            Ok(_) | Err(herdr_client::Error::SshUnsupported) => {}
            Err(error) => return Err(Error::Target(error)),
        }
        if host.id.is_empty() || host.id.len() > 256 || host.id.chars().any(char::is_control) {
            return Err(Error::TargetUnavailable);
        }
        Ok(Self::Ssh {
            id: host.id.clone(),
            fingerprint: digest(host.target.as_bytes()),
        })
    }

    pub(crate) fn valid(&self) -> bool {
        match self {
            Self::Configured | Self::Local => true,
            Self::Ssh { id, fingerprint } => {
                !id.is_empty()
                    && id.len() <= 256
                    && !id.chars().any(char::is_control)
                    && fingerprint.len() == 64
                    && fingerprint
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }
        }
    }

    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) fn resolve(&self, hosts: &[SavedHost]) -> Result<Option<String>> {
        match self {
            Self::Configured | Self::Local => Ok(None),
            Self::Ssh { id, .. } => hosts
                .iter()
                .find(|host| &host.id == id && host.enabled)
                .filter(|host| Self::saved(host).is_ok_and(|target| &target == self))
                .map(|host| Some(host.target.clone()))
                .ok_or(Error::TargetUnavailable),
        }
    }

    pub(crate) fn owner(
        owner: &ConnectTarget,
        saved_id: Option<&str>,
        hosts: &[SavedHost],
    ) -> Result<Self> {
        match owner {
            ConnectTarget::Local | ConnectTarget::Session { .. } | ConnectTarget::Socket(_) => {
                Ok(Self::Local)
            }
            ConnectTarget::Ssh { target, .. } => hosts
                .iter()
                .find(|host| {
                    host.enabled && Some(host.id.as_str()) == saved_id && &host.target == target
                })
                .ok_or(Error::TargetUnavailable)
                .and_then(Self::saved),
            _ => Err(Error::UnsupportedTarget),
        }
    }

    pub(crate) fn default_for(
        code: &CodeConfig,
        owner: &ConnectTarget,
        saved_id: Option<&str>,
        hosts: &[SavedHost],
    ) -> Result<Self> {
        if code.mode == Some(CodeMode::Address) || (code.mode.is_none() && code.url.is_some()) {
            Ok(Self::Configured)
        } else {
            Self::owner(owner, saved_id, hosts)
        }
    }

    /// A filesystem-safe, nonsecret key, also suitable for native profile UUID input.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) fn key(&self) -> String {
        match self {
            Self::Configured => "configured".into(),
            Self::Local => "local".into(),
            Self::Ssh { id, fingerprint } => format!(
                "ssh-{}",
                digest(format!("{}:{id}{fingerprint}", id.len()).as_bytes())
            ),
        }
    }
}

pub(super) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests;

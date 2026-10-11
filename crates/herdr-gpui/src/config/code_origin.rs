//! A managed server's acknowledgement must not overwrite a newer explicit edit.
use super::{Config, LOCAL_CONFIG, files::write_config};
use crate::{Error, Result};
use std::{
    fs,
    num::NonZeroU16,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

impl Config {
    pub(crate) fn save_managed_code_port(port: NonZeroU16, cancelled: &AtomicBool) -> Result<bool> {
        let (_lock, local) = Self::prepare_files(&Self::path()?)?;
        Self::save_managed_code_port_path(port, cancelled, &local)
    }

    pub(super) fn save_managed_code_port_path(
        port: NonZeroU16,
        cancelled: &AtomicBool,
        path: &Path,
    ) -> Result<bool> {
        let result = (|| {
            let text = match fs::read_to_string(path) {
                Ok(text) => text,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => LOCAL_CONFIG.into(),
                Err(error) => return Err(error.into()),
            };
            let mut document = text.parse::<toml_edit::DocumentMut>()?;
            let code = document
                .entry("code")
                .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
                .as_table_like_mut()
                .ok_or(Error::InvalidCodeTable)?;
            if code.contains_key("port") || cancelled.load(Ordering::Acquire) {
                return Ok(false);
            }
            code.insert("port", toml_edit::value(i64::from(port.get())));
            if cancelled.load(Ordering::Acquire) {
                return Ok(false);
            }
            write_config(path, &document.to_string())?;
            Ok(true)
        })();
        result.map_err(|error: Error| error.at_path(path))
    }
}

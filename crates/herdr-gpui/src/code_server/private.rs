//! Private managed state follows the existing Unix credential/cache policy.
use super::{Error, error::Result};
use rustix::fs::{Mode, OFlags, open};
use std::{
    fs::{DirBuilder, File, Metadata},
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::Path,
};

fn io(error: rustix::io::Errno) -> Error {
    match error {
        rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => Error::UnsafeCache,
        error => Error::Cache(error.into()),
    }
}

fn checked(meta: &Metadata, directory: bool, mode: u32) -> Result<()> {
    if meta.is_dir() != directory
        || (!directory && (!meta.is_file() || meta.nlink() != 1))
        || meta.uid() != rustix::process::geteuid().as_raw()
        || meta.mode() & 0o7777 != mode
    {
        return Err(Error::UnsafeCache);
    }
    Ok(())
}

pub(super) fn existing_directory(path: &Path) -> Result<bool> {
    let file = match open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => File::from(fd),
        Err(rustix::io::Errno::NOENT) => return Ok(false),
        Err(error) => return Err(io(error)),
    };
    checked(&file.metadata().map_err(Error::Cache)?, true, 0o700)?;
    Ok(true)
}

pub(super) fn directory(path: &Path) -> Result<()> {
    if !existing_directory(path)? {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(Error::Cache)?;
        existing_directory(path)?;
    }
    Ok(())
}

pub(super) fn file(path: &Path, mode: u32) -> Result<Option<File>> {
    let file = match open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => File::from(fd),
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(io(error)),
    };
    checked(&file.metadata().map_err(Error::Cache)?, false, mode)?;
    Ok(Some(file))
}

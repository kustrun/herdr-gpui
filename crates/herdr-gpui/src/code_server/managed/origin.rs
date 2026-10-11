//! Acknowledged origins survive app restarts. Collisions never pick a new port.
use super::*;
use std::io::Read;

pub(super) fn load(path: &Path) -> Result<Option<NonZeroU16>> {
    let Some(file) = super::super::private::file(path, 0o600)? else {
        return Ok(None);
    };
    let mut text = String::new();
    file.take(8)
        .read_to_string(&mut text)
        .map_err(Error::Cache)?;
    text.trim()
        .parse()
        .map(Some)
        .map_err(|_| Error::RemoteOwnership)
}

#[cfg(test)]
pub(super) fn save(path: &Path, port: NonZeroU16) -> Result<()> {
    save_active(path, port, &AtomicBool::new(false))
}

pub(super) fn save_active(path: &Path, port: NonZeroU16, cancelled: &AtomicBool) -> Result<()> {
    use rustix::fs::{Mode, OFlags, open};
    use std::{fs::File, io::Write};
    let parent = path.parent().ok_or(Error::NoDataDir)?;
    super::super::private::directory(parent)?;
    let lock_path = path.with_extension("lock");
    super::super::private::file(&lock_path, 0o600)?;
    let lock = File::from(
        open(
            &lock_path,
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|error| Error::Cache(error.into()))?,
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        install::check(cancelled)?;
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(Error::Cache(std::io::ErrorKind::WouldBlock.into()));
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(Error::Cache(error)),
        }
    }
    install::check(cancelled)?;
    super::super::private::file(path, 0o600)?;
    let mut staging = tempfile::NamedTempFile::new_in(parent).map_err(Error::Cache)?;
    write!(staging, "{port}").map_err(Error::Cache)?;
    staging.as_file().sync_all().map_err(Error::Cache)?;
    install::check(cancelled)?;
    staging
        .persist(path)
        .map_err(|error| Error::Cache(error.error))?;
    Ok(())
}

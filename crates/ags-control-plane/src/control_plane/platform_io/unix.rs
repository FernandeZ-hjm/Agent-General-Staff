#![allow(clippy::unnecessary_cast, clippy::useless_conversion)] // stat field widths differ per platform
use rustix::fd::OwnedFd;
use rustix::fs::FileType;
use std::io::Read;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StableStat {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) mode: u32,
    pub(crate) size: u64,
    pub(crate) mtime: i64,
    pub(crate) mtime_nsec: i64,
    pub(crate) ctime: i64,
    pub(crate) ctime_nsec: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StableRead {
    pub(crate) bytes: Vec<u8>,
    pub(crate) stable_stat: StableStat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StableReadError {
    NotRegular,
    TooLarge,
    Changed,
    Io(String),
}

pub(crate) fn read_regular_fd(
    descriptor: &OwnedFd,
    limit: u64,
    after_read_hook: impl FnOnce(),
) -> Result<StableRead, StableReadError> {
    let before =
        rustix::fs::fstat(descriptor).map_err(|error| StableReadError::Io(error.to_string()))?;
    if !FileType::from_raw_mode(before.st_mode).is_file() {
        return Err(StableReadError::NotRegular);
    }
    if before.st_size < 0 || before.st_size as u64 > limit {
        return Err(StableReadError::TooLarge);
    }
    let capacity = usize::try_from(before.st_size).map_err(|_| StableReadError::TooLarge)?;
    let reader =
        rustix::io::dup(descriptor).map_err(|error| StableReadError::Io(error.to_string()))?;
    let mut file = std::fs::File::from(reader);
    let mut bytes = Vec::with_capacity(capacity);
    file.read_to_end(&mut bytes)
        .map_err(|error| StableReadError::Io(error.to_string()))?;
    if bytes.len() as u64 > limit {
        return Err(StableReadError::TooLarge);
    }
    after_read_hook();
    let after =
        rustix::fs::fstat(descriptor).map_err(|error| StableReadError::Io(error.to_string()))?;
    if !same_file_version(&before, &after) || bytes.len() as u64 != after.st_size as u64 {
        return Err(StableReadError::Changed);
    }
    Ok(StableRead {
        bytes,
        stable_stat: StableStat {
            device: before.st_dev as u64,
            inode: before.st_ino,
            mode: before.st_mode as u32,
            size: before.st_size as u64,
            mtime: before.st_mtime,
            mtime_nsec: before.st_mtime_nsec as i64,
            ctime: before.st_ctime,
            ctime_nsec: before.st_ctime_nsec as i64,
        },
    })
}

fn same_file_version(before: &rustix::fs::Stat, after: &rustix::fs::Stat) -> bool {
    before.st_dev == after.st_dev
        && before.st_ino == after.st_ino
        && before.st_mode == after.st_mode
        && before.st_size == after.st_size
        && before.st_mtime == after.st_mtime
        && before.st_mtime_nsec == after.st_mtime_nsec
        && before.st_ctime == after.st_ctime
        && before.st_ctime_nsec == after.st_ctime_nsec
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::fs::{Mode, OFlags};

    #[test]
    fn stable_read_returns_bytes_and_metadata_from_one_validated_version() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact.txt");
        std::fs::write(&path, b"stable").unwrap();
        let descriptor = rustix::fs::open(
            &path,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();

        let stable = read_regular_fd(&descriptor, 1024, || {}).unwrap();
        assert_eq!(stable.bytes, b"stable");
        assert_eq!(stable.stable_stat.size, stable.bytes.len() as u64);
        assert_ne!(stable.stable_stat.inode, 0);
    }
}

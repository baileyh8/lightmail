//! One process per mail data directory. `MailEngine::new` recovers interrupted
//! submissions when it opens a database, so a second process opening a directory
//! that is still in use would mark live sends as unknown and revert queued mail.
use std::{
    fs::{File, OpenOptions, TryLockError},
    io,
    path::Path,
    time::{Duration, Instant},
};

/// Held for the life of the process; the operating system releases it on exit.
pub struct DirectoryLock {
    _file: File,
}

impl DirectoryLock {
    pub fn acquire(directory: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        // Keep the lock file from being deleted and recreated while it is held,
        // which would let another process lock a different file.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_SHARE_READ: u32 = 0x1;
            const FILE_SHARE_WRITE: u32 = 0x2;
            options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
        }
        let file = options.open(directory.join(".lightmail-instance.lock"))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "此邮箱数据目录已在另一个轻邮窗口中打开",
            )),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }

    /// Switching between demo and real mail starts the next process just before
    /// the previous one exits, so a holder that is about to leave gets a moment.
    pub fn acquire_within(directory: &Path, patience: Duration) -> io::Result<Self> {
        let deadline = Instant::now() + patience;
        loop {
            match Self::acquire(directory) {
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(100));
                }
                result => return result,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_holder_is_refused_until_the_first_releases() {
        let directory = tempfile::tempdir().unwrap();
        let first = DirectoryLock::acquire(directory.path()).unwrap();
        let second = DirectoryLock::acquire(directory.path());
        assert_eq!(second.err().unwrap().kind(), io::ErrorKind::WouldBlock);
        drop(first);
        assert!(DirectoryLock::acquire(directory.path()).is_ok());
    }

    #[test]
    fn a_short_wait_covers_a_holder_that_is_exiting() {
        let directory = tempfile::tempdir().unwrap();
        let first = DirectoryLock::acquire(directory.path()).unwrap();
        let path = directory.path().to_owned();
        let waiter = std::thread::spawn(move || {
            DirectoryLock::acquire_within(&path, Duration::from_secs(3)).is_ok()
        });
        std::thread::sleep(Duration::from_millis(300));
        drop(first);
        assert!(waiter.join().unwrap());
        let _held = DirectoryLock::acquire(directory.path()).unwrap();
        let started = Instant::now();
        assert!(
            DirectoryLock::acquire_within(directory.path(), Duration::from_millis(300)).is_err()
        );
        assert!(started.elapsed() >= Duration::from_millis(300));
    }

    #[test]
    fn separate_directories_do_not_block_each_other() {
        let root = tempfile::tempdir().unwrap();
        let _mail = DirectoryLock::acquire(&root.path().join("Mail")).unwrap();
        let _preview = DirectoryLock::acquire(&root.path().join("Preview")).unwrap();
    }
}

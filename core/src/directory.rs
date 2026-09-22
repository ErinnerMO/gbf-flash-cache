use same_file::Handle;
use std::{
    io,
    path::{Path, PathBuf},
};

// Hold the original directory for relative I/O, not just pathname comparisons.
// cap-std uses directory-relative operations on Unix and denies directory
// rename/delete while its handle is open on Windows.
pub(crate) struct Identity {
    path: PathBuf,
    directory: Handle,
    dir: cap_std::fs::Dir,
    lock: Option<Handle>,
}
impl Identity {
    pub fn open(path: &Path) -> io::Result<Self> {
        let lock_path = path.join(".gbf-flash-cache.lock");
        let dir = cap_std::fs::Dir::open_ambient_dir(path, cap_std::ambient_authority())?;
        let directory = Handle::from_file(dir.try_clone()?.into_std_file())?;
        Ok(Self {
            path: path.to_owned(),
            directory,
            dir,
            lock: if lock_path.exists() {
                Some(handle(lock_path)?)
            } else {
                None
            },
        })
    }
    pub fn snapshot(&self) -> io::Result<DirectoryHandle<'_>> {
        self.check()?;
        Ok(DirectoryHandle { identity: self })
    }
    pub fn lock_directory(&self) -> io::Result<Option<std::fs::File>> {
        // Lock the directory inode, so replacing a filename cannot transfer ownership.
        #[cfg(unix)]
        {
            let file = self.dir.open(".")?.into_std();
            file.try_lock().map_err(io::Error::other)?;
            Ok(Some(file))
        }
        #[cfg(not(unix))]
        Ok(None)
    }
    pub fn matches_lock(&self, file: &std::fs::File) -> io::Result<bool> {
        Ok(self.lock.as_ref() == Some(&Handle::from_file(file.try_clone()?)?))
    }
    pub fn check(&self) -> io::Result<()> {
        if self.path.is_symlink() || handle(&self.path)? != self.directory {
            return Err(io::Error::other("目录已被替换，请停止服务后重试"));
        }
        if let Some(lock) = &self.lock {
            let path = self.path.join(".gbf-flash-cache.lock");
            if path.is_symlink() || handle(path)? != *lock {
                return Err(io::Error::other("目录锁已被替换，请停止服务后重试"));
            }
        }
        Ok(())
    }
}

// Recheck ownership for every file operation, including after a long copy.
pub struct DirectoryHandle<'a> {
    identity: &'a Identity,
}
impl DirectoryHandle<'_> {
    pub fn check(&self) -> io::Result<()> {
        self.identity.check()
    }
    pub fn entries(&self) -> io::Result<cap_std::fs::ReadDir> {
        self.check()?;
        self.identity.dir.entries()
    }
    pub fn open(&self, name: impl AsRef<Path>) -> io::Result<cap_std::fs::File> {
        self.check()?;
        let file = self.identity.dir.open(name)?;
        self.check()?;
        Ok(file)
    }
    pub fn create_new(&self, name: impl AsRef<Path>) -> io::Result<cap_std::fs::File> {
        self.check()?;
        self.identity.dir.open_with(
            name,
            cap_std::fs::OpenOptions::new().write(true).create_new(true),
        )
    }
    pub(crate) fn copy_new(
        &self,
        name: &std::ffi::OsStr,
        input: &mut impl io::Read,
        validate_source: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<()> {
        // Migration is serialized while the directory is exclusively owned.
        // A killed process may leave this staging file; retry safely replaces it.
        const STAGING: &str = ".gbf-flash-cache-migration.tmp";
        if let Err(error) = self.remove_file(STAGING) {
            if error.kind() != io::ErrorKind::NotFound {
                return Err(error);
            }
        }
        let result = (|| {
            let mut output = self.create_new(STAGING)?;
            io::copy(input, &mut output)?;
            output.sync_all()?;
            validate_source()?;
            self.check()?;
            #[cfg(not(windows))]
            self.identity
                .dir
                .hard_link(STAGING, &self.identity.dir, name)?;
            #[cfg(windows)]
            {
                // Windows directory and lock handles prohibit rename/delete.
                // Use no-replace rename, which also supports volumes without hard links.
                let path = tempfile::TempPath::try_from_path(self.identity.path.join(STAGING))?;
                tempfile::NamedTempFile::from_parts(output.into_std(), path)
                    .persist_noclobber(self.identity.path.join(name))
                    .map_err(|e| e.error)?;
            }
            Ok(())
        })();
        let _ = self.remove_file(STAGING);
        result
    }
    pub fn remove_file(&self, name: impl AsRef<Path>) -> io::Result<()> {
        self.check()?;
        self.identity.dir.remove_file(name)?;
        self.check()
    }
}

fn handle(path: impl AsRef<Path>) -> io::Result<Handle> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_BACKUP_SEMANTICS opens directories; sharing read/write/delete
        // preserves normal rename behavior while we keep an identity handle alive.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(0x02000000)
            .share_mode(0x00000007)
            .open(path)?;
        Handle::from_file(file)
    }
    #[cfg(not(windows))]
    Handle::from_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_copy_child() {
        let Some(root) = std::env::var_os("GFC_MIGRATION_TEST_DIR") else {
            return;
        };
        let root = PathBuf::from(root);
        struct PausedRead {
            first: bool,
            signal: PathBuf,
        }
        impl io::Read for PausedRead {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if self.first {
                    self.first = false;
                    buffer.fill(42);
                    return Ok(buffer.len());
                }
                std::fs::write(&self.signal, b"paused")?;
                loop {
                    std::thread::park();
                }
            }
        }
        let identity = Identity::open(&root).unwrap();
        identity
            .snapshot()
            .unwrap()
            .copy_new(
                std::ffi::OsStr::new("item.gfc"),
                &mut PausedRead {
                    first: true,
                    signal: root.parent().unwrap().join("paused"),
                },
                || Ok(()),
            )
            .unwrap();
        panic!("child must be killed during copy");
    }

    #[test]
    fn killed_copy_leaves_no_final_file_and_service_can_retry() {
        use crate::service::{Fields, Service};
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("gbf-flash-cache-cache");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join(".gbf-flash-cache"), b"cache").unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "directory::tests::migration_copy_child",
                "--nocapture",
            ])
            .env("GFC_MIGRATION_TEST_DIR", &target)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !temp.path().join("paused").exists() {
            if child.try_wait().unwrap().is_some() || std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child did not pause while copying");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!target.join("item.gfc").exists());
        assert!(
            std::fs::metadata(target.join(".gbf-flash-cache-migration.tmp"))
                .unwrap()
                .len()
                > 0
        );
        let home = temp.path().join("home");
        let mut service = Service::open(home.clone()).unwrap();
        let original = vec![17_u8; 32768];
        std::fs::write(home.join("cache/item.gfc"), &original).unwrap();
        service
            .change_directory(
                &Fields::from([
                    ("kind".into(), "cache".into()),
                    ("migrate".into(), "true".into()),
                    ("path".into(), temp.path().to_string_lossy().into()),
                ]),
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(std::fs::read(target.join("item.gfc")).unwrap(), original);
        assert!(!target.join(".gbf-flash-cache-migration.tmp").exists());
    }

    #[test]
    fn migration_publish_preserves_existing_file_and_source_check_failure() {
        let temp = tempfile::tempdir().unwrap();
        let identity = Identity::open(temp.path()).unwrap();
        let directory = identity.snapshot().unwrap();
        let name = std::ffi::OsStr::new("item.gfc");
        std::fs::write(temp.path().join(name), b"existing").unwrap();
        let error = directory
            .copy_new(name, &mut &b"new"[..], || Ok(()))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(temp.path().join(name)).unwrap(), b"existing");
        directory.remove_file(name).unwrap();
        assert!(directory
            .copy_new(name, &mut &b"new"[..], || Err(io::Error::other(
                "source changed"
            )))
            .is_err());
        assert!(!temp.path().join(name).exists());
        assert!(!temp.path().join(".gbf-flash-cache-migration.tmp").exists());
    }

    #[test]
    fn opened_directory_does_not_follow_replacement_during_io() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache");
        let detached = temp.path().join("detached");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("same.gfc"), b"original").unwrap();
        let identity = Identity::open(&root).unwrap();
        let snapshot = identity.snapshot().unwrap();
        let entries: Vec<_> = snapshot
            .entries()
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        if let Err(error) = std::fs::rename(&root, &detached) {
            // Windows holds a non-delete-sharing directory handle instead.
            assert!(cfg!(windows), "{error}");
            assert_eq!(identity.dir.read("same.gfc").unwrap(), b"original");
            return;
        }
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("same.gfc"), b"other instance").unwrap();
        let lock = std::fs::File::create(root.join(".gbf-flash-cache.lock")).unwrap();
        lock.try_lock().unwrap();
        assert!(identity.snapshot().is_err());
        // Already-started export and cleanup stay on the original object.
        assert_eq!(identity.dir.read("same.gfc").unwrap(), b"original");
        for name in entries {
            assert!(snapshot.remove_file(name).is_err());
        }
        assert!(detached.join("same.gfc").exists());
        assert_eq!(
            std::fs::read(root.join("same.gfc")).unwrap(),
            b"other instance"
        );
    }
}

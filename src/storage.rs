//! Private workspace writes. Use a new, unpredictable temporary file beside
//! the destination so crashes/symlinks cannot corrupt another file or expose
//! partially written SQL. Unix permissions do not replace Windows ACL policy.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

fn prepare_parent(path: &Path) -> io::Result<()> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    // Never chmod an existing user-selected directory (it might be shared).
    builder.create(parent)
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    options
}

pub fn open_private_append(path: &Path) -> io::Result<File> {
    prepare_parent(path)?;
    let file = private_options().create(true).append(true).open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("workspace destination is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

struct TempFile(PathBuf);
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn write_private_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    prepare_parent(path)?;
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let temp_path = parent.join(format!(".trunq-{:016x}.tmp", rand::random::<u64>()));
    let mut file = private_options().create_new(true).open(&temp_path)?;
    let tmp = TempFile(temp_path);
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp.0, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_replacement_and_private_permissions() {
        let dir = TempFile(std::env::temp_dir().join(format!("trunq-storage-{}", rand::random::<u64>())));
        let path = dir.0.join("config.json");
        write_private_atomic(&path, b"old").unwrap();
        write_private_atomic(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(&dir.0).unwrap().permissions().mode() & 0o777, 0o700);
        }
        std::fs::remove_dir_all(&dir.0).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn shared_directory_symlinks_cannot_redirect_workspace_writes() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = std::env::temp_dir().join(format!("trunq-storage-links-{}", rand::random::<u64>()));
        std::fs::create_dir(&dir).unwrap();
        let target = dir.join("unrelated.txt");
        std::fs::write(&target, b"keep").unwrap();
        let link = dir.join("history.jsonl");
        symlink(&target, &link).unwrap();
        assert!(open_private_append(&link).is_err());
        write_private_atomic(&link, b"new workspace").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        assert_eq!(std::fs::read(&link).unwrap(), b"new workspace");
        assert_eq!(std::fs::metadata(&link).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

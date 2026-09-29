//! Small JSON files only the user can read, replaced atomically.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

/// Largest store file read. Anything bigger is not ours.
pub(crate) const MAX_FILE_BYTES: u64 = 4 << 20;

/// Writes `bytes` to `file` through a temporary file and a rename, mode 0600, in
/// a directory made 0700 if needed.
pub(crate) fn write_private(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    let temp = file.with_extension("json.tmp");
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp)?;
    out.write_all(bytes)?;
    out.sync_all()?;
    fs::rename(&temp, file)
}

/// Reads a store file: `None` if it does not exist. A file that cannot be read,
/// is too large, or does not parse is moved aside to `<name>.corrupt` and the
/// returned warning says so.
pub(crate) fn read_json<T: serde::de::DeserializeOwned>(
    file: &Path,
) -> (Option<T>, Option<String>) {
    let parsed = match fs::metadata(file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (None, None),
        Err(e) => Err(e.to_string()),
        Ok(m) if m.len() > MAX_FILE_BYTES => Err(format!("{} bytes is too large", m.len())),
        Ok(_) => fs::read(file)
            .map_err(|e| e.to_string())
            .and_then(|bytes| serde_json::from_slice::<T>(&bytes).map_err(|e| e.to_string())),
    };
    match parsed {
        Ok(value) => (Some(value), None),
        Err(reason) => {
            let aside = file.with_extension("json.corrupt");
            let _ = fs::rename(file, &aside);
            (
                None,
                Some(format!(
                    "{} was damaged ({reason}); starting empty. The old file is at {}",
                    file.display(),
                    aside.display()
                )),
            )
        }
    }
}

//! Small filesystem helpers shared across the core.

use std::io::Write;
use std::path::Path;

/// Writes `bytes` to `path` via a temporary file in the same directory and
/// an atomic rename, so the destination is never observed half-written.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a file path"))?;

    // Unique-enough temp name: same directory (required for an atomic
    // rename across the board) plus pid to survive concurrent editors.
    let mut temp = dir.map(Path::to_path_buf).unwrap_or_default();
    temp.push(format!(
        ".{}.textchum-{}.tmp",
        file_name.to_string_lossy(),
        std::process::id()
    ));

    let result = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

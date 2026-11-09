#![deny(missing_docs)]

//! Small platform boundary for durable Windows file replacement.

use std::io;
use std::path::Path;

/// Renames a file and requests write-through completion on Windows.
///
/// On non-Windows platforms this delegates to `std::fs::rename`. Windows
/// uses `MoveFileExW` with `MOVEFILE_WRITE_THROUGH`; replacing an existing
/// destination follows the semantics required by the state-store transaction.
pub fn rename_write_through(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        windows::rename_write_through(from, to)
    }

    #[cfg(not(windows))]
    {
        std::fs::rename(from, to)
    }
}

#[cfg(windows)]
mod windows {
    use std::io;
    use std::iter;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "Kernel32")]
    extern "system" {
        #[link_name = "MoveFileExW"]
        fn move_file_ex_w(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    fn encode_path(path: &Path) -> io::Result<Vec<u16>> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        if wide.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path contains an embedded NUL",
            ));
        }
        Ok(wide.into_iter().chain(iter::once(0)).collect())
    }

    pub(super) fn rename_write_through(from: &Path, to: &Path) -> io::Result<()> {
        let from = encode_path(from)?;
        let to = encode_path(to)?;
        // SAFETY: both paths are valid NUL-terminated UTF-16 buffers that live
        // for the duration of the call; the flags are documented MoveFileExW
        // flags, and the API does not retain either pointer after returning.
        let result = unsafe {
            move_file_ex_w(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if result == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn rename_replaces_destination_and_moves_source() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "filesystem-write-through-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.tmp");
        let destination = directory.join("destination");
        fs::write(&source, b"new state").unwrap();
        fs::write(&destination, b"old state").unwrap();

        rename_write_through(&source, &destination).unwrap();

        assert!(!source.exists());
        assert_eq!(fs::read(destination).unwrap(), b"new state");
        fs::remove_dir_all(directory).unwrap();
    }
}

//! Small safe wrapper around the Windows process AppUserModelID API.

/// Assigns the same explicit Windows taskbar identity to related PeaceEmu processes.
///
/// Call this before creating any UI. Windows uses the identifier to group windows from
/// multiple processes under one taskbar application button.
pub fn set_current_process_app_user_model_id(app_id: &str) -> Result<(), String> {
    let utf16: Vec<u16> = app_id.encode_utf16().collect();
    if utf16.is_empty()
        || utf16.len() > 128
        || app_id.contains('\0')
        || app_id.chars().any(char::is_control)
    {
        return Err(
            "AppUserModelID must contain 1 to 128 non-control UTF-16 characters.".to_owned(),
        );
    }

    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        extern "system" {
            fn SetCurrentProcessExplicitAppUserModelID(app_id: *const u16) -> i32;
        }

        let mut wide = utf16;
        wide.push(0);
        // SAFETY: wide is NUL-terminated UTF-16 and remains alive for the duration
        // of the synchronous Shell32 call.
        let result = unsafe { SetCurrentProcessExplicitAppUserModelID(wide.as_ptr()) };
        if result >= 0 {
            Ok(())
        } else {
            Err(format!(
                "SetCurrentProcessExplicitAppUserModelID failed (HRESULT 0x{:08X}).",
                result as u32
            ))
        }
    }

    #[cfg(not(windows))]
    {
        let _ = utf16;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::set_current_process_app_user_model_id;

    #[test]
    fn accepts_the_shared_peaceemu_identity() {
        assert!(set_current_process_app_user_model_id("Likounsee.PeaceEmu").is_ok());
    }

    #[test]
    fn rejects_empty_or_invalid_identities() {
        assert!(set_current_process_app_user_model_id("").is_err());
        assert!(set_current_process_app_user_model_id("PeaceEmu\0NES").is_err());
        assert!(set_current_process_app_user_model_id(&"x".repeat(129)).is_err());
    }
}

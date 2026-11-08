//! Shell dialogs run on their own short-lived STA, independently of Slint redraws.
use std::path::PathBuf;
use windows::{
    Win32::{
        Foundation::{ERROR_CANCELLED, HWND},
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
        },
        UI::{
            Shell::{
                Common::COMDLG_FILTERSPEC, FOS_DONTADDTORECENT, FOS_FILEMUSTEXIST,
                FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_PATHMUSTEXIST, FileOpenDialog,
                IFileOpenDialog, SIGDN_FILESYSPATH,
            },
            WindowsAndMessaging::IsWindow,
        },
    },
    core::{HRESULT, w},
};

struct Apartment;
impl Apartment {
    fn initialize() -> Result<Self, String> {
        // SAFETY: the caller uses a newly spawned picker thread. Every successful
        // initialization (including S_FALSE) is balanced on the same thread.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)
                .ok()
                .map_err(|e| format!("Could not initialize the file picker: {e}"))?;
        }
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: this guard exists only after successful initialization.
        unsafe { CoUninitialize() }
    }
}

/// Call on a disposable worker thread, never in a Slint event callback. `owner`
/// identifies the existing integration manager; no COM interface crosses threads.
pub fn choose_plugin_package(owner: isize) -> Result<Option<PathBuf>, String> {
    let _apartment = Apartment::initialize()?;
    let owner = HWND(owner as *mut _);
    // SAFETY: owner is only used as an opaque HWND; validate it before use. The
    // Shell owns the dialog and pumps this STA while Show is active. All COM
    // objects are released before the apartment guard. UI values return by value.
    unsafe {
        if !IsWindow(Some(owner)).as_bool() {
            return Err("The integration window closed before the file picker opened.".into());
        }
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("Could not create the file picker: {e}"))?;
        dialog
            .SetTitle(w!("Select a plugin package"))
            .map_err(|e| e.to_string())?;
        dialog
            .SetOkButtonLabel(w!("Select"))
            .map_err(|e| e.to_string())?;
        dialog
            .SetFileTypes(&[COMDLG_FILTERSPEC {
                pszName: w!("Still Island plugin packages (*.zip)"),
                pszSpec: w!("*.zip"),
            }])
            .map_err(|e| e.to_string())?;
        let options = dialog.GetOptions().map_err(|e| e.to_string())?;
        dialog
            .SetOptions(
                options
                    | FOS_FORCEFILESYSTEM
                    | FOS_FILEMUSTEXIST
                    | FOS_PATHMUSTEXIST
                    | FOS_NOCHANGEDIR
                    | FOS_DONTADDTORECENT,
            )
            .map_err(|e| e.to_string())?;
        if let Err(error) = dialog.Show(Some(owner)) {
            if cancelled(error.code()) {
                return Ok(None);
            }
            return Err(format!("Could not open the file picker: {error}"));
        }
        let item = dialog.GetResult().map_err(|e| e.to_string())?;
        let name = item
            .GetDisplayName(SIGDN_FILESYSPATH)
            .map_err(|e| e.to_string())?;
        // Copy UTF-16 directly: Windows paths need not be valid Unicode strings.
        let path = {
            use std::os::windows::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_wide(name.as_wide()))
        };
        CoTaskMemFree(Some(name.0.cast()));
        Ok(Some(path))
    }
}

fn cancelled(code: HRESULT) -> bool {
    code == HRESULT::from_win32(ERROR_CANCELLED.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_explicit_cancel_is_a_silent_dismissal() {
        assert!(cancelled(HRESULT::from_win32(ERROR_CANCELLED.0)));
        assert!(!cancelled(HRESULT(0x80070005u32 as i32)));
        assert!(!cancelled(HRESULT(0x80010106u32 as i32)));
    }
}

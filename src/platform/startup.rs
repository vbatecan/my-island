//! Per-user startup registration. Reading preferences never registers anything.
//!
//! Packaged applications use the declared Windows StartupTask; unpackaged copies
//! own one HKCU Run value. Windows' disabled-by-user/policy decisions are respected.
use std::{marker::PhantomData, os::windows::ffi::OsStrExt, path::Path, rc::Rc};
use windows::{
    ApplicationModel::{StartupTask, StartupTaskState},
    Win32::{
        Foundation::{
            APPMODEL_ERROR_NO_PACKAGE, ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER,
            ERROR_PATH_NOT_FOUND, ERROR_SUCCESS, RPC_E_CHANGED_MODE,
        },
        Storage::Packaging::Appx::GetCurrentPackageFullName,
        System::{
            Registry::*,
            WinRT::{RO_INIT_MULTITHREADED, RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize},
        },
    },
    core::{HSTRING, PCWSTR},
};

const TASK_ID: &str = "StillIslandStartup";
const VALUE_NAME: &str = "StillIsland";
const RUN_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVAL_PATH: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const MAX_REGISTRY_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupStatus {
    pub enabled: bool,
    pub available: bool,
    pub message: String,
}
impl StartupStatus {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            enabled: false,
            available: false,
            message: message.into(),
        }
    }
}

/// Call on the GUI thread. WinRT awaits yield to its event loop; no worker or
/// polling timer is created. Snapshot never changes startup registration.
pub async fn snapshot() -> StartupStatus {
    let result = match packaged() {
        Ok(true) => packaged_snapshot().await,
        Ok(false) => portable_snapshot(),
        Err(error) => Err(error),
    };
    result.unwrap_or_else(StartupStatus::unavailable)
}

/// Explicit settings action only. The value is read back before reporting success.
/// No settings-file boolean is used as evidence of native registration.
pub async fn set_enabled(enabled: bool) -> Result<StartupStatus, String> {
    if packaged()? {
        let _runtime = Runtime::initialize()?;
        let task = StartupTask::GetAsync(&HSTRING::from(TASK_ID))
            .map_err(winrt_error)?
            .await
            .map_err(winrt_error)?;
        let current = task_status(task.State().map_err(winrt_error)?);
        if current.enabled == enabled {
            return Ok(current);
        }
        if !current.available {
            return Err(current.message);
        }
        if enabled {
            let state = task
                .RequestEnableAsync()
                .map_err(winrt_error)?
                .await
                .map_err(winrt_error)?;
            Ok(task_status(state))
        } else {
            task.Disable().map_err(winrt_error)?;
            Ok(task_status(task.State().map_err(winrt_error)?))
        }
    } else {
        let command = quoted_command(&std::env::current_exe().map_err(|e| e.to_string())?)?;
        registration().set_enabled(&command, enabled)
    }
}

fn packaged() -> Result<bool, String> {
    let mut length = 0;
    // SAFETY: size-only query uses a live length output and no data buffer.
    let result = unsafe { GetCurrentPackageFullName(&mut length, None) };
    if result == APPMODEL_ERROR_NO_PACKAGE {
        Ok(false)
    } else if result == ERROR_SUCCESS || result == ERROR_INSUFFICIENT_BUFFER {
        Ok(true)
    } else {
        Err(format!(
            "Cannot determine startup registration type (Win32 {})",
            result.0
        ))
    }
}

async fn packaged_snapshot() -> Result<StartupStatus, String> {
    let _runtime = Runtime::initialize()?;
    let task = StartupTask::GetAsync(&HSTRING::from(TASK_ID))
        .map_err(winrt_error)?
        .await
        .map_err(winrt_error)?;
    Ok(task_status(task.State().map_err(winrt_error)?))
}

fn winrt_error(error: windows::core::Error) -> String {
    format!("Windows startup task is unavailable: {error}")
}

fn task_status(state: StartupTaskState) -> StartupStatus {
    match state {
        StartupTaskState::Enabled => StartupStatus {
            enabled: true,
            available: true,
            message: "Starts Still Island when you sign in.".into(),
        },
        StartupTaskState::Disabled => StartupStatus {
            enabled: false,
            available: true,
            message: "Start Still Island automatically when you sign in.".into(),
        },
        StartupTaskState::DisabledByUser => StartupStatus::unavailable(
            "Windows disabled startup for Still Island. Re-enable it in Windows Settings → Apps → Startup, then reopen these settings.",
        ),
        StartupTaskState::DisabledByPolicy => StartupStatus::unavailable(
            "Startup is disabled by your administrator or unsupported on this device.",
        ),
        StartupTaskState::EnabledByPolicy => StartupStatus {
            enabled: true,
            available: false,
            message: "Startup is enabled by your administrator.".into(),
        },
        _ => StartupStatus::unavailable("Windows returned an unknown startup-task state."),
    }
}

/// WinRT requests must remain on the initiating GUI thread across `.await`.
/// The Rc marker deliberately makes this guard (and its future) non-Send.
struct Runtime(PhantomData<Rc<()>>);
impl Runtime {
    fn initialize() -> Result<Self, String> {
        // SAFETY: initialize this calling thread and balance every successful
        // S_OK/S_FALSE with RoUninitialize. A preexisting apartment is retained.
        let result = unsafe { RoInitialize(RO_INIT_SINGLETHREADED) };
        if result
            .as_ref()
            .is_err_and(|e| e.code() == RPC_E_CHANGED_MODE)
        {
            unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(winrt_error)?;
        } else {
            result.map_err(winrt_error)?;
        }
        Ok(Self(PhantomData))
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        // SAFETY: initialize succeeded on this same GUI-thread future.
        unsafe { RoUninitialize() };
    }
}

fn portable_snapshot() -> Result<StartupStatus, String> {
    let command = quoted_command(&std::env::current_exe().map_err(|e| e.to_string())?)?;
    registration().snapshot(&command)
}
fn registration() -> Registration<'static> {
    Registration {
        run_path: RUN_PATH,
        approval_path: APPROVAL_PATH,
        value_name: VALUE_NAME,
    }
}

fn quoted_command(executable: &Path) -> Result<Vec<u16>, String> {
    let path: Vec<_> = executable.as_os_str().encode_wide().collect();
    if !executable.is_absolute()
        || path.is_empty()
        || path.iter().any(|c| *c == 0 || *c == b'"' as u16)
    {
        return Err("The executable path cannot be registered for Windows startup.".into());
    }
    // Microsoft documents a 260-character command limit for Run values.
    if path.len() + 2 > 260 {
        return Err("This executable path is too long for Windows startup. Move the portable app to a shorter path or use the installer.".into());
    }
    Ok(std::iter::once(b'"' as u16)
        .chain(path)
        .chain([b'"' as u16])
        .collect())
}

struct Registration<'a> {
    run_path: &'a str,
    approval_path: &'a str,
    value_name: &'a str,
}
impl Registration<'_> {
    fn snapshot(&self, command: &[u16]) -> Result<StartupStatus, String> {
        let current = read_string(self.run_path, self.value_name)?;
        if current
            .as_ref()
            .is_none_or(|value| !same_command(value, command))
        {
            return Ok(StartupStatus {
                enabled: false,
                available: true,
                message: if current.is_some() {
                    "Another copy is registered for startup. Enabling replaces that registration with this copy.".into()
                } else {
                    "Starts this copy when you sign in. Keep its executable in the same location."
                        .into()
                },
            });
        }
        // StartupApproved is an OS implementation detail, used read-only. We
        // never change these flags to bypass a user's choice in Startup Apps.
        match read_value(self.approval_path, self.value_name, RRF_RT_REG_BINARY)? {
            None => Ok(StartupStatus {
                enabled: true,
                available: true,
                message: "Registered to start when you sign in. Windows may defer startup.".into(),
            }),
            Some(data) if matches!(data.first(), Some(2 | 6)) => Ok(StartupStatus {
                enabled: true,
                available: true,
                message: "Starts this copy when you sign in.".into(),
            }),
            Some(data) if matches!(data.first(), Some(3 | 7)) => Ok(StartupStatus::unavailable(
                "Windows disabled startup for Still Island. Re-enable it in Windows Settings → Apps → Startup, then reopen these settings.",
            )),
            Some(_) => Ok(StartupStatus::unavailable(
                "This copy is registered, but Windows' startup approval state is unknown. Check Windows Settings → Apps → Startup.",
            )),
        }
    }
    fn set_enabled(&self, command: &[u16], enabled: bool) -> Result<StartupStatus, String> {
        if enabled {
            let current = self.snapshot(command)?;
            if !current.available {
                return Err(current.message);
            }
            let bytes: Vec<u8> = command
                .iter()
                .copied()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect();
            write_value(self.run_path, self.value_name, REG_SZ, &bytes)?;
        } else if read_string(self.run_path, self.value_name)?
            .as_ref()
            .is_some_and(|value| same_command(value, command))
        {
            delete_value(self.run_path, self.value_name)?;
        }
        self.snapshot(command)
    }
}

fn same_command(left: &[u16], right: &[u16]) -> bool {
    let fold = |c: &u16| if (65..=90).contains(c) { *c + 32 } else { *c };
    left.len() == right.len() && left.iter().map(fold).eq(right.iter().map(fold))
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

fn read_string(path: &str, name: &str) -> Result<Option<Vec<u16>>, String> {
    let Some(bytes) = read_value(path, name, RRF_RT_REG_SZ)? else {
        return Ok(None);
    };
    if bytes.len() % 2 != 0 {
        return Err("The startup registry value is malformed.".into());
    }
    let mut words: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    while words.last() == Some(&0) {
        words.pop();
    }
    Ok(Some(words))
}

fn read_value(path: &str, name: &str, kind: REG_ROUTINE_FLAGS) -> Result<Option<Vec<u8>>, String> {
    let (path, name) = (wide(path), wide(name));
    let mut count = 0;
    // SAFETY: registry queries are synchronous, strings remain NUL terminated,
    // and buffers match the byte length supplied to Windows. No handle is owned.
    unsafe {
        let status = RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            PCWSTR(name.as_ptr()),
            kind,
            None,
            None,
            Some(&mut count),
        );
        if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
            return Ok(None);
        }
        status
            .ok()
            .map_err(|e| format!("Cannot read Windows startup settings: {e}"))?;
        if count as usize > MAX_REGISTRY_BYTES {
            return Err("The startup registry value exceeds its size limit.".into());
        }
        let mut bytes = vec![0; count as usize];
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            PCWSTR(name.as_ptr()),
            kind,
            None,
            Some(bytes.as_mut_ptr().cast()),
            Some(&mut count),
        )
        .ok()
        .map_err(|e| format!("Cannot read Windows startup settings: {e}"))?;
        bytes.truncate(count as usize);
        Ok(Some(bytes))
    }
}

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: only successfully opened/created keys enter this RAII owner.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}
fn write_value(path: &str, name: &str, kind: REG_VALUE_TYPE, bytes: &[u8]) -> Result<(), String> {
    let (path, name) = (wide(path), wide(name));
    let mut key = HKEY::default();
    // SAFETY: request only per-user value-writing rights, retain all backing
    // buffers throughout synchronous calls, and close the returned handle once.
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
        .ok()
        .map_err(|e| format!("Cannot register Windows startup: {e}"))?;
        let key = Key(key);
        RegSetValueExW(key.0, PCWSTR(name.as_ptr()), None, kind, Some(bytes))
            .ok()
            .map_err(|e| format!("Cannot register Windows startup: {e}"))
    }
}
fn delete_value(path: &str, name: &str) -> Result<(), String> {
    let (path, name) = (wide(path), wide(name));
    let mut key = HKEY::default();
    // SAFETY: remove only the app-owned value after its command was checked;
    // never remove the Run key or another application's value.
    unsafe {
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            KEY_SET_VALUE,
            &mut key,
        );
        if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
            return Ok(());
        }
        status
            .ok()
            .map_err(|e| format!("Cannot update Windows startup: {e}"))?;
        let key = Key(key);
        let status = RegDeleteValueW(key.0, PCWSTR(name.as_ptr()));
        if status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            status
                .ok()
                .map_err(|e| format!("Cannot update Windows startup: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(String);
    impl Fixture {
        fn new() -> Self {
            Self(format!(
                r"Software\StillIsland\Tests\Startup-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert!(self.0.starts_with(r"Software\StillIsland\Tests\Startup-"));
            // SAFETY: unique test-only subtree; never the real Run/approval key.
            let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(wide(&self.0).as_ptr())) };
        }
    }
    #[test]
    fn quotes_unicode_paths_and_rejects_unsupported_run_commands() {
        let command = quoted_command(Path::new(r"C:\Apps\Quiet 雨\still-island.exe")).unwrap();
        assert_eq!(
            String::from_utf16(&command).unwrap(),
            "\"C:\\Apps\\Quiet 雨\\still-island.exe\""
        );
        assert!(quoted_command(Path::new("relative.exe")).is_err());
        assert!(quoted_command(Path::new("C:\\bad\"path.exe")).is_err());
        assert!(quoted_command(Path::new(&format!("C:\\{}.exe", "a".repeat(260)))).is_err());
    }
    #[test]
    fn startup_task_states_respect_windows_user_and_policy_decisions() {
        assert!(task_status(StartupTaskState::Enabled).enabled);
        assert!(task_status(StartupTaskState::Disabled).available);
        for state in [
            StartupTaskState::DisabledByUser,
            StartupTaskState::DisabledByPolicy,
        ] {
            let status = task_status(state);
            assert!(!status.enabled && !status.available);
        }
        let policy = task_status(StartupTaskState::EnabledByPolicy);
        assert!(policy.enabled && !policy.available);
    }
    #[test]
    fn reading_native_status_never_changes_user_registration() {
        let before = read_value(RUN_PATH, VALUE_NAME, RRF_RT_ANY).unwrap();
        let status = portable_snapshot().unwrap();
        assert!(!status.message.is_empty());
        assert_eq!(
            read_value(RUN_PATH, VALUE_NAME, RRF_RT_ANY).unwrap(),
            before
        );
    }
    #[test]
    fn isolated_registry_round_trip_preserves_other_copies_and_windows_approval() {
        let fixture = Fixture::new();
        let approval = format!(r"{}\Approval", fixture.0);
        let registration = Registration {
            run_path: &fixture.0,
            approval_path: &approval,
            value_name: "IsolatedStartupFixture",
        };
        let command = quoted_command(Path::new(r"C:\Test\still-island.exe")).unwrap();
        let other = quoted_command(Path::new(r"C:\Other\still-island.exe")).unwrap();
        assert!(!registration.snapshot(&command).unwrap().enabled);
        assert!(registration.set_enabled(&command, true).unwrap().enabled);
        assert!(registration.set_enabled(&command, true).unwrap().enabled);
        assert!(!registration.set_enabled(&other, false).unwrap().enabled);
        assert!(registration.snapshot(&command).unwrap().enabled);
        assert!(!registration.set_enabled(&command, false).unwrap().enabled);
        assert!(
            read_string(&fixture.0, registration.value_name)
                .unwrap()
                .is_none()
        );
        registration.set_enabled(&command, true).unwrap();
        write_value(
            &approval,
            registration.value_name,
            REG_BINARY,
            &[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        )
        .unwrap();
        let disabled = registration.snapshot(&command).unwrap();
        assert!(!disabled.enabled && !disabled.available);
        assert!(registration.set_enabled(&command, true).is_err());
        assert_eq!(
            read_value(&approval, registration.value_name, RRF_RT_REG_BINARY)
                .unwrap()
                .unwrap()[0],
            3
        );
        registration.set_enabled(&command, false).unwrap();
    }
}

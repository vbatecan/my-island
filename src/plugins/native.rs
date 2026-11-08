//! Small Windows ownership boundaries for trusted plugin processes and secrets.
use std::{mem::size_of, os::windows::io::AsRawHandle};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree},
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
        System::JobObjects::*,
    },
    core::PCWSTR,
};
pub struct Job(HANDLE);
// SAFETY: Job handles are kernel objects. Closing/terminating is thread safe;
// the handle is owned once, and references never outlive this RAII owner.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}
impl Job {
    pub fn attach(child: &std::process::Child) -> Result<Self, String> {
        // SAFETY: all handles come from successful creation; information uses
        // the exact Win32 size. child remains owned by its supervisor.
        unsafe {
            let job = Self(CreateJobObjectW(None, PCWSTR::null()).map_err(|e| e.to_string())?);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .map_err(|e| e.to_string())?;
            AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle()))
                .map_err(|e| e.to_string())?;
            Ok(job)
        }
    }
    pub fn terminate(&self) {
        // SAFETY: this owner retains a valid job handle.
        let _ = unsafe { TerminateJobObject(self.0, 1) };
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
pub fn protect(bytes: &[u8], decrypt: bool) -> Result<Vec<u8>, String> {
    if bytes.len() > 65536 {
        return Err("Secret configuration too large".into());
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: input/output buffers live for the call. DPAPI allocates output
    // with LocalAlloc; copy it before releasing exactly once with LocalFree.
    unsafe {
        if decrypt {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
        .map_err(|_| "Windows could not protect/unprotect plugin credentials".to_string())?;
        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData as _)));
        Ok(result)
    }
}

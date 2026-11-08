//! Disposable picker rendering probe; only opens the supplied generated folder.
use windows::{
    Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoCreateInstance,
            CoInitializeEx, CoUninitialize,
        },
        UI::Shell::{FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName},
    },
    core::{HSTRING, PCWSTR},
};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    unsafe {
        if args[1] == "mta" {
            println!(
                "MTA initialization: {:?}",
                CoInitializeEx(None, COINIT_MULTITHREADED)
            );
        }
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        println!("STA initialization: {initialized:?}");
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).unwrap();
        dialog
            .SetTitle(&HSTRING::from(format!(
                "Still Island Picker Probe {}",
                args[1]
            )))
            .unwrap();
        let folder: IShellItem =
            SHCreateItemFromParsingName(&HSTRING::from(&args[2]), None).unwrap();
        dialog.SetFolder(&folder).unwrap();
        println!("Show: {:?}", dialog.Show(None));
        drop(dialog);
        if initialized.is_ok() {
            CoUninitialize();
        }
        if args[1] == "mta" {
            CoUninitialize();
        }
        // Distinguish actual Common Controls activation from a build assumption.
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetModuleHandleW(name: PCWSTR) -> isize;
            fn GetModuleFileNameW(module: isize, path: *mut u16, size: u32) -> u32;
        }
        let module = GetModuleHandleW(windows::core::w!("comctl32.dll"));
        if module == 0 {
            println!("Common Controls: not loaded under unqualified module name");
        } else {
            let mut path = [0u16; 2048];
            let count = GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32);
            println!(
                "Common Controls: {}",
                String::from_utf16_lossy(&path[..count as usize])
            );
        }
    }
}

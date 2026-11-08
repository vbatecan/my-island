//! Finite native test. It launches only the separately built owned marker fixture.
#[path = "../../src/platform/launcher.rs"]
mod launcher;
#[path = "../../src/windows_commands.rs"]
mod windows_commands;

use std::{
    os::windows::ffi::OsStrExt,
    path::Path,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, IPersistFile,
        },
        UI::Shell::{IShellLinkW, ShellLink},
    },
    core::{Interface, PCWSTR},
};

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn create_shortcut(directory: &Path, shortcut: &str, marker: &str) -> std::path::PathBuf {
    let target = wide(&directory.join("owned-launch-fixture.exe"));
    let shortcut = directory.join(shortcut);
    let shortcut_name = wide(&shortcut);
    let cwd = wide(directory);
    let arguments: Vec<u16> = format!(
        "\"{}\" \"argument with spaces\"",
        directory.join(marker).display()
    )
    .encode_utf16()
    .chain(Some(0))
    .collect();
    // SAFETY: all strings outlive the synchronous COM calls. This probe creates
    // the shortcut itself in its caller-provided unique target directory.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
        {
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
            link.SetPath(PCWSTR(target.as_ptr())).unwrap();
            link.SetArguments(PCWSTR(arguments.as_ptr())).unwrap();
            link.SetWorkingDirectory(PCWSTR(cwd.as_ptr())).unwrap();
            let persist: IPersistFile = link.cast().unwrap();
            persist.Save(PCWSTR(shortcut_name.as_ptr()), true).unwrap();
        }
        CoUninitialize();
    }
    shortcut
}

fn main() {
    let directory = std::env::args_os()
        .nth(1)
        .expect("unique owned probe directory");
    // Shell links reject the extended-length prefix added by canonicalize;
    // preserve the script's absolute ordinary path for COM string parameters.
    let directory = std::path::PathBuf::from(directory);
    assert!(directory.is_absolute());
    assert!(directory.join("owned-launch-fixture.exe").is_file());
    let marker = directory.join("native-launch-result.txt");
    assert!(!marker.exists(), "refuse an existing fixture marker");
    let shortcut = create_shortcut(
        &directory,
        "Owned native launch.lnk",
        "native-launch-result.txt",
    );
    // A separate, unlaunched target is supplied for the normal GUI controller's
    // launch verification. Its marker must not exist before that test starts.
    create_shortcut(&directory, "Owned GUI launch.lnk", "gui-launch-result.txt");
    launcher::launch(&launcher::LaunchTarget::Shortcut(shortcut))
        .expect("launch own marker shortcut");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    let result = std::fs::read_to_string(marker).expect("own marker result");
    assert!(result.contains("argument=argument with spaces"));
    let actual_cwd = result
        .lines()
        .find_map(|line| line.strip_prefix("cwd="))
        .unwrap();
    assert_eq!(
        std::fs::canonicalize(actual_cwd).unwrap(),
        std::fs::canonicalize(&directory).unwrap()
    );
    println!("PASS owned native shortcut launch preserved arguments and working directory");

    let catalog = launcher::enumerate().expect("native application discovery");
    assert!(!catalog.entries.is_empty());
    let apps: Vec<_> = catalog
        .entries
        .iter()
        .filter(|entry| matches!(entry.target, launcher::LaunchTarget::AppId(_)))
        .collect();
    let shortcuts = catalog
        .entries
        .iter()
        .filter(|entry| matches!(entry.target, launcher::LaunchTarget::Shortcut(_)))
        .count();
    let mut shortcut_icons = 0;
    for entry in catalog
        .entries
        .iter()
        .filter(|entry| matches!(entry.target, launcher::LaunchTarget::Shortcut(_)))
        .take(4)
    {
        for size in [32, 48, 64] {
            let image = launcher::load_icon(&entry.target, size).expect("shortcut native icon");
            assert_eq!(image.rgba.len(), (image.width * image.height * 4) as usize);
            assert!(image.rgba.chunks_exact(4).any(|pixel| pixel[3] != 0));
            shortcut_icons += 1;
        }
    }
    let mut packaged_icons = 0;
    for entry in apps
        .iter()
        .filter(
            |entry| matches!(&entry.target, launcher::LaunchTarget::AppId(id) if id.contains('!')),
        )
        .take(4)
    {
        for size in [32, 48, 64] {
            let image = launcher::load_icon(&entry.target, size).expect("packaged native icon");
            assert_eq!(image.rgba.len(), (image.width * image.height * 4) as usize);
            assert!(image.rgba.chunks_exact(4).any(|pixel| pixel[3] != 0));
            packaged_icons += 1;
        }
    }
    assert!(shortcut_icons > 0, "required a Start Menu shortcut icon");
    assert!(packaged_icons > 0, "required a packaged Windows app icon");
    println!(
        "PASS catalog={} shortcuts={shortcuts} AppsFolder={} warnings={} shortcut_icons={shortcut_icons} packaged_icons={packaged_icons}",
        catalog.entries.len(),
        apps.len(),
        catalog.warnings.len()
    );
}

//! Native application discovery, launch, and size-specific Shell icons.
//!
//! These operations can load Shell extensions and must run on a disposable or
//! sleeping STA worker, never in the Slint event callback. No COM object or GDI
//! handle is retained in the catalog or crosses a thread boundary.
use std::{
    collections::HashSet,
    ffi::OsString,
    marker::PhantomData,
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    rc::Rc,
};
use windows::{
    Win32::{
        Foundation::SIZE,
        Graphics::Gdi::{
            BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS,
            DeleteDC, DeleteObject, GetDIBits, GetObjectW, HBITMAP, HDC,
        },
        System::Com::{
            COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoTaskMemFree,
            CoUninitialize,
        },
        UI::{
            Shell::{
                BHID_EnumItems, FOLDERID_AppsFolder, FOLDERID_CommonStartMenu, FOLDERID_StartMenu,
                IEnumShellItems, IShellItem, IShellItemImageFactory, KF_FLAG_DEFAULT,
                SHCreateItemFromParsingName, SHGetKnownFolderItem, SHGetKnownFolderPath, SIGDN,
                SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING, SIIGBF_ICONONLY, SIIGBF_SCALEUP,
                ShellExecuteW,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
    core::{GUID, Interface, PCWSTR, PWSTR, w},
};

const MAX_ENTRIES: usize = 10_000;
const MAX_DIRECTORY_DEPTH: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchTarget {
    Shortcut(PathBuf),
    /// A Shell AppsFolder child parsing name, not a command line.
    AppId(String),
    WindowsCommand(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub keywords: Vec<String>,
    pub target: LaunchTarget,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub entries: Vec<AppEntry>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct IconPixels {
    pub width: u32,
    pub height: u32,
    /// Unpremultiplied RGBA, ready for `SharedPixelBuffer<Rgba8Pixel>`.
    pub rgba: Vec<u8>,
}

struct Apartment(PhantomData<Rc<()>>);
impl Apartment {
    fn new() -> Result<Self, String> {
        // SAFETY: initialized and balanced on this same thread. The !Send guard
        // is declared before COM values, so they are dropped before uninitializing.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)
                .ok()
                .map_err(|e| format!("Could not initialize application discovery: {e}"))?;
        }
        Ok(Self(PhantomData))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: successful initialization, including S_FALSE, owns one balance.
        unsafe { CoUninitialize() }
    }
}

fn wide(value: &std::ffi::OsStr) -> Result<Vec<u16>, String> {
    let mut result: Vec<u16> = value.encode_wide().collect();
    if result.contains(&0) {
        return Err("Application target contains an invalid NUL character.".into());
    }
    result.push(0);
    Ok(result)
}

fn known_folder(id: &GUID) -> Result<PathBuf, String> {
    // SAFETY: Shell allocates a UTF-16 string with CoTaskMemAlloc. Copy it before
    // releasing it; the caller's apartment outlives this call.
    unsafe {
        let name = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).map_err(|e| e.to_string())?;
        let path = PathBuf::from(OsString::from_wide(name.as_wide()));
        CoTaskMemFree(Some(name.0.cast()));
        Ok(path)
    }
}

fn shortcut_name(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?;
    if !extension.eq_ignore_ascii_case("lnk") && !extension.eq_ignore_ascii_case("url") {
        return None;
    }
    let name = path.file_stem()?.to_string_lossy().trim().to_owned();
    (!name.is_empty()).then_some(name)
}

fn scan_shortcuts(root: &Path, catalog: &mut Catalog) {
    let mut directories = vec![(root.to_owned(), 0usize)];
    while let Some((directory, depth)) = directories.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                catalog.warnings.push(format!(
                    "Could not read Start Menu folder {}: {error}",
                    directory.display()
                ));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    catalog
                        .warnings
                        .push(format!("Could not read a Start Menu entry: {error}"));
                    continue;
                }
            };
            if catalog.entries.len() >= MAX_ENTRIES {
                catalog
                    .warnings
                    .push("Application catalog reached its entry limit.".into());
                return;
            }
            let path = entry.path();
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            // Do not follow directory junctions/symlinks into arbitrary trees.
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                continue;
            }
            if metadata.is_dir() && depth < MAX_DIRECTORY_DEPTH {
                directories.push((path, depth + 1));
            } else if metadata.is_file()
                && let Some(name) = shortcut_name(&path)
            {
                catalog.entries.push(AppEntry {
                    id: format!("shortcut:{}", path.to_string_lossy().to_lowercase()),
                    name,
                    detail: "Start Menu".into(),
                    keywords: vec![],
                    target: LaunchTarget::Shortcut(path),
                });
            }
        }
    }
}

fn display_name(item: &IShellItem, kind: SIGDN) -> Result<String, String> {
    // SAFETY: the owned Shell string remains valid until CoTaskMemFree. All
    // IShellItems stay on the worker's initialized apartment.
    unsafe {
        let text: PWSTR = item.GetDisplayName(kind).map_err(|e| e.to_string())?;
        let owned = String::from_utf16_lossy(text.as_wide());
        CoTaskMemFree(Some(text.0.cast()));
        Ok(owned)
    }
}

fn scan_apps(catalog: &mut Catalog) -> Result<(), String> {
    // SAFETY: Shell COM objects are created, used, and released in this apartment;
    // only copied names and targets escape to the catalog.
    unsafe {
        let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)
            .map_err(|e| format!("Could not open Windows installed applications: {e}"))?;
        let enumerator: IEnumShellItems = folder
            .BindToHandler(None, &BHID_EnumItems)
            .map_err(|e| format!("Could not enumerate Windows installed applications: {e}"))?;
        let mut names: HashSet<String> = catalog
            .entries
            .iter()
            .map(|e| e.name.to_lowercase())
            .collect();
        while catalog.entries.len() < MAX_ENTRIES {
            let mut items = [None];
            let mut fetched = 0;
            enumerator
                .Next(&mut items, Some(&mut fetched))
                .map_err(|e| e.to_string())?;
            if fetched == 0 {
                return Ok(());
            }
            let Some(item) = items[0].take() else {
                continue;
            };
            let name = match display_name(&item, SIGDN_NORMALDISPLAY) {
                Ok(name) if !name.trim().is_empty() => name,
                _ => continue,
            };
            let target = match display_name(&item, SIGDN_PARENTRELATIVEPARSING) {
                Ok(target) if !target.is_empty() => target,
                _ => continue,
            };
            // Prefer a matching Start Menu shortcut, which carries the user's
            // original arguments and working directory. AppsFolder fills gaps.
            if !names.insert(name.to_lowercase()) {
                continue;
            }
            catalog.entries.push(AppEntry {
                id: format!("app:{}", target.to_lowercase()),
                name,
                detail: "Installed app".into(),
                keywords: vec![],
                target: LaunchTarget::AppId(target),
            });
        }
        catalog
            .warnings
            .push("Application catalog reached its entry limit.".into());
        Ok(())
    }
}

/// Discover per-user/common Start Menu shortcuts plus the Shell's installed apps.
/// Partial discovery is returned with warnings; an entirely failed discovery is
/// an error rather than an indistinguishable empty success.
pub fn enumerate() -> Result<Catalog, String> {
    let _apartment = Apartment::new()?;
    let mut catalog = Catalog::default();
    for id in [&FOLDERID_StartMenu, &FOLDERID_CommonStartMenu] {
        match known_folder(id) {
            Ok(path) => scan_shortcuts(&path, &mut catalog),
            Err(error) => catalog
                .warnings
                .push(format!("Could not locate Start Menu: {error}")),
        }
    }
    if let Err(error) = scan_apps(&mut catalog) {
        catalog.warnings.push(error);
    }
    catalog
        .entries
        .extend(crate::windows_commands::ENTRIES.iter().map(|entry| {
            AppEntry {
                id: format!("windows:{}", entry.id),
                name: entry.label.into(),
                detail: if entry.executable.is_empty() {
                    "Windows Settings"
                } else {
                    "Windows system tool"
                }
                .into(),
                keywords: entry.keywords.split('|').map(str::to_owned).collect(),
                target: LaunchTarget::WindowsCommand(entry.id.into()),
            }
        }));
    catalog.entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    let mut ids = HashSet::new();
    catalog.entries.retain(|entry| ids.insert(entry.id.clone()));
    if catalog.entries.is_empty() && !catalog.warnings.is_empty() {
        return Err(catalog.warnings.join("\n"));
    }
    Ok(catalog)
}

fn target_name(target: &LaunchTarget) -> OsString {
    match target {
        LaunchTarget::Shortcut(path) => path.as_os_str().to_owned(),
        LaunchTarget::AppId(id) => OsString::from(format!("shell:AppsFolder\\{id}")),
        LaunchTarget::WindowsCommand(_) => OsString::new(),
    }
}

/// Ask the Shell to open the exact catalog target. Shortcut arguments and working
/// directories are resolved by Windows; no shell command string is constructed.
/// Success means the Shell accepted launch, not that the app has finished opening.
pub fn launch(target: &LaunchTarget) -> Result<(), String> {
    let _apartment = Apartment::new()?;
    let command = if let LaunchTarget::WindowsCommand(id) = target {
        Some(crate::windows_commands::resolve(id)?)
    } else {
        None
    };
    let name = if let Some(entry) = command {
        if entry.executable.is_empty() {
            OsString::from(entry.argument)
        } else {
            PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
                .join("System32")
                .join(entry.executable)
                .into_os_string()
        }
    } else {
        target_name(target)
    };
    let parameters = command
        .filter(|entry| !entry.executable.is_empty() && !entry.argument.is_empty())
        .map(|entry| wide(std::ffi::OsStr::new(entry.argument)))
        .transpose()?;
    let target = wide(&name)?;
    // SAFETY: the NUL-terminated strings outlive the synchronous Shell call.
    // No parameters are supplied and no untrusted string is treated as a command.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(target.as_ptr()),
            parameters
                .as_ref()
                .map(|value| PCWSTR(value.as_ptr()))
                .unwrap_or(PCWSTR::null()),
            None,
            SW_SHOWNORMAL,
        )
    };
    let status = result.0 as isize;
    if status <= 32 {
        Err(format!(
            "Windows could not launch this application (Shell error {status})."
        ))
    } else {
        Ok(())
    }
}

struct Bitmap(HBITMAP);
impl Drop for Bitmap {
    fn drop(&mut self) {
        // SAFETY: GetImage transfers this owned HBITMAP, never selected into a DC.
        unsafe {
            let _ = DeleteObject(self.0.into());
        }
    }
}
struct MemoryDc(HDC);
impl Drop for MemoryDc {
    fn drop(&mut self) {
        // SAFETY: CreateCompatibleDC ownership is paired with DeleteDC.
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}

/// Extract a native icon at the display's physical pixel size, not a stretched
/// low-resolution image. Cache by stable application ID and physical_size in the
/// controller. This function keeps no GDI resources alive after returning.
pub fn load_icon(target: &LaunchTarget, physical_size: u32) -> Result<IconPixels, String> {
    if matches!(target, LaunchTarget::WindowsCommand(_)) {
        return Err("Use Windows command category icon".into());
    }
    let _apartment = Apartment::new()?;
    let size = physical_size.clamp(16, 256);
    let target = wide(&target_name(target))?;
    // SAFETY: Shell allocates the bitmap and all memory buffers below are sized
    // from bounded validated bitmap dimensions. No bitmap is selected into a DC.
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(target.as_ptr()), None)
            .map_err(|e| format!("Could not locate application icon: {e}"))?;
        let factory: IShellItemImageFactory = item.cast().map_err(|e| e.to_string())?;
        let bitmap = Bitmap(
            factory
                .GetImage(
                    SIZE {
                        cx: size as i32,
                        cy: size as i32,
                    },
                    SIIGBF_ICONONLY | SIIGBF_SCALEUP,
                )
                .map_err(|e| format!("Could not load application icon: {e}"))?,
        );
        let mut object = BITMAP::default();
        if GetObjectW(
            bitmap.0.into(),
            size_of::<BITMAP>() as i32,
            Some((&mut object as *mut BITMAP).cast()),
        ) == 0
        {
            return Err("Could not inspect application icon bitmap.".into());
        }
        let width = object.bmWidth;
        let height = object.bmHeight.abs();
        if !(1..=256).contains(&width) || !(1..=256).contains(&height) {
            return Err("Application icon dimensions exceed the supported range.".into());
        }
        let dc = MemoryDc(CreateCompatibleDC(None));
        if dc.0.is_invalid() {
            return Err("Could not create icon conversion context.".into());
        }
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bytes = vec![0u8; width as usize * height as usize * 4];
        if GetDIBits(
            dc.0,
            bitmap.0,
            0,
            height as u32,
            Some(bytes.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        ) != height
        {
            return Err("Could not read application icon pixels.".into());
        }
        bgra_to_rgba(&mut bytes);
        Ok(IconPixels {
            width: width as u32,
            height: height as u32,
            rgba: bytes,
        })
    }
}

fn bgra_to_rgba(bytes: &mut [u8]) {
    // Legacy Shell bitmaps sometimes have no alpha channel. Preserve their RGB
    // rather than returning an invisible image. Modern Shell icons are premultiplied.
    let legacy = bytes.as_chunks::<4>().0.iter().all(|p| p[3] == 0);
    for pixel in bytes.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        if legacy {
            pixel[3] = 255;
        } else if pixel[3] != 0 {
            let alpha = u32::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_start_menu_shortcuts_are_catalogued() {
        assert_eq!(
            shortcut_name(Path::new("Editor.LNK")),
            Some("Editor".into())
        );
        assert_eq!(shortcut_name(Path::new("Game.url")), Some("Game".into()));
        assert_eq!(shortcut_name(Path::new("notes.txt")), None);
    }
    #[test]
    fn icon_pixels_preserve_transparency_and_unpremultiply() {
        let mut pixels = [0, 0, 64, 128, 0, 0, 0, 0];
        bgra_to_rgba(&mut pixels);
        assert_eq!(pixels, [128, 0, 0, 128, 0, 0, 0, 0]);
        let mut legacy = [1, 2, 3, 0];
        bgra_to_rgba(&mut legacy);
        assert_eq!(legacy, [3, 2, 1, 255]);
    }
    #[test]
    fn shell_targets_are_data_and_preserve_unicode() {
        let target = LaunchTarget::Shortcut(PathBuf::from("C:\\工具\\Editor.lnk"));
        let encoded = wide(&target_name(&target)).unwrap();
        assert_eq!(*encoded.last().unwrap(), 0);
        assert!(wide(std::ffi::OsStr::new("bad\0target")).is_err());
        assert_eq!(
            target_name(&LaunchTarget::AppId("Package!App".into())),
            "shell:AppsFolder\\Package!App"
        );
    }
}

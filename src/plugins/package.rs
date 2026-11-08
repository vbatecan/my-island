//! Bounded, non-executing local package import.
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use still_island_plugin_protocol::{Manifest, validate_manifest};

const MAX_ARCHIVE: u64 = 50 * 1024 * 1024;
const MAX_EXTRACTED: u64 = 250 * 1024 * 1024;
pub fn relative(name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.contains(['\\', ':', '\0']) {
        return Err("Invalid package path".into());
    }
    let p = Path::new(name);
    for c in p.components() {
        match c {
            Component::Normal(v) => {
                let s = v.to_string_lossy();
                let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
                if s.ends_with(['.', ' '])
                    || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                    || (stem.len() == 4
                        && (stem.starts_with("COM") || stem.starts_with("LPT"))
                        && stem.as_bytes()[3].is_ascii_digit())
                {
                    return Err("Reserved package path".into());
                }
            }
            _ => return Err("Package paths must stay inside the package".into()),
        }
    }
    Ok(p.to_path_buf())
}
pub fn read_manifest(path: &Path) -> Result<Manifest, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(65537).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("Manifest exceeds 64 KiB".into());
    }
    let m: Manifest =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid manifest: {e}"))?;
    validate_manifest(&m)?;
    if m.id.eq_ignore_ascii_case("config") || m.id != m.id.to_ascii_lowercase() {
        return Err("Plugin IDs must be lowercase and cannot use the config namespace".into());
    }
    for part in [&m.id, &m.version] {
        let p = relative(part)?;
        if p.components().count() != 1
            || !part
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
        {
            return Err("Plugin identity/version must be safe single components".into());
        }
    }
    relative(&m.entrypoint)?;
    if !m.entrypoint.ends_with(".exe") {
        return Err("Windows packages require an .exe entry point".into());
    }
    for asset in &m.assets {
        relative(asset)?;
    }
    Ok(m)
}
pub fn contained_file(root: &Path, name: &str) -> Result<PathBuf, String> {
    use std::os::windows::fs::MetadataExt;
    let rel = relative(name)?;
    let mut path = root.to_path_buf();
    for component in rel.components() {
        path.push(component);
        let m = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if m.file_attributes() & 0x400 != 0 {
            return Err("Plugin paths cannot contain reparse points".into());
        }
    }
    let actual = path.canonicalize().map_err(|e| e.to_string())?;
    if !actual.starts_with(root) || !actual.is_file() {
        return Err("Plugin file escapes its package".into());
    }
    Ok(actual)
}
pub fn directory_inside(root: &Path, path: &Path) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "Directory escapes plugin storage")?;
    let mut current = root.to_path_buf();
    for part in relative.components() {
        if !matches!(part, Component::Normal(_)) {
            return Err("Invalid directory component".into());
        }
        current.push(part);
        let m = fs::symlink_metadata(&current).map_err(|e| e.to_string())?;
        if !m.is_dir() || m.file_attributes() & 0x400 != 0 {
            return Err("Plugin directories cannot contain reparse points".into());
        }
    }
    Ok(())
}
pub fn import(archive: &Path, root: &Path) -> Result<(Manifest, PathBuf), String> {
    if fs::metadata(archive).map_err(|e| e.to_string())?.len() > MAX_ARCHIVE {
        return Err("Package exceeds 50 MiB".into());
    }
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let stage = root.join(format!(
        ".import-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut zip = zip::ZipArchive::new(fs::File::open(archive).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if zip.len() > 256 {
            return Err("Package has too many entries".into());
        }
        let mut names = HashSet::new();
        let mut total = 0u64;
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
            let rel = relative(entry.name())?;
            if !names.insert(entry.name().trim_end_matches('/').to_ascii_lowercase()) {
                return Err("Duplicate package entry".into());
            }
            let mode = entry.unix_mode().unwrap_or(0) & 0o170000;
            if mode != 0 && mode != 0o100000 && mode != 0o040000 {
                return Err("Links and special files are not supported".into());
            }
            total = total
                .checked_add(entry.size())
                .ok_or("Package size overflow")?;
            if total > MAX_EXTRACTED {
                return Err("Expanded package exceeds 250 MiB".into());
            }
            let target = stage.join(rel);
            if entry.is_dir() {
                fs::create_dir_all(&target).map_err(|e| e.to_string())?;
                continue;
            }
            fs::create_dir_all(target.parent().ok_or("Invalid destination")?)
                .map_err(|e| e.to_string())?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map_err(|e| e.to_string())?;
            let copied = std::io::copy(&mut entry.by_ref().take(MAX_EXTRACTED + 1), &mut file)
                .map_err(|e| e.to_string())?;
            if copied != entry.size() {
                return Err("Invalid expanded package size".into());
            }
            file.flush().map_err(|e| e.to_string())?;
        }
        let manifest = read_manifest(&stage.join("manifest.json"))?;
        let exe = stage.join(relative(&manifest.entrypoint)?);
        let mut header = [0u8; 2];
        fs::File::open(exe)
            .and_then(|mut f| f.read_exact(&mut header))
            .map_err(|_| "Missing executable entry point")?;
        if header != *b"MZ" {
            return Err("Entry point is not a Windows executable".into());
        }
        for asset in &manifest.assets {
            if !stage.join(relative(asset)?).is_file() {
                return Err("A declared asset is missing".into());
            }
        }
        let parent = root.join(&manifest.id);
        if parent.exists() {
            use std::os::windows::fs::MetadataExt;
            if fs::symlink_metadata(&parent)
                .map_err(|e| e.to_string())?
                .file_attributes()
                & 0x400
                != 0
            {
                return Err("Plugin directory is a reparse point".into());
            }
        }
        fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
        if !parent
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(&root)
        {
            return Err("Invalid plugin directory".into());
        }
        let target = parent.join(&manifest.version);
        if target.exists() {
            return Err("This plugin version is already installed".into());
        }
        fs::rename(&stage, &target).map_err(|e| e.to_string())?;
        Ok((manifest, target))
    })();
    if stage.exists() {
        safe_remove(&stage, &root)?;
    }
    result
}
pub fn safe_remove(path: &Path, root: &Path) -> Result<(), String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let actual = path.canonicalize().map_err(|e| e.to_string())?;
    if actual == root || !actual.starts_with(&root) {
        return Err("Removal target escapes the plugin directory".into());
    }
    // Never traverse reparse points, including ones introduced after import.
    fn check(p: &Path) -> Result<(), String> {
        use std::os::windows::fs::MetadataExt;
        let m = fs::symlink_metadata(p).map_err(|e| e.to_string())?;
        if m.file_attributes() & 0x400 != 0 {
            return Err("Plugin directory contains a reparse point".into());
        }
        if m.is_dir() {
            for child in fs::read_dir(p).map_err(|e| e.to_string())? {
                check(&child.map_err(|e| e.to_string())?.path())?;
            }
        }
        Ok(())
    }
    check(path)?;
    fs::remove_dir_all(path).map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_paths_cannot_escape() {
        for p in [
            "../x",
            "/absolute",
            "C:/x",
            "a\\x",
            "a/../../x",
            "NUL.txt",
            "a/COM1.exe",
            "a/file.",
        ] {
            assert!(relative(p).is_err(), "{p}");
        }
        assert!(relative("assets/icon.png").is_ok());
    }
}

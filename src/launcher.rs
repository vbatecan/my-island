//! Application ranking, local launch history, and user-created shortcuts.
use crate::platform::launcher::{AppEntry, LaunchTarget, is_absolute_folder_path};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{self, Read, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

const MAX_RECENTS: usize = 32;
const MAX_HISTORY_BYTES: u64 = 64 * 1024;
const MAX_RESULTS: usize = 50;
const MAX_SHORTCUTS: usize = 128;
const MAX_ALIAS_CHARS: usize = 64;
const MAX_TARGET_BYTES: usize = 8 * 1024;
const MAX_SHORTCUT_FILE_BYTES: u64 = 4 * 1024 * 1024;
const USER_SHORTCUT_PREFIX: &str = "user-shortcut:";
static SAVE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecentLaunches {
    // Most recently launched first. IDs survive display-name changes.
    ids: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct HistoryFile {
    version: u32,
    ids: Vec<String>,
}

fn valid_id(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= 2048 && !id.chars().any(char::is_control)
}

fn read_bounded(path: &Path, max_bytes: u64) -> Option<Vec<u8>> {
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > max_bytes {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(max_bytes + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= max_bytes).then_some(bytes)
}

fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing parent directory"))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing file name"))?
        .to_string_lossy();
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        SAVE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

impl RecentLaunches {
    pub fn load(path: &Path) -> Self {
        let load = || -> Option<Self> {
            let bytes = read_bounded(path, MAX_HISTORY_BYTES)?;
            let data: HistoryFile = serde_json::from_slice(&bytes).ok()?;
            if data.version != 1 {
                return None;
            }
            let mut seen = HashSet::new();
            let ids = data
                .ids
                .into_iter()
                .filter(|id| valid_id(id) && seen.insert(id.clone()))
                .take(MAX_RECENTS)
                .collect();
            Some(Self { ids })
        };
        load().unwrap_or_default()
    }

    /// Call only after Windows has accepted the launch request.
    pub fn record(&mut self, id: &str) {
        if !valid_id(id) {
            return;
        }
        self.ids.retain(|previous| previous != id);
        self.ids.insert(0, id.to_owned());
        self.ids.truncate(MAX_RECENTS);
    }

    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    pub fn clear(&mut self) {
        self.ids.clear();
    }

    /// Write a sibling temporary file, then replace the destination in one
    /// rename. A failed write leaves the previous history intact.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec(&HistoryFile {
            version: 1,
            ids: self.ids.clone(),
        })
        .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_HISTORY_BYTES {
            return Err("Launch history exceeds its size limit".into());
        }
        write_atomically(path, &bytes).map_err(|error| error.to_string())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LauncherShortcuts {
    pins: Vec<String>,
    shortcuts: Vec<UserShortcut>,
    next_shortcut_id: u64,
}

#[derive(Serialize, Deserialize)]
struct ShortcutsFile<T> {
    version: u32,
    #[serde(flatten)]
    shortcuts: T,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct UserShortcut {
    id: String,
    alias: String,
    target: String,
}

impl LauncherShortcuts {
    pub fn load(path: &Path) -> Self {
        let load = || -> Option<Self> {
            let bytes = read_bounded(path, MAX_SHORTCUT_FILE_BYTES)?;
            let file: ShortcutsFile<Self> = serde_json::from_slice(&bytes).ok()?;
            if file.version != 1 {
                return None;
            }
            let mut data = file.shortcuts;
            let mut ids = HashSet::new();
            let mut aliases = HashSet::new();
            data.shortcuts.retain_mut(|shortcut| {
                let Some(ordinal) = shortcut
                    .id
                    .strip_prefix(USER_SHORTCUT_PREFIX)
                    .and_then(|ordinal| ordinal.parse::<u64>().ok())
                else {
                    return false;
                };
                let Ok(alias) = checked_alias(&shortcut.alias) else {
                    return false;
                };
                let Ok(target) = checked_target(&shortcut.target) else {
                    return false;
                };
                let normalized_alias = normalized(alias, MAX_ALIAS_CHARS * 2);
                if aliases.len() == MAX_SHORTCUTS
                    || shortcut.id != format!("{USER_SHORTCUT_PREFIX}{ordinal}")
                    || aliases.contains(&normalized_alias)
                    || !ids.insert(shortcut.id.clone())
                {
                    return false;
                }
                aliases.insert(normalized_alias);
                data.next_shortcut_id = data.next_shortcut_id.max(ordinal.saturating_add(1));
                shortcut.alias = alias.to_owned();
                shortcut.target = target;
                true
            });
            let mut seen = HashSet::new();
            data.pins.retain(|id| {
                seen.len() < MAX_RESULTS
                    && valid_id(id)
                    && (!id.starts_with(USER_SHORTCUT_PREFIX) || ids.contains(id))
                    && seen.insert(id.clone())
            });
            Some(data)
        };
        load().unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let save_error = "Could not save launcher shortcuts. Check that Still Island's settings folder is writable.";
        let bytes = serde_json::to_vec(&ShortcutsFile {
            version: 1,
            shortcuts: self,
        })
        .map_err(|_| save_error.to_owned())?;
        if bytes.len() as u64 > MAX_SHORTCUT_FILE_BYTES {
            return Err("Launcher shortcuts exceed the storage limit.".into());
        }
        write_atomically(path, &bytes).map_err(|_| save_error.into())
    }

    pub fn entries(&self) -> Vec<AppEntry> {
        self.shortcuts
            .iter()
            .map(|shortcut| {
                let (target, detail) = if shortcut.target.starts_with("http://")
                    || shortcut.target.starts_with("https://")
                {
                    (LaunchTarget::Url(shortcut.target.clone()), "Website")
                } else {
                    (
                        LaunchTarget::Folder(shortcut.target.clone().into()),
                        "Folder",
                    )
                };
                AppEntry {
                    id: shortcut.id.clone(),
                    name: shortcut.alias.clone(),
                    detail: detail.into(),
                    keywords: Vec::new(),
                    target,
                }
            })
            .collect()
    }

    pub fn is_pinned(&self, id: &str) -> bool {
        self.pins.iter().any(|pinned| pinned == id)
    }

    pub fn toggle_pin(&mut self, id: &str) -> Result<(), String> {
        if let Some(index) = self.pins.iter().position(|pinned| pinned == id) {
            self.pins.remove(index);
            return Ok(());
        }
        if !valid_id(id) || (id.starts_with(USER_SHORTCUT_PREFIX) && self.shortcut(id).is_none()) {
            return Err("This item is no longer available.".into());
        }
        if self.pins.len() == MAX_RESULTS {
            return Err("Unpin an item before adding another pin.".into());
        }
        self.pins.push(id.to_owned());
        Ok(())
    }

    pub fn put_shortcut(
        &mut self,
        previous_id: Option<&str>,
        alias: &str,
        target: &str,
    ) -> Result<String, String> {
        let alias = checked_alias(alias)?;
        let target = checked_target(target)?;
        let name = normalized(alias, MAX_ALIAS_CHARS * 2);
        if self.shortcuts.iter().any(|shortcut| {
            Some(shortcut.id.as_str()) != previous_id
                && normalized(&shortcut.alias, MAX_ALIAS_CHARS * 2) == name
        }) {
            return Err("A shortcut already uses that name.".into());
        }
        if let Some(id) = previous_id {
            let shortcut = self
                .shortcuts
                .iter_mut()
                .find(|shortcut| shortcut.id == id)
                .ok_or("This shortcut is no longer available.")?;
            shortcut.alias = alias.to_owned();
            shortcut.target = target;
            return Ok(shortcut.id.clone());
        }
        if self.shortcuts.len() == MAX_SHORTCUTS {
            return Err("Remove a shortcut before adding another.".into());
        }
        let id = format!("{USER_SHORTCUT_PREFIX}{}", self.next_shortcut_id);
        self.next_shortcut_id = self
            .next_shortcut_id
            .checked_add(1)
            .ok_or("Could not create another launcher shortcut.")?;
        self.shortcuts.push(UserShortcut {
            id: id.clone(),
            alias: alias.to_owned(),
            target,
        });
        Ok(id)
    }

    pub fn remove_shortcut(&mut self, id: &str) {
        if let Some(index) = self.shortcuts.iter().position(|shortcut| shortcut.id == id) {
            self.shortcuts.remove(index);
            self.pins.retain(|pinned| pinned != id);
        }
    }

    pub fn shortcut(&self, id: &str) -> Option<(&str, &str)> {
        let shortcut = self.shortcuts.iter().find(|shortcut| shortcut.id == id)?;
        Some((&shortcut.alias, &shortcut.target))
    }

    pub fn has_exact_alias(&self, query: &str) -> bool {
        let query = normalized(query, 128);
        self.shortcuts
            .iter()
            .any(|shortcut| normalized(&shortcut.alias, MAX_ALIAS_CHARS * 2) == query)
    }
}

fn checked_alias(alias: &str) -> Result<&str, String> {
    if alias.chars().any(char::is_control) {
        return Err("Use a single line for the shortcut name.".into());
    }
    let alias = alias.trim();
    if alias.is_empty() || alias.chars().count() > MAX_ALIAS_CHARS {
        return Err(format!(
            "Enter a shortcut name up to {MAX_ALIAS_CHARS} characters."
        ));
    }
    Ok(alias)
}

fn checked_target(target: &str) -> Result<String, String> {
    let invalid =
        || "Enter a full Windows folder path or an http:// or https:// address.".to_owned();
    if target.is_empty() || target.len() > MAX_TARGET_BYTES || target.chars().any(char::is_control)
    {
        return Err(invalid());
    }
    if is_absolute_folder_path(Path::new(target)) {
        return Ok(target.to_owned());
    }

    let target = target.trim_matches(' ');
    let (scheme, address) = target.split_once("://").ok_or_else(invalid)?;
    let authority = address.split(['/', '?', '#']).next().unwrap_or("");
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        || authority.is_empty()
        || target.contains('\\')
        || target.chars().any(char::is_whitespace)
    {
        return Err(invalid());
    }
    if authority.contains('@') {
        return Err("Use a website address without a username or password.".into());
    }
    let url = url::Url::parse(target).map_err(|_| invalid())?;
    if url.as_str().len() > MAX_TARGET_BYTES {
        return Err(invalid());
    }
    Ok(url.into())
}

fn normalized(text: &str, maximum: usize) -> String {
    let mut output = String::new();
    let mut count = 0;
    let mut pending_space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            pending_space = !output.is_empty();
            continue;
        }
        if pending_space {
            if count >= maximum {
                break;
            }
            output.push(' ');
            count += 1;
            pending_space = false;
        }
        for lowercase in character.to_lowercase() {
            if count >= maximum {
                return output;
            }
            output.push(lowercase);
            count += 1;
        }
    }
    output
}

/// Strict tiers ensure a fuzzy match never outranks an exact or prefix match.
/// Within a tier, contiguous letters, word starts and short gaps rank higher.
fn match_score(name: &str, query: &str) -> Option<(u8, usize)> {
    if name == query {
        return Some((0, 0));
    }
    if name.starts_with(query) {
        return Some((
            1,
            name.chars().count().saturating_sub(query.chars().count()),
        ));
    }
    if let Some((offset, _)) = name.match_indices(query).find(|(offset, _)| {
        name[..*offset]
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_alphanumeric())
    }) {
        return Some((2, name[..offset].chars().count()));
    }
    if let Some(offset) = name.find(query) {
        return Some((3, name[..offset].chars().count()));
    }

    let letters: Vec<char> = name.chars().collect();
    let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).collect();
    let mut cursor = 0;
    let mut cost = 0usize;
    for (query_index, wanted) in query.iter().enumerate() {
        let relative = letters[cursor..]
            .iter()
            .position(|letter| letter == wanted)?;
        let at = cursor + relative;
        let word_start = at == 0 || !letters[at - 1].is_alphanumeric();
        cost = cost.saturating_add(relative.saturating_mul(4));
        if !word_start && relative != 0 {
            cost = cost.saturating_add(3);
        }
        if query_index == 0 {
            cost = cost.saturating_add(at);
        }
        cursor = at + 1;
    }
    Some((
        4,
        cost.saturating_add(letters.len().saturating_sub(query.len())),
    ))
}

#[cfg(test)]
pub fn search(
    apps: &[AppEntry],
    query: &str,
    recents: &RecentLaunches,
    limit: usize,
) -> Vec<usize> {
    search_with_shortcuts(
        apps,
        query,
        recents,
        &LauncherShortcuts::default(),
        true,
        limit,
    )
}

/// Return catalog indices without cloning targets or icons. On an empty query,
/// show pinned items, then recent items, then alphabetical matches.
pub fn search_with_shortcuts(
    apps: &[AppEntry],
    query: &str,
    recents: &RecentLaunches,
    shortcuts: &LauncherShortcuts,
    include_windows: bool,
    limit: usize,
) -> Vec<usize> {
    let query = normalized(query, 128);
    let mut seen = HashSet::new();
    let mut matches: Vec<_> = apps
        .iter()
        .enumerate()
        .filter(|(_, app)| include_windows || !app.id.starts_with("windows:"))
        .filter(|(_, app)| seen.insert(app.id.as_str()))
        .filter_map(|(index, app)| {
            let name = normalized(&app.name, 512);
            let recent = recents
                .ids
                .iter()
                .position(|id| id == &app.id)
                .unwrap_or(usize::MAX);
            let score = if query.is_empty() {
                (0, 0)
            } else {
                let name_score = match_score(&name, &query)
                    .map(|(tier, cost)| (if tier >= 2 { tier + 1 } else { tier }, cost));
                let keyword_score = app
                    .keywords
                    .iter()
                    .filter_map(|keyword| {
                        let keyword = normalized(keyword, 512);
                        match_score(&keyword, &query)
                            .map(|(tier, cost)| (if tier < 4 { 2 } else { 5 }, cost))
                    })
                    .min();
                name_score.into_iter().chain(keyword_score).min()?
            };
            let exact_alias = !query.is_empty()
                && score == (0, 0)
                && matches!(app.target, LaunchTarget::Folder(_) | LaunchTarget::Url(_));
            let pin = if query.is_empty() {
                shortcuts
                    .pins
                    .iter()
                    .position(|id| id == &app.id)
                    .unwrap_or(usize::MAX)
            } else {
                usize::MAX
            };
            Some((
                score,
                !exact_alias,
                pin,
                recent,
                name,
                app.id.as_str(),
                index,
            ))
        })
        .collect();
    matches.sort_unstable();
    matches
        .into_iter()
        .take(limit.min(MAX_RESULTS))
        .map(|(_, _, _, _, _, _, index)| index)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::launcher::LaunchTarget;
    use std::path::PathBuf;

    fn app(id: &str, name: &str) -> AppEntry {
        AppEntry {
            id: id.into(),
            name: name.into(),
            detail: "Test fixture".into(),
            keywords: vec![],
            target: LaunchTarget::Shortcut(PathBuf::from("unused.lnk")),
        }
    }
    #[test]
    fn exact_prefix_word_substring_and_fuzzy_have_strict_order() {
        let apps = vec![
            app("fuzzy", "Command Desk"),
            app("substring", "Decode"),
            app("word", "My Code"),
            app("prefix", "Code Editor"),
            app("exact", "Code"),
        ];
        assert_eq!(
            search(&apps, "code", &RecentLaunches::default(), 10),
            vec![4, 3, 2, 1, 0]
        );
    }
    #[test]
    fn windows_keywords_follow_name_prefixes_before_fuzzy_matches() {
        let mut action = app("windows:installed-apps", "Installed apps");
        action.keywords = vec!["add or remove programs".into(), "uninstall apps".into()];
        let apps = vec![
            app("prefix", "Uninstall apps helper"),
            action,
            app("exact", "Uninstall apps"),
        ];
        assert_eq!(
            search(&apps, "uninstall apps", &RecentLaunches::default(), 6),
            vec![2, 0, 1]
        );
        assert_eq!(
            search(&apps, "add or remove", &RecentLaunches::default(), 6),
            vec![1]
        );
        let mut history = RecentLaunches::default();
        history.record("windows:installed-apps");
        assert_eq!(search(&apps, "", &history, 6)[0], 1);
    }
    #[test]
    fn matching_is_case_insensitive_unicode_and_whitespace_aware() {
        let apps = vec![app("a", "Éditeur Texte"), app("b", "Windows Terminal")];
        assert_eq!(
            search(&apps, "  ÉDITEUR   TEXTE ", &RecentLaunches::default(), 4),
            vec![0]
        );
        assert_eq!(
            search(&apps, "wtrm", &RecentLaunches::default(), 4),
            vec![1]
        );
        assert!(search(&apps, "nonexistent", &RecentLaunches::default(), 4).is_empty());
    }
    #[test]
    fn blank_query_uses_recents_then_names_and_ignores_unknown_or_duplicate_ids() {
        let apps = vec![
            app("z", "Zulu"),
            app("a", "Alpha"),
            app("b", "Beta"),
            app("z", "Duplicate"),
        ];
        let mut recent = RecentLaunches::default();
        recent.record("z");
        recent.record("missing");
        assert_eq!(search(&apps, "", &recent, 8), vec![0, 1, 2]);
        assert!(search(&apps, "", &recent, 0).is_empty());
    }
    #[test]
    fn recency_breaks_equal_scores_but_never_overrides_match_quality() {
        let apps = vec![app("a", "Code"), app("b", "Code"), app("c", "Code Editor")];
        let mut recent = RecentLaunches::default();
        recent.record("b");
        recent.record("c");
        assert_eq!(search(&apps, "code", &recent, 8), vec![1, 0, 2]);
    }
    #[test]
    fn history_and_results_are_bounded() {
        let mut recent = RecentLaunches::default();
        let apps: Vec<_> = (0..100)
            .map(|i| app(&i.to_string(), &format!("App {i}")))
            .collect();
        for app in &apps {
            recent.record(&app.id);
        }
        recent.record("99");
        recent.record("\ninvalid");
        assert_eq!(recent.ids.len(), MAX_RECENTS);
        assert_eq!(recent.ids[0], "99");
        assert_eq!(recent.ids.iter().filter(|id| *id == "99").count(), 1);
        assert_eq!(search(&apps, "", &recent, usize::MAX).len(), MAX_RESULTS);
    }
    #[test]
    fn history_round_trips_and_safely_replaces_existing_file() {
        let directory = std::env::temp_dir().join(format!(
            "still-island-launch-history-{}-{}",
            std::process::id(),
            SAVE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let path = directory.join("recent.json");
        let mut recent = RecentLaunches::default();
        recent.record("shortcut:one");
        recent.save(&path).unwrap();
        recent.record("app:two");
        recent.save(&path).unwrap();
        assert_eq!(RecentLaunches::load(&path), recent);
        fs::write(&path, br#"{"version":1,"ids":["a","a","","b"]}"#).unwrap();
        assert_eq!(RecentLaunches::load(&path).ids, vec!["a", "b"]);
        fs::write(&path, b"malformed JSON").unwrap();
        assert!(RecentLaunches::load(&path).ids.is_empty());
        fs::write(&path, vec![b' '; MAX_HISTORY_BYTES as usize + 1]).unwrap();
        assert!(RecentLaunches::load(&path).ids.is_empty());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn exact_aliases_outrank_apps_and_pins_stay_ahead_of_recents() {
        let mut shortcuts = LauncherShortcuts::default();
        let mut recent = RecentLaunches::default();
        let docs = shortcuts
            .put_shortcut(None, "Docs", r"C:\Users\Example\Documents")
            .unwrap();
        let guide = shortcuts
            .put_shortcut(None, "Guide", "https://example.com/guide")
            .unwrap();
        let mut apps = vec![app("app:docs", "Docs"), app("app:editor", "Editor")];
        apps.extend(shortcuts.entries());
        recent.record("app:docs");
        shortcuts.toggle_pin("app:editor").unwrap();
        shortcuts.toggle_pin(&guide).unwrap();
        assert!(shortcuts.has_exact_alias(" dOcS "));
        assert_eq!(apps[2].id, docs);
        assert_eq!(
            search_with_shortcuts(&apps, "docs", &recent, &shortcuts, true, 8),
            [2, 0]
        );
        assert_eq!(
            search_with_shortcuts(&apps, "", &recent, &shortcuts, true, 8),
            [1, 3, 0, 2]
        );
        shortcuts.toggle_pin("app:editor").unwrap();
        assert_eq!(
            search_with_shortcuts(&apps, "", &recent, &shortcuts, true, 3),
            [3, 0, 2]
        );
    }

    #[test]
    fn disabled_windows_commands_are_filtered_before_pins_fill_the_result_limit() {
        let mut shortcuts = LauncherShortcuts::default();
        let mut apps = Vec::new();
        for index in 0..MAX_RESULTS {
            let id = format!("windows:{index}");
            shortcuts.toggle_pin(&id).unwrap();
            apps.push(app(&id, "Windows command"));
        }
        shortcuts
            .put_shortcut(None, "Guide", "https://example.com/guide")
            .unwrap();
        apps.extend(shortcuts.entries());
        let recent = RecentLaunches::default();
        assert_eq!(
            search_with_shortcuts(&apps, "", &recent, &shortcuts, true, MAX_RESULTS).len(),
            MAX_RESULTS
        );
        assert_eq!(
            search_with_shortcuts(&apps, "", &recent, &shortcuts, false, MAX_RESULTS),
            [MAX_RESULTS]
        );
    }

    #[test]
    fn shortcuts_validate_targets_without_requiring_existing_folders() {
        let mut shortcuts = LauncherShortcuts::default();
        shortcuts
            .put_shortcut(None, "Project   Docs", r"Z:\QA fixtures\工具")
            .unwrap();
        assert!(
            shortcuts
                .put_shortcut(None, "project docs", "https://example.com")
                .is_err()
        );
        let website = shortcuts
            .put_shortcut(None, "Website", "HTTPS://Example.COM:443/guide?a=1&b=2")
            .unwrap();
        assert_eq!(
            shortcuts.shortcut(&website),
            Some(("Website", "https://example.com/guide?a=1&b=2"))
        );
        for target in [
            r"Docs\Templates",
            r"C:Documents",
            r"\Windows\System32",
            r"\\.\PhysicalDrive0",
            r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy1",
            r"C:\Folder:stream",
            "file:///C:/Windows",
            "javascript:alert(1)",
            "https:///example.com",
            "https://user:password@example.com",
            "https://@example.com",
            "https://example.com\0",
            "https://example.com\n",
            r"https://example.com\evil",
        ] {
            assert!(
                shortcuts.put_shortcut(None, "Invalid", target).is_err(),
                "{target:?}"
            );
        }
        assert_eq!(shortcuts.entries().len(), 2);
    }

    #[test]
    fn user_shortcuts_and_pins_round_trip_and_preserve_identity_on_edit() {
        let directory = std::env::temp_dir().join(format!(
            "still-island-launcher-shortcuts-{}-{}",
            std::process::id(),
            SAVE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let path = directory.join("shortcuts.json");
        let mut shortcuts = LauncherShortcuts::default();
        let id = shortcuts
            .put_shortcut(None, "One", "https://example.com/")
            .unwrap();
        shortcuts.toggle_pin(&id).unwrap();
        shortcuts.toggle_pin("app:editor").unwrap();
        shortcuts.save(&path).unwrap();
        let mut shortcuts = LauncherShortcuts::load(&path);
        let edited = shortcuts
            .put_shortcut(Some(&id), "Two", r"\\server\share\folder")
            .unwrap();
        assert_eq!(edited, id);
        assert!(shortcuts.is_pinned(&edited));
        shortcuts.save(&path).unwrap();
        let mut loaded = LauncherShortcuts::load(&path);
        assert_eq!(loaded, shortcuts);
        loaded.remove_shortcut(&edited);
        assert!(!loaded.is_pinned(&edited));
        assert!(loaded.is_pinned("app:editor"));
        let next = loaded
            .put_shortcut(None, "Two", "https://example.com/")
            .unwrap();
        assert_ne!(next, edited);
        assert_eq!(loaded.entries().len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }
}

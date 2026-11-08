//! Pure application ranking and bounded, local launch history.
use crate::platform::launcher::AppEntry;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

const MAX_RECENTS: usize = 32;
const MAX_HISTORY_BYTES: u64 = 64 * 1024;
const MAX_RESULTS: usize = 50;
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

impl RecentLaunches {
    pub fn load(path: &Path) -> Self {
        let load = || -> Option<Self> {
            let file = fs::File::open(path).ok()?;
            if file.metadata().ok()?.len() > MAX_HISTORY_BYTES {
                return None;
            }
            let mut bytes = Vec::new();
            file.take(MAX_HISTORY_BYTES + 1)
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() as u64 > MAX_HISTORY_BYTES {
                return None;
            }
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
        let parent = path
            .parent()
            .ok_or("Launch history has no parent directory")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let name = path
            .file_name()
            .ok_or("Launch history has no file name")?
            .to_string_lossy();
        let temporary = parent.join(format!(
            ".{name}.{}.{}.tmp",
            std::process::id(),
            SAVE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let bytes = serde_json::to_vec(&HistoryFile {
            version: 1,
            ids: self.ids.clone(),
        })
        .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_HISTORY_BYTES {
            return Err("Launch history exceeds its size limit".into());
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            drop(file);
            fs::rename(&temporary, path).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
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

/// Return catalog indices, without cloning targets or icons. Empty search lists
/// known recent apps first, followed by deterministic alphabetical suggestions.
/// Unknown history IDs are ignored; duplicate catalog identities appear once.
pub fn search(
    apps: &[AppEntry],
    query: &str,
    recents: &RecentLaunches,
    limit: usize,
) -> Vec<usize> {
    let query = normalized(query, 128);
    let mut seen = HashSet::new();
    let mut matches: Vec<_> = apps
        .iter()
        .enumerate()
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
            Some((score, recent, name, app.id.as_str(), index))
        })
        .collect();
    matches.sort_unstable();
    matches
        .into_iter()
        .take(limit.min(MAX_RESULTS))
        .map(|(_, _, _, _, index)| index)
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
}

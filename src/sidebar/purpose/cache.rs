//! GUI-owned results only: the temporary model conversation is never persisted.
use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

pub(super) const FILE: &str = "gui-thread-summaries.json";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(super) struct Entry {
    pub short: String,
    pub tooltip: String,
    pub fingerprint: String,
    pub max_chars: usize,
    /// Sticky across refresh, restart and every subsequent model response.
    pub manual_title: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub(super) struct Cache {
    pub version: u32,
    pub threads: BTreeMap<String, Entry>,
}
impl Default for Cache {
    fn default() -> Self {
        Self {
            version: 1,
            threads: BTreeMap::new(),
        }
    }
}
impl Cache {
    pub fn load(home: Option<&Path>) -> Self {
        let Some(home) = home else {
            return Self::default();
        };
        match std::fs::read(home.join(FILE)) {
            Ok(bytes) => match serde_json::from_slice::<Self>(&bytes) {
                Ok(cache) if cache.version == 1 => cache,
                _ => {
                    tracing::warn!("Ignoring incompatible GUI thread summary cache");
                    Self::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                tracing::warn!(%error, "Could not load GUI thread summaries");
                Self::default()
            }
        }
    }
    pub fn save(&self, home: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(home)?;
        // Preserve manual renames made by another concurrently running instance.
        let mut merged = self.clone();
        for (id, previous) in Self::load(Some(home)).threads {
            if previous.manual_title.is_some() {
                let entry = merged.threads.entry(id).or_default();
                if entry.manual_title.is_none() {
                    entry.manual_title = previous.manual_title;
                }
            }
        }
        let bytes = serde_json::to_vec_pretty(&merged).map_err(std::io::Error::other)?;
        let temporary = home.join(format!("{FILE}.{}.tmp", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let result = (|| {
            let mut file = options.open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, home.join(FILE))
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
}

/// Generated titles are ASCII and bounded by the measured representative glyph budget.
/// Defensive fallback keeps whole words where possible, without adding ellipses.
pub(super) fn fit_title(text: &str, maximum: usize) -> String {
    let cleaned = text.replace("...", "").replace('…', "");
    let plain: String = cleaned
        .chars()
        .filter(|c| (c.is_ascii_graphic() || *c == ' ') && !matches!(c, '`' | '*'))
        .collect();
    let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if plain.len() <= maximum {
        return plain;
    }
    let truncated = &plain[..maximum];
    truncated
        .rsplit_once(' ')
        .map_or(truncated, |(words, _)| words)
        .trim()
        .to_owned()
}

pub(super) fn plain_tooltip(text: &str) -> Option<String> {
    let plain = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let plain = plain
        .unicode_sentences()
        .take(2)
        .collect::<String>()
        .trim()
        .to_owned();
    (!plain.is_empty()
        && plain.chars().count() <= 500
        && !plain.contains(['`', '*'])
        && !plain.starts_with("# ")
        && !plain.starts_with("##"))
    .then_some(plain)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v8_restart_keeps_results_and_manual_name_without_model_context() {
        let home = tempfile::tempdir().unwrap();
        let mut cache = Cache::default();
        cache.threads.insert(
            "a".into(),
            Entry {
                short: "Fix auth".into(),
                tooltip: "Repair login and add coverage.".into(),
                fingerprint: "user-content".into(),
                max_chars: 15,
                manual_title: Some("My name".into()),
            },
        );
        cache.save(home.path()).unwrap();
        let reloaded = Cache::load(Some(home.path()));
        assert_eq!(
            reloaded.threads["a"].manual_title.as_deref(),
            Some("My name")
        );
        assert_eq!(
            reloaded.threads["a"].tooltip,
            "Repair login and add coverage."
        );
        let mut stale_instance = Cache::default();
        stale_instance.threads.insert(
            "a".into(),
            Entry {
                short: "Generated".into(),
                ..Entry::default()
            },
        );
        stale_instance.save(home.path()).unwrap();
        assert_eq!(
            Cache::load(Some(home.path())).threads["a"]
                .manual_title
                .as_deref(),
            Some("My name")
        );
        assert!(
            !std::fs::read_to_string(home.path().join(FILE))
                .unwrap()
                .contains("assistant")
        );
    }
    #[test]
    fn v8_labels_fit_without_ellipses_and_tooltips_are_plain_single_line() {
        assert_eq!(fit_title("Fix login and improve coverage", 12), "Fix login");
        assert_eq!(fit_title("abcdefghijk", 6), "abcdef");
        assert_eq!(fit_title("**Fix... auth…**", 20), "Fix auth");
        assert_eq!(fit_title("Fix auth", 1), "F");
        assert_eq!(
            plain_tooltip("Repair C# authentication."),
            Some("Repair C# authentication.".into())
        );
        assert_eq!(
            plain_tooltip("Fix login.\n\nAdd coverage."),
            Some("Fix login. Add coverage.".into())
        );
        assert_eq!(plain_tooltip("**Fancy**"), None);
        assert_eq!(plain_tooltip(""), None);
    }
}

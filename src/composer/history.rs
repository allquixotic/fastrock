//! Bounded, UTF-8-safe composer undo/redo persisted with each chat draft.
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

const MAX_EDITS: usize = 512;
const MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Edit {
    start: usize,
    removed: String,
    inserted: String,
    before: usize,
    after: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct History {
    undo: VecDeque<Edit>,
    redo: Vec<Edit>,
    pub cursor: usize,
}

fn cursor(text: &str, position: usize) -> usize {
    let mut position = position.min(text.len());
    while !text.is_char_boundary(position) {
        position -= 1;
    }
    position
}

impl History {
    pub fn observe(&mut self, old: &str, new: &str, position: usize) {
        let after = cursor(new, position);
        if old != new {
            let start = old
                .chars()
                .zip(new.chars())
                .take_while(|(a, b)| a == b)
                .map(|(c, _)| c.len_utf8())
                .sum::<usize>();
            let suffix = old[start..]
                .chars()
                .rev()
                .zip(new[start..].chars().rev())
                .take_while(|(a, b)| a == b)
                .map(|(c, _)| c.len_utf8())
                .sum::<usize>();
            self.undo.push_back(Edit {
                start,
                removed: old[start..old.len() - suffix].into(),
                inserted: new[start..new.len() - suffix].into(),
                before: cursor(old, self.cursor),
                after,
            });
            self.redo.clear();
            let mut bytes: usize = self
                .undo
                .iter()
                .map(|e| e.removed.len() + e.inserted.len())
                .sum();
            while self.undo.len() > MAX_EDITS || (bytes > MAX_BYTES && self.undo.len() > 1) {
                let edit = self.undo.pop_front().unwrap();
                bytes -= edit.removed.len() + edit.inserted.len();
            }
        }
        self.cursor = after;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn travel(&mut self, text: &mut String, redo: bool) -> bool {
        let edit = if redo {
            self.redo.pop()
        } else {
            self.undo.pop_back()
        };
        let Some(edit) = edit else { return false };
        let (expected, replacement, position) = if redo {
            (&edit.removed, &edit.inserted, edit.after)
        } else {
            (&edit.inserted, &edit.removed, edit.before)
        };
        let end = edit.start.saturating_add(expected.len());
        if text.get(edit.start..end) != Some(expected.as_str()) {
            // Old or damaged snapshots must never apply an edit to unrelated text.
            self.undo.clear();
            self.redo.clear();
            return false;
        }
        text.replace_range(edit.start..end, replacement);
        self.cursor = cursor(text, position);
        if redo {
            self.undo.push_back(edit);
        } else {
            self.redo.push(edit);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v108_unicode_undo_redo_survives_restart_and_branches() {
        let mut h = History::default();
        let mut text = String::new();
        for next in ["é 🔭", "é changed 🔭", "replacement\ntext", ""] {
            h.observe(&text, next, next.len());
            text = next.into();
        }
        let mut h: History = serde_json::from_slice(&serde_json::to_vec(&h).unwrap()).unwrap();
        for expected in ["replacement\ntext", "é changed 🔭", "é 🔭", ""] {
            assert!(h.travel(&mut text, false));
            assert_eq!(text, expected);
            assert!(text.is_char_boundary(h.cursor));
        }
        assert!(!h.travel(&mut text, false));
        assert!(h.travel(&mut text, true));
        assert_eq!(text, "é 🔭");
        let mut h: History = serde_json::from_slice(&serde_json::to_vec(&h).unwrap()).unwrap();
        assert!(h.travel(&mut text, true));
        assert_eq!(text, "é changed 🔭");
        h.observe(&text, "new branch", 10);
        assert!(!h.can_redo());
    }

    #[test]
    fn v108_history_is_bounded_and_mismatched_text_is_preserved() {
        let mut h = History::default();
        for i in 0..2000 {
            h.observe(&i.to_string(), &(i + 1).to_string(), 4);
        }
        assert_eq!(h.undo.len(), MAX_EDITS);
        let mut unrelated = "keep this".into();
        assert!(!h.travel(&mut unrelated, false));
        assert_eq!(unrelated, "keep this");
        assert!(!h.can_undo());
    }
}

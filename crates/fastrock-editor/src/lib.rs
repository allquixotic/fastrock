#![forbid(unsafe_code)]

use std::collections::{BTreeMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};

use ropey::Rope;
use thiserror::Error;
use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorBuffer {
    pub path: String,
    pub dirty: bool,
    pub read_only: bool,
    pub version: u64,
    pub encoding: TextEncoding,
    pub language: EditorLanguage,
    rope: Rope,
    clean_hash: u64,
    clean_snapshot: Option<String>,
    undo_stack: Vec<EditorEdit>,
    redo_stack: Vec<EditorEdit>,
}

impl EditorBuffer {
    pub fn from_text(path: impl Into<String>, text: &str) -> Self {
        let path = path.into();
        let language = EditorLanguage::detect(&path);
        let rope = Rope::from_str(text);
        let clean_hash = hash_rope(&rope);
        Self {
            path,
            dirty: false,
            read_only: false,
            version: 0,
            encoding: TextEncoding::Utf8,
            language,
            rope,
            clean_hash,
            clean_snapshot: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    pub fn read_only_from_text(path: impl Into<String>, text: &str) -> Self {
        let mut buffer = Self::from_text(path, text);
        buffer.read_only = true;
        buffer
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, EditorError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(EditorError::Io)?;
        Ok(Self::from_bytes(path, &bytes))
    }

    pub async fn open_chunked(
        path: impl AsRef<Path>,
        chunk_size: usize,
    ) -> Result<Self, EditorError> {
        if chunk_size == 0 {
            return Err(EditorError::InvalidChunkSize);
        }
        let path = path.as_ref();
        let mut file = tokio::fs::File::open(path).await.map_err(EditorError::Io)?;
        let mut bytes = Vec::new();
        let mut chunk = vec![0; chunk_size];
        loop {
            let bytes_read = tokio::io::AsyncReadExt::read(&mut file, &mut chunk)
                .await
                .map_err(EditorError::Io)?;
            if bytes_read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..bytes_read]);
        }
        Ok(Self::from_bytes(path, &bytes))
    }

    fn from_bytes(path: &Path, bytes: &[u8]) -> Self {
        let (encoding, text) = decode_text(bytes);
        let rope = Rope::from_str(&text);
        let clean_hash = hash_rope(&rope);
        Self {
            path: path.to_string_lossy().into_owned(),
            dirty: false,
            read_only: false,
            version: 0,
            encoding,
            language: EditorLanguage::detect_path(path),
            rope,
            clean_hash,
            clean_snapshot: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn char_count(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn visible_lines(&self, start_line: usize, line_count: usize) -> Vec<EditorLine> {
        self.visible_viewport(start_line, line_count).lines
    }

    pub fn visible_viewport(&self, start_line: usize, line_count: usize) -> EditorViewport {
        let window = self.visible_line_window(start_line, line_count);
        let total_line_count = self.rope.len_lines();
        let mut tree_sitter_highlights =
            self.tree_sitter_highlights(&window.source, &window.line_byte_offsets);
        let lines = window
            .lines
            .into_iter()
            .enumerate()
            .map(|(relative_line_index, text)| {
                let line_start = window.line_byte_offsets[relative_line_index];
                let line_end = window.line_byte_offsets[relative_line_index + 1];
                let highlights = tree_sitter_highlights
                    .remove(&relative_line_index)
                    .unwrap_or_else(|| {
                        highlight_visible_line_fallback(&text)
                            .into_iter()
                            .map(|mut highlight| {
                                highlight.source = SyntaxHighlightSource::Fallback;
                                highlight
                            })
                            .collect()
                    })
                    .into_iter()
                    .filter(|highlight| {
                        line_start + highlight.start_byte < line_end
                            && line_start + highlight.end_byte <= line_end
                    })
                    .collect();
                EditorLine {
                    line_index: window.start_line + relative_line_index,
                    highlights,
                    text,
                }
            })
            .collect();

        EditorViewport {
            lines,
            metrics: EditorViewportMetrics {
                requested_start_line: start_line,
                requested_line_count: line_count,
                rendered_start_line: window.start_line,
                rendered_line_count: window.end_line.saturating_sub(window.start_line),
                rendered_bytes: window.source_len,
                total_line_count,
            },
        }
    }

    fn visible_line_window(&self, start_line: usize, line_count: usize) -> VisibleLineWindow {
        let total_lines = self.rope.len_lines();
        let start_line = start_line.min(total_lines);
        let end_line = start_line.saturating_add(line_count).min(total_lines);
        let mut lines = Vec::with_capacity(end_line.saturating_sub(start_line));
        let mut source = String::new();
        let mut line_byte_offsets = Vec::with_capacity(lines.capacity() + 1);
        line_byte_offsets.push(0);
        for line_index in start_line..end_line {
            let line = self.rope.line(line_index).to_string();
            source.push_str(&line);
            line_byte_offsets.push(source.len());
            lines.push(line);
        }
        let source_len = source.len();
        VisibleLineWindow {
            start_line,
            end_line,
            lines,
            source,
            source_len,
            line_byte_offsets,
        }
    }

    fn tree_sitter_highlights(
        &self,
        visible_source: &str,
        line_byte_offsets: &[usize],
    ) -> std::collections::BTreeMap<usize, Vec<SyntaxHighlight>> {
        if self.language != EditorLanguage::Rust || visible_source.is_empty() {
            return std::collections::BTreeMap::new();
        }
        let mut parser = Parser::new();
        let language = tree_sitter::Language::new(tree_sitter_rust::LANGUAGE);
        if parser.set_language(&language).is_err() {
            return std::collections::BTreeMap::new();
        }
        let prefix = "fn __fastrock_viewport_context__() {\n";
        let suffix = "\n}\n";
        let visible_byte_start = prefix.len();
        let visible_byte_end = visible_byte_start + visible_source.len();
        let mut parse_source =
            String::with_capacity(prefix.len() + visible_source.len() + suffix.len());
        parse_source.push_str(prefix);
        parse_source.push_str(visible_source);
        parse_source.push_str(suffix);
        let Some(tree) = parser.parse(&parse_source, None) else {
            return std::collections::BTreeMap::new();
        };
        let mut highlights = Vec::new();
        collect_tree_sitter_highlights(
            tree.root_node(),
            visible_byte_start,
            visible_byte_end,
            &mut highlights,
        );
        let mut by_line = std::collections::BTreeMap::<usize, Vec<SyntaxHighlight>>::new();
        for highlight in highlights {
            let local_start = highlight.start_byte - visible_byte_start;
            let local_end = highlight.end_byte - visible_byte_start;
            let line_index = line_index_for_byte(line_byte_offsets, local_start);
            let line_start = line_byte_offsets[line_index];
            by_line
                .entry(line_index)
                .or_default()
                .push(SyntaxHighlight {
                    start_byte: local_start - line_start,
                    end_byte: local_end - line_start,
                    kind: highlight.kind,
                    source: SyntaxHighlightSource::TreeSitter,
                });
        }
        for line_highlights in by_line.values_mut() {
            line_highlights.sort_by_key(|highlight| highlight.start_byte);
        }
        by_line
    }

    pub fn replace_range(
        &mut self,
        start_char: usize,
        end_char: usize,
        replacement: &str,
    ) -> Result<(), EditorError> {
        if self.read_only {
            return Err(EditorError::ReadOnly);
        }
        let removed = self.apply_replacement(start_char, end_char, replacement);
        self.undo_stack.push(EditorEdit {
            start_char,
            removed,
            inserted: replacement.to_owned(),
        });
        self.redo_stack.clear();
        Ok(())
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo(&mut self) -> Result<bool, EditorError> {
        if self.read_only {
            return Err(EditorError::ReadOnly);
        }
        let Some(edit) = self.undo_stack.pop() else {
            return Ok(false);
        };
        let inserted_chars = edit.inserted.chars().count();
        self.apply_replacement(
            edit.start_char,
            edit.start_char + inserted_chars,
            &edit.removed,
        );
        self.redo_stack.push(edit);
        Ok(true)
    }

    pub fn redo(&mut self) -> Result<bool, EditorError> {
        if self.read_only {
            return Err(EditorError::ReadOnly);
        }
        let Some(edit) = self.redo_stack.pop() else {
            return Ok(false);
        };
        let removed_chars = edit.removed.chars().count();
        self.apply_replacement(
            edit.start_char,
            edit.start_char + removed_chars,
            &edit.inserted,
        );
        self.undo_stack.push(edit);
        Ok(true)
    }

    fn apply_replacement(
        &mut self,
        start_char: usize,
        end_char: usize,
        replacement: &str,
    ) -> String {
        if !self.dirty && self.clean_snapshot.is_none() {
            self.clean_snapshot = Some(self.rope.to_string());
        }
        let removed = self.rope.slice(start_char..end_char).to_string();
        self.rope.remove(start_char..end_char);
        self.rope.insert(start_char, replacement);
        self.version += 1;
        self.dirty = hash_rope(&self.rope) != self.clean_hash;
        if !self.dirty {
            self.clean_snapshot = None;
        }
        removed
    }

    pub fn find_all(&self, needle: &str) -> Vec<TextMatch> {
        if needle.is_empty() {
            return Vec::new();
        }
        let text = self.rope.to_string();
        text.match_indices(needle)
            .map(|(char_start, _)| TextMatch {
                start_char: char_start,
                end_char: char_start + needle.len(),
            })
            .collect()
    }

    pub fn save(&mut self) -> Result<(), EditorError> {
        self.save_with_conflict_policy(false)
    }

    pub fn mark_clean_after_external_save(&mut self) {
        self.clean_hash = hash_rope(&self.rope);
        self.clean_snapshot = None;
        self.dirty = false;
    }

    pub fn force_save(&mut self) -> Result<(), EditorError> {
        self.save_with_conflict_policy(true)
    }

    fn save_with_conflict_policy(&mut self, allow_conflict: bool) -> Result<(), EditorError> {
        if self.read_only {
            return Err(EditorError::ReadOnly);
        }
        let path = Path::new(&self.path);
        if self.dirty && !allow_conflict {
            self.ensure_disk_unchanged(path)?;
        }
        let current_text = self.rope.to_string();
        atomic_write(path, current_text.as_bytes())?;
        self.clean_hash = hash_rope(&self.rope);
        self.clean_snapshot = None;
        self.dirty = false;
        Ok(())
    }

    fn ensure_disk_unchanged(&self, path: &Path) -> Result<(), EditorError> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(EditorError::Io(error)),
        };
        let (_, text) = decode_text(&bytes);
        let disk_hash = hash_rope(&Rope::from_str(&text));
        if disk_hash == self.clean_hash {
            Ok(())
        } else {
            Err(EditorError::FileConflict {
                path: self.path.clone(),
            })
        }
    }

    pub fn revert(&mut self) -> Result<(), EditorError> {
        if self.read_only {
            return Err(EditorError::ReadOnly);
        }
        let path = Path::new(&self.path);
        let bytes = std::fs::read(path).map_err(EditorError::Io)?;
        let mut reverted = Self::from_bytes(path, &bytes);
        reverted.version = self.version + 1;
        *self = reverted;
        Ok(())
    }

    pub fn revert_to_last_clean(&mut self) -> Result<(), EditorError> {
        if self.read_only {
            return Err(EditorError::ReadOnly);
        }
        let clean_text = self
            .clean_snapshot
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| self.rope.to_string());
        self.rope = Rope::from_str(&clean_text);
        self.version += 1;
        self.dirty = false;
        self.clean_snapshot = None;
        self.undo_stack.clear();
        self.redo_stack.clear();
        Ok(())
    }

    pub fn go_to_line(&self, line_number: usize) -> Option<EditorCursor> {
        if line_number == 0 {
            return None;
        }
        let line_index = (line_number - 1).min(self.rope.len_lines().saturating_sub(1));
        let char_index = self.rope.line_to_char(line_index);
        Some(EditorCursor {
            line_index,
            line_number: line_index + 1,
            char_index,
        })
    }

    pub fn diff_against(&self, other: &EditorBuffer) -> Vec<LineDiff> {
        line_diff(&self.rope, &other.rope)
    }

    pub fn diff_against_clean(&self) -> Vec<LineDiff> {
        let Some(clean_snapshot) = &self.clean_snapshot else {
            return Vec::new();
        };
        let clean_rope = Rope::from_str(clean_snapshot);
        line_diff(&clean_rope, &self.rope)
    }
}

fn line_diff(left_rope: &Rope, right_rope: &Rope) -> Vec<LineDiff> {
    let max_lines = left_rope.len_lines().max(right_rope.len_lines());
    (0..max_lines)
        .filter_map(|line_index| {
            let left = left_rope
                .get_line(line_index)
                .map(|line| line.to_string())
                .unwrap_or_default();
            let right = right_rope
                .get_line(line_index)
                .map(|line| line.to_string())
                .unwrap_or_default();
            (left != right).then_some(LineDiff {
                line_index,
                left,
                right,
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorEdit {
    start_char: usize,
    removed: String,
    inserted: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorLanguage {
    Rust,
    PlainText,
}

impl EditorLanguage {
    fn detect(path: &str) -> Self {
        Self::detect_path(Path::new(path))
    }

    fn detect_path(path: &Path) -> Self {
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("rs") => Self::Rust,
            _ => Self::PlainText,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorLine {
    pub line_index: usize,
    pub text: String,
    pub highlights: Vec<SyntaxHighlight>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorViewport {
    pub lines: Vec<EditorLine>,
    pub metrics: EditorViewportMetrics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorViewportMetrics {
    pub requested_start_line: usize,
    pub requested_line_count: usize,
    pub rendered_start_line: usize,
    pub rendered_line_count: usize,
    pub rendered_bytes: usize,
    pub total_line_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VisibleLineWindow {
    start_line: usize,
    end_line: usize,
    lines: Vec<String>,
    source: String,
    source_len: usize,
    line_byte_offsets: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxHighlight {
    pub start_byte: usize,
    pub end_byte: usize,
    pub kind: SyntaxKind,
    pub source: SyntaxHighlightSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyntaxKind {
    Keyword,
    String,
    Comment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxHighlightSource {
    TreeSitter,
    Fallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMatch {
    pub start_char: usize,
    pub end_char: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineDiff {
    pub line_index: usize,
    pub left: String,
    pub right: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCursor {
    pub line_index: usize,
    pub line_number: usize,
    pub char_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EditorCommand {
    Save,
    Revert,
    Find,
    GoToLine,
    Undo,
    Redo,
    CloseTab,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorShortcutMap {
    bindings: BTreeMap<String, EditorCommand>,
}

impl EditorShortcutMap {
    pub fn bind(
        &mut self,
        shortcut: impl Into<String>,
        command: EditorCommand,
    ) -> Result<(), EditorError> {
        let shortcut = normalize_shortcut(&shortcut.into());
        if shortcut.is_empty() {
            return Err(EditorError::InvalidShortcut);
        }
        self.bindings.insert(shortcut, command);
        Ok(())
    }

    pub fn command_for(&self, shortcut: &str) -> Option<EditorCommand> {
        let shortcut = normalize_shortcut(shortcut);
        self.bindings.get(&shortcut).copied()
    }

    pub fn bindings(&self) -> &BTreeMap<String, EditorCommand> {
        &self.bindings
    }
}

impl Default for EditorShortcutMap {
    fn default() -> Self {
        let mut bindings = BTreeMap::new();
        bindings.insert("mod+s".to_owned(), EditorCommand::Save);
        bindings.insert("mod+f".to_owned(), EditorCommand::Find);
        bindings.insert("mod+g".to_owned(), EditorCommand::GoToLine);
        bindings.insert("mod+z".to_owned(), EditorCommand::Undo);
        bindings.insert("mod+shift+z".to_owned(), EditorCommand::Redo);
        bindings.insert("mod+w".to_owned(), EditorCommand::CloseTab);
        bindings.insert("mod+r".to_owned(), EditorCommand::Revert);
        Self { bindings }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EditorTabSet {
    tabs: Vec<EditorBuffer>,
    active_path: Option<String>,
}

impl EditorTabSet {
    pub fn open_tab(&mut self, buffer: EditorBuffer) {
        self.active_path = Some(buffer.path.clone());
        if let Some(existing) = self.tabs.iter_mut().find(|tab| tab.path == buffer.path) {
            *existing = buffer;
        } else {
            self.tabs.push(buffer);
        }
    }

    pub fn close_tab(&mut self, path: &str) -> bool {
        let len_before = self.tabs.len();
        self.tabs.retain(|tab| tab.path != path);
        if self.active_path.as_deref() == Some(path) {
            self.active_path = self.tabs.last().map(|tab| tab.path.clone());
        }
        self.tabs.len() != len_before
    }

    pub fn activate_tab(&mut self, path: &str) -> bool {
        if self.tabs.iter().any(|tab| tab.path == path) {
            self.active_path = Some(path.to_owned());
            true
        } else {
            false
        }
    }

    pub fn active(&self) -> Option<&EditorBuffer> {
        let active_path = self.active_path.as_deref()?;
        self.tabs.iter().find(|tab| tab.path == active_path)
    }

    pub fn active_mut(&mut self) -> Option<&mut EditorBuffer> {
        let active_path = self.active_path.as_deref()?;
        self.tabs.iter_mut().find(|tab| tab.path == active_path)
    }

    pub fn active_path(&self) -> Option<&str> {
        self.active_path.as_deref()
    }

    pub fn tabs(&self) -> &[EditorBuffer] {
        &self.tabs
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }
}

#[derive(Debug, Error)]
pub enum EditorError {
    #[error("editor buffer is read-only")]
    ReadOnly,
    #[error("editor chunk size must be greater than zero")]
    InvalidChunkSize,
    #[error("editor shortcut is invalid")]
    InvalidShortcut,
    #[error("editor file changed on disk before save: {path}")]
    FileConflict { path: String },
    #[error("editor I/O error: {0}")]
    Io(std::io::Error),
}

fn normalize_shortcut(shortcut: &str) -> String {
    shortcut
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("+")
}

fn decode_text(bytes: &[u8]) -> (TextEncoding, String) {
    if let Some(stripped) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return (
            TextEncoding::Utf8Bom,
            String::from_utf8_lossy(stripped).into_owned(),
        );
    }
    (
        TextEncoding::Utf8,
        String::from_utf8_lossy(bytes).into_owned(),
    )
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), EditorError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let mut temp_path = parent
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let file_name = path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .unwrap_or("buffer");
    temp_path.push(format!(".{file_name}.fastrock-tmp-{}", std::process::id()));
    {
        let mut file = std::fs::File::create(&temp_path).map_err(EditorError::Io)?;
        file.write_all(bytes).map_err(EditorError::Io)?;
        file.sync_all().map_err(EditorError::Io)?;
    }
    std::fs::rename(&temp_path, path).map_err(|error| {
        let _ = std::fs::remove_file(&temp_path);
        EditorError::Io(error)
    })?;
    Ok(())
}

fn hash_rope(rope: &Rope) -> u64 {
    let mut hasher = DefaultHasher::new();
    for chunk in rope.chunks() {
        chunk.hash(&mut hasher);
    }
    hasher.finish()
}

fn highlight_visible_line_fallback(line: &str) -> Vec<SyntaxHighlight> {
    let mut highlights = Vec::new();
    if let Some(index) = line.find("//") {
        highlights.push(SyntaxHighlight {
            start_byte: index,
            end_byte: line.len(),
            kind: SyntaxKind::Comment,
            source: SyntaxHighlightSource::Fallback,
        });
    }
    let mut in_string = false;
    let mut start = 0;
    for (index, byte) in line.bytes().enumerate() {
        if byte == b'"' {
            if in_string {
                highlights.push(SyntaxHighlight {
                    start_byte: start,
                    end_byte: index + 1,
                    kind: SyntaxKind::String,
                    source: SyntaxHighlightSource::Fallback,
                });
            } else {
                start = index;
            }
            in_string = !in_string;
        }
    }
    for keyword in ["fn", "let", "struct", "enum", "impl"] {
        if let Some(index) = line.find(keyword) {
            highlights.push(SyntaxHighlight {
                start_byte: index,
                end_byte: index + keyword.len(),
                kind: SyntaxKind::Keyword,
                source: SyntaxHighlightSource::Fallback,
            });
        }
    }
    highlights.sort_by_key(|highlight| highlight.start_byte);
    highlights
}

fn collect_tree_sitter_highlights(
    node: Node<'_>,
    visible_byte_start: usize,
    visible_byte_end: usize,
    highlights: &mut Vec<SyntaxHighlight>,
) {
    if node.end_byte() <= visible_byte_start || node.start_byte() >= visible_byte_end {
        return;
    }

    if let Some(kind) = syntax_kind_for_tree_sitter_node(node.kind()) {
        let start_byte = node.start_byte().max(visible_byte_start);
        let end_byte = node.end_byte().min(visible_byte_end);
        if start_byte < end_byte {
            highlights.push(SyntaxHighlight {
                start_byte,
                end_byte,
                kind,
                source: SyntaxHighlightSource::TreeSitter,
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_tree_sitter_highlights(child, visible_byte_start, visible_byte_end, highlights);
    }
}

fn syntax_kind_for_tree_sitter_node(kind: &str) -> Option<SyntaxKind> {
    match kind {
        "fn" | "let" | "struct" | "enum" | "impl" | "pub" | "use" | "mod" | "trait" | "async"
        | "await" | "match" | "if" | "else" | "for" | "while" | "loop" | "return" | "const"
        | "static" | "mut" | "where" | "unsafe" => Some(SyntaxKind::Keyword),
        "string_literal" | "raw_string_literal" | "char_literal" => Some(SyntaxKind::String),
        "line_comment" | "block_comment" => Some(SyntaxKind::Comment),
        _ => None,
    }
}

fn line_index_for_byte(line_offsets: &[usize], byte_index: usize) -> usize {
    match line_offsets.binary_search(&byte_index) {
        Ok(index) => index.min(line_offsets.len().saturating_sub(2)),
        Err(index) => index
            .saturating_sub(1)
            .min(line_offsets.len().saturating_sub(2)),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn v13_visible_lines_render_only_requested_range() {
        let text = (0..10)
            .map(|index| format!("line {index}\n"))
            .collect::<String>();
        let buffer = EditorBuffer::from_text("test.rs", &text);

        let viewport = buffer.visible_viewport(3, 2);
        let visible = viewport.lines;

        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].line_index, 3);
        assert_eq!(visible[0].text, "line 3\n");
        assert_eq!(visible[1].line_index, 4);
        assert_eq!(
            viewport.metrics,
            EditorViewportMetrics {
                requested_start_line: 3,
                requested_line_count: 2,
                rendered_start_line: 3,
                rendered_line_count: 2,
                rendered_bytes: "line 3\nline 4\n".len(),
                total_line_count: 11,
            }
        );
    }

    #[test]
    fn v13_large_viewport_materializes_requested_bytes_only() {
        let text = (0..10_000)
            .map(|index| format!("line {index:05} hidden payload that should stay outside view\n"))
            .collect::<String>();
        let buffer = EditorBuffer::from_text("large.txt", &text);

        let viewport = buffer.visible_viewport(9_000, 20);

        assert_eq!(viewport.lines.len(), 20);
        assert_eq!(viewport.lines[0].line_index, 9_000);
        assert_eq!(viewport.metrics.rendered_line_count, 20);
        assert!(viewport.metrics.rendered_bytes < text.len() / 100);
        assert!(
            viewport
                .lines
                .iter()
                .all(|line| line.text.contains("hidden payload"))
        );
    }

    #[test]
    fn t22_editor_find_replace_diff_and_tabs() {
        let mut left = EditorBuffer::from_text("left.rs", "fn main() {\nlet x = \"a\";\n}\n");
        let right = EditorBuffer::from_text("right.rs", "fn main() {\nlet x = \"b\";\n}\n");

        assert_eq!(
            left.find_all("let"),
            vec![TextMatch {
                start_char: 12,
                end_char: 15,
            }]
        );
        left.replace_range(21, 22, "b").unwrap();
        assert!(left.dirty);
        assert_eq!(left.diff_against(&right), Vec::new());

        let mut tabs = EditorTabSet::default();
        tabs.open_tab(left);
        tabs.open_tab(right);
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.active().unwrap().path, "right.rs");
        assert!(tabs.close_tab("right.rs"));
        assert_eq!(tabs.active().unwrap().path, "left.rs");
        assert!(tabs.activate_tab("left.rs"));
        assert!(!tabs.activate_tab("missing.rs"));
    }

    #[test]
    fn t22_highlight_operates_on_visible_line() {
        let buffer = EditorBuffer::from_text("test.rs", "fn main() { let s = \"x\"; } // c\n");
        let line = buffer.visible_lines(0, 1).remove(0);

        assert!(
            line.highlights
                .iter()
                .any(|highlight| highlight.kind == SyntaxKind::Keyword)
        );
        assert!(
            line.highlights
                .iter()
                .any(|highlight| highlight.kind == SyntaxKind::String)
        );
        assert!(
            line.highlights
                .iter()
                .any(|highlight| highlight.kind == SyntaxKind::Comment)
        );
    }

    #[test]
    fn t22_rust_highlight_uses_tree_sitter_for_visible_ranges() {
        let buffer = EditorBuffer::from_text(
            "test.rs",
            "fn main() {\nlet s = \"visible\"; // visible\n}\nfn hidden() {}\n",
        );

        let visible = buffer.visible_lines(1, 1);
        let line = &visible[0];

        assert_eq!(buffer.language, EditorLanguage::Rust);
        assert_eq!(line.line_index, 1);
        assert!(
            line.highlights
                .iter()
                .all(|highlight| highlight.source == SyntaxHighlightSource::TreeSitter)
        );
        assert!(
            line.highlights
                .iter()
                .any(|highlight| highlight.kind == SyntaxKind::String)
        );
        assert!(
            line.highlights
                .iter()
                .any(|highlight| highlight.kind == SyntaxKind::Comment)
        );
    }

    #[test]
    fn t22_plain_text_highlight_degrades_to_fallback() {
        let buffer = EditorBuffer::from_text("notes.txt", "let text = \"not rust\" // note\n");
        let line = buffer.visible_lines(0, 1).remove(0);

        assert_eq!(buffer.language, EditorLanguage::PlainText);
        assert!(
            line.highlights
                .iter()
                .all(|highlight| highlight.source == SyntaxHighlightSource::Fallback)
        );
    }

    #[test]
    fn t22_save_writes_buffer_to_disk() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().join("file.txt");
        std::fs::write(&path, "hello").unwrap();
        let mut buffer = EditorBuffer::open(&path).unwrap();

        buffer.replace_range(0, 5, "world").unwrap();
        buffer.save().unwrap();

        assert_eq!(std::fs::read_to_string(path).unwrap(), "world");
        assert!(!buffer.dirty);
    }

    #[test]
    fn t22_save_detects_external_file_conflict_before_overwrite() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().join("file.txt");
        std::fs::write(&path, "hello").unwrap();
        let mut buffer = EditorBuffer::open(&path).unwrap();

        buffer.replace_range(0, 5, "world").unwrap();
        std::fs::write(&path, "external").unwrap();

        assert!(matches!(
            buffer.save(),
            Err(EditorError::FileConflict { path: conflict_path }) if conflict_path.ends_with("file.txt")
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        assert!(buffer.dirty);

        buffer.force_save().unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "world");
        assert!(!buffer.dirty);
    }

    #[test]
    fn t22_read_only_buffers_block_mutation_for_remote_or_generated_views() {
        let mut buffer =
            EditorBuffer::read_only_from_text("remote:/repo/file.rs", "fn main() {}\n");

        assert!(buffer.read_only);
        assert!(matches!(
            buffer.replace_range(0, 2, "pub fn"),
            Err(EditorError::ReadOnly)
        ));
        assert!(matches!(buffer.undo(), Err(EditorError::ReadOnly)));
        assert!(matches!(buffer.redo(), Err(EditorError::ReadOnly)));
        assert!(matches!(buffer.save(), Err(EditorError::ReadOnly)));
        assert_eq!(buffer.text(), "fn main() {}\n");
    }

    #[test]
    fn t22_go_to_line_returns_clamped_cursor_positions() {
        let buffer = EditorBuffer::from_text("main.rs", "one\ntwo\nthree\n");

        assert_eq!(buffer.go_to_line(0), None);
        assert_eq!(
            buffer.go_to_line(2),
            Some(EditorCursor {
                line_index: 1,
                line_number: 2,
                char_index: 4,
            })
        );
        assert_eq!(
            buffer.go_to_line(999),
            Some(EditorCursor {
                line_index: 3,
                line_number: 4,
                char_index: 14,
            })
        );
    }

    #[test]
    fn t22_revert_restores_file_contents_and_clears_dirty_state() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().join("file.txt");
        std::fs::write(&path, "disk").unwrap();
        let mut buffer = EditorBuffer::open(&path).unwrap();

        buffer.replace_range(0, 4, "memory").unwrap();
        assert!(buffer.dirty);
        buffer.revert().unwrap();

        assert_eq!(buffer.text(), "disk");
        assert!(!buffer.dirty);
        assert!(!buffer.can_undo());
        assert!(!buffer.can_redo());
    }

    #[test]
    fn t22_in_memory_revert_and_clean_diff_do_not_require_disk_read() {
        let mut buffer = EditorBuffer::from_text("scratch.txt", "alpha\n");

        buffer
            .replace_range(buffer.char_count(), buffer.char_count(), "beta")
            .unwrap();
        assert_eq!(buffer.diff_against_clean().len(), 1);
        assert_eq!(buffer.diff_against_clean()[0].line_index, 1);
        assert_eq!(buffer.diff_against_clean()[0].left, "");
        assert_eq!(buffer.diff_against_clean()[0].right, "beta");

        buffer.revert_to_last_clean().unwrap();

        assert_eq!(buffer.text(), "alpha\n");
        assert!(!buffer.dirty);
        assert!(buffer.diff_against_clean().is_empty());
        assert!(!buffer.can_undo());
        assert!(!buffer.can_redo());
    }

    #[test]
    fn t22_keyboard_shortcuts_are_configurable() {
        let mut shortcuts = EditorShortcutMap::default();

        assert_eq!(shortcuts.command_for("MOD+S"), Some(EditorCommand::Save));
        assert_eq!(
            shortcuts.command_for("mod+shift+z"),
            Some(EditorCommand::Redo)
        );
        shortcuts
            .bind("ctrl+alt+g", EditorCommand::GoToLine)
            .unwrap();
        assert_eq!(
            shortcuts.command_for("CTRL + ALT + G"),
            Some(EditorCommand::GoToLine)
        );
        assert!(matches!(
            shortcuts.bind(" + ", EditorCommand::Find),
            Err(EditorError::InvalidShortcut)
        ));
    }

    #[test]
    fn t22_undo_redo_restores_content_and_dirty_state() {
        let mut buffer = EditorBuffer::from_text("note.txt", "hello");

        buffer.replace_range(0, 5, "world").unwrap();

        assert_eq!(buffer.text(), "world");
        assert!(buffer.dirty);
        assert!(buffer.can_undo());
        assert!(!buffer.can_redo());

        assert!(buffer.undo().unwrap());
        assert_eq!(buffer.text(), "hello");
        assert!(!buffer.dirty);
        assert!(!buffer.can_undo());
        assert!(buffer.can_redo());

        assert!(buffer.redo().unwrap());
        assert_eq!(buffer.text(), "world");
        assert!(buffer.dirty);
    }

    #[test]
    fn t22_new_edit_after_undo_clears_redo_stack() {
        let mut buffer = EditorBuffer::from_text("note.txt", "abc");

        buffer.replace_range(0, 1, "x").unwrap();
        assert!(buffer.undo().unwrap());
        buffer.replace_range(2, 3, "z").unwrap();

        assert_eq!(buffer.text(), "abz");
        assert!(!buffer.can_redo());
        assert_eq!(buffer.version, 3);
    }

    #[test]
    fn t22_undo_after_save_marks_buffer_dirty_again() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().join("file.txt");
        std::fs::write(&path, "before").unwrap();
        let mut buffer = EditorBuffer::open(&path).unwrap();

        buffer.replace_range(0, 6, "after").unwrap();
        buffer.save().unwrap();
        assert!(!buffer.dirty);

        assert!(buffer.undo().unwrap());

        assert_eq!(buffer.text(), "before");
        assert!(buffer.dirty);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "after");
    }

    #[tokio::test]
    async fn t22_async_chunked_open_loads_large_file_without_sync_reader() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().join("large.rs");
        let text = (0..512)
            .map(|index| format!("fn f_{index}() {{}}\n"))
            .collect::<String>();
        std::fs::write(&path, &text).unwrap();

        let buffer = EditorBuffer::open_chunked(&path, 17).await.unwrap();

        assert_eq!(buffer.language, EditorLanguage::Rust);
        assert_eq!(buffer.char_count(), text.chars().count());
        assert_eq!(
            EditorBuffer::open_chunked(&path, 0)
                .await
                .unwrap_err()
                .to_string(),
            "editor chunk size must be greater than zero"
        );
    }

    #[test]
    fn t22_save_is_atomic_and_removes_temp_file() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().join("atomic.txt");
        std::fs::write(&path, "old").unwrap();
        let mut buffer = EditorBuffer::open(&path).unwrap();

        buffer.replace_range(0, 3, "new").unwrap();
        buffer.save().unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(
            std::fs::read_dir(temp_dir.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().contains("fastrock-tmp"))
                .count(),
            0
        );
    }
}

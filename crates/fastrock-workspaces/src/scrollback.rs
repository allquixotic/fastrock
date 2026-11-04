//! Lossless, indexed cold transcript pages; call from a background runtime.
//!
//! Completed content is stored in a bounded-cache Fjall database with LZ4
//! configured in the engine. Indexed pages and a decoded LRU bound residency;
//! Fastrock performs no compression or decompression itself.
use anyhow::Result;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    ops::Range,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

const PAGE_ITEMS: usize = 64;
const PAGE_BYTES: usize = 128 * 1024;
const CACHE_BYTES: usize = 8 * 1024 * 1024;
const CACHE_PAGES: usize = 16;

#[derive(serde::Serialize)]
pub struct Window {
    pub start: usize,
    pub total: usize,
    pub items: Vec<Value>,
    pub turns: Vec<Value>,
}

// Shared across histories, bounded independently of the decoded viewport cache.
// Creation, reads and writes happen on the existing background runtime.
struct ColdStore {
    pages: fjall::Keyspace,
    _db: fjall::Database,
    _lease: std::fs::File,
    _directory: tempfile::TempDir,
}
static COLD: OnceLock<Mutex<Weak<ColdStore>>> = OnceLock::new();
static PAGE_ID: AtomicU64 = AtomicU64::new(1);
impl ColdStore {
    fn open() -> Result<Arc<Self>> {
        let mut shared = COLD
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(store) = shared.upgrade() {
            return Ok(store);
        }
        cleanup_cold_stores();
        let directory = tempfile::Builder::new()
            .prefix("fastrock-scrollback-")
            .tempdir()?;
        let lease = std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(directory.path().join("owner"))?;
        lease.try_lock()?;
        std::io::Write::write_all(&mut &lease, b"fastrock-transcript-cache-v1")?;
        let db = fjall::Database::builder(directory.path().join("data"))
            .manual_journal_persist(true)
            .cache_size(16 * 1024 * 1024)
            .max_journaling_size(64 * 1024 * 1024)
            .worker_threads(1)
            .journal_compression(fjall::CompressionType::Lz4)
            .open()?;
        let pages = db.keyspace("pages", || {
            fjall::KeyspaceCreateOptions::default()
                .manual_journal_persist(true)
                .max_memtable_size(4 * 1024 * 1024)
                .data_block_compression_policy(fjall::config::CompressionPolicy::all(
                    fjall::CompressionType::Lz4,
                ))
        })?;
        let store = Arc::new(Self {
            pages,
            _db: db,
            _lease: lease,
            _directory: directory,
        });
        *shared = Arc::downgrade(&store);
        Ok(store)
    }
}
/// Crash leftovers are disposable cache only. Bound enumeration and skip live
/// owners, recent directories, symlinks and anything without our exact marker.
fn cleanup_cold_stores() {
    use std::io::Read;
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.take(4096).flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("fastrock-scrollback-")
        {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            continue;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                continue;
            }
        }
        if meta
            .modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_none_or(|d| d.as_secs() <= 86400)
        {
            continue;
        }
        let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(entry.path().join("owner"))
        else {
            continue;
        };
        if file.try_lock().is_err() {
            continue;
        }
        let mut marker = Vec::new();
        if (&file).take(64).read_to_end(&mut marker).is_err()
            || marker != b"fastrock-transcript-cache-v1"
        {
            continue;
        }
        let _ = std::fs::remove_dir_all(entry.path());
    }
}
enum Encoded {
    Stored { store: Arc<ColdStore>, key: [u8; 8] },
    // Storage failure must never discard conversation content. Retain raw data
    // in memory; authoritative conversation history remains with the provider.
    Raw(Vec<u8>),
    Values(Vec<Value>),
}
struct Page {
    first: usize,
    count: usize,
    encoded: Arc<std::sync::Mutex<Encoded>>,
}
/// New history is readable from RAM immediately. Cache creation, stale-cache
/// cleanup and Fjall insertion must not delay opening the conversation.
fn spill_page(
    page: std::sync::Weak<std::sync::Mutex<Encoded>>,
    open: impl FnOnce() -> Result<Arc<ColdStore>>,
) {
    let Some(page) = page.upgrade() else {
        return;
    };
    let raw = {
        let guard = page.lock().unwrap_or_else(|e| e.into_inner());
        let Encoded::Raw(raw) = &*guard else {
            return;
        };
        raw.clone()
    };
    let key = PAGE_ID.fetch_add(1, Ordering::Relaxed).to_be_bytes();
    if let Ok(store) = open().and_then(|store| {
        store.pages.insert(key, raw.as_slice())?;
        Ok(store)
    }) {
        *page.lock().unwrap_or_else(|e| e.into_inner()) = Encoded::Stored { store, key };
    }
}
fn queue_page(page: &Arc<std::sync::Mutex<Encoded>>) {
    type Work = std::sync::Weak<std::sync::Mutex<Encoded>>;
    static WORKER: std::sync::OnceLock<Option<std::sync::mpsc::Sender<Work>>> =
        std::sync::OnceLock::new();
    if let Some(sender) = WORKER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<Work>();
        std::thread::Builder::new()
            .name("fastrock-scrollback".into())
            .spawn(move || {
                for page in receiver {
                    spill_page(page, ColdStore::open);
                }
            })
            .ok()
            .map(|_| sender)
    }) {
        // Queued entries are weak handles, never duplicate payloads or owners.
        let _ = sender.send(Arc::downgrade(page));
    }
}
impl Page {
    fn new(first: usize, items: Vec<Value>) -> Self {
        let count = items.len();
        let encoded = match serde_json::to_vec(&items) {
            Ok(raw) => Encoded::Raw(raw),
            Err(_) => Encoded::Values(items),
        };
        let encoded = Arc::new(std::sync::Mutex::new(encoded));
        queue_page(&encoded);
        Self {
            first,
            count,
            encoded,
        }
    }
    fn decode(&self) -> Result<Vec<Value>> {
        match &*self.encoded.lock().unwrap_or_else(|e| e.into_inner()) {
            Encoded::Stored { store, key } => {
                let bytes = store
                    .pages
                    .get(key)?
                    .ok_or_else(|| anyhow::anyhow!("Scrollback page missing"))?;
                Ok(serde_json::from_slice(&bytes)?)
            }
            Encoded::Raw(bytes) => Ok(serde_json::from_slice(bytes)?),
            Encoded::Values(items) => Ok(items.clone()),
        }
    }
}
impl Drop for Encoded {
    fn drop(&mut self) {
        if let Encoded::Stored { store, key } = self {
            let _ = store.pages.remove(*key);
        }
    }
}
struct Turn {
    metadata: Value,
    items: Range<usize>,
}
struct Cached {
    items: Arc<Vec<Value>>,
    weight: usize,
}

#[derive(Default)]
pub struct History {
    turns: Vec<Turn>,
    pages: Vec<Page>,
    cold_items: usize,
    // Only the current turn is mutable. All cold pages are immutable.
    hot: Option<Value>,
    cache: HashMap<usize, Cached>,
    lru: VecDeque<usize>,
    cache_bytes: usize,
    decompressions: usize,
}

impl History {
    pub fn len(&self) -> usize {
        self.turns.len() + usize::from(self.hot.is_some())
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn last(&self) -> Option<&Value> {
        self.hot.as_ref()
    }
    pub fn last_mut(&mut self) -> Option<&mut Value> {
        self.hot.as_mut()
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn push(&mut self, next: Value) {
        if let Some(mut old) = self.hot.replace(next) {
            let first = self.cold_items;
            let items = old["items"]
                .as_array_mut()
                .map(std::mem::take)
                .unwrap_or_default();
            let mut chunk = Vec::new();
            let mut weight = 0usize;
            for item in items {
                weight = weight.saturating_add(value_bytes(&item));
                chunk.push(item);
                if chunk.len() >= PAGE_ITEMS || weight >= PAGE_BYTES {
                    self.seal(std::mem::take(&mut chunk));
                    weight = 0;
                }
            }
            if !chunk.is_empty() {
                self.seal(chunk);
            }
            self.turns.push(Turn {
                metadata: old,
                items: first..self.cold_items,
            });
        }
    }

    fn seal(&mut self, items: Vec<Value>) {
        let page = Page::new(self.cold_items, items);
        self.cold_items += page.count;
        self.pages.push(page);
    }

    pub fn item_count(&self) -> usize {
        self.cold_items
            + self
                .hot
                .as_ref()
                .and_then(|t| t["items"].as_array())
                .map_or(0, Vec::len)
    }

    fn decoded(&mut self, index: usize) -> Result<Arc<Vec<Value>>> {
        if let Some(cached) = self.cache.get(&index) {
            self.lru.retain(|i| *i != index);
            self.lru.push_back(index);
            return Ok(cached.items.clone());
        }
        let items = Arc::new(self.pages[index].decode()?);
        self.decompressions += 1;
        let weight = items.capacity() * std::mem::size_of::<Value>()
            + items.iter().map(value_bytes).sum::<usize>();
        // A single huge tool result may exceed the cache budget. Return it for
        // this request without retaining it in the resident cache.
        if weight <= CACHE_BYTES {
            while self.cache_bytes + weight > CACHE_BYTES || self.cache.len() >= CACHE_PAGES {
                let Some(oldest) = self.lru.pop_front() else {
                    break;
                };
                if let Some(old) = self.cache.remove(&oldest) {
                    self.cache_bytes -= old.weight;
                }
            }
            self.cache_bytes += weight;
            self.cache.insert(
                index,
                Cached {
                    items: items.clone(),
                    weight,
                },
            );
            self.lru.push_back(index);
        }
        Ok(items)
    }

    fn raw_items(&mut self, range: Range<usize>) -> Result<Vec<Value>> {
        let mut result = Vec::with_capacity(range.len());
        let mut page = self
            .pages
            .partition_point(|p| p.first + p.count <= range.start);
        while page < self.pages.len() && self.pages[page].first < range.end {
            let first = self.pages[page].first;
            let items = self.decoded(page)?;
            let start = range.start.saturating_sub(first);
            let end = (range.end - first).min(items.len());
            result.extend(items[start..end].iter().cloned());
            page += 1;
        }
        if range.end > self.cold_items
            && let Some(items) = self.hot.as_ref().and_then(|t| t["items"].as_array())
        {
            let start = range.start.saturating_sub(self.cold_items).min(items.len());
            let end = (range.end - self.cold_items).min(items.len());
            result.extend(items[start..end].iter().cloned());
        }
        Ok(result)
    }

    /// Index lookup touches only requested pages, never clones the whole log.
    pub fn items_page(
        &mut self,
        offset: usize,
        limit: usize,
        descending: bool,
    ) -> Result<Vec<Value>> {
        let range = page_range(self.item_count(), offset, limit, descending);
        let mut turn = self.turns.partition_point(|t| t.items.end <= range.start);
        let items = self.raw_items(range.clone())?;
        let mut result = Vec::with_capacity(items.len());
        for (index, item) in range.zip(items) {
            while turn < self.turns.len() && self.turns[turn].items.end <= index {
                turn += 1;
            }
            let id = if index < self.cold_items {
                &self.turns[turn].metadata["id"]
            } else {
                &self.hot.as_ref().unwrap()["id"]
            };
            result.push(json!({"turnId":id,"item":item}));
        }
        if descending {
            result.reverse();
        }
        Ok(result)
    }

    pub fn turns_page(&self, offset: usize, limit: usize, descending: bool) -> Vec<Value> {
        let range = page_range(self.len(), offset, limit, descending);
        let mut result = Vec::with_capacity(range.len());
        for index in range {
            let source = self
                .turns
                .get(index)
                .map(|t| &t.metadata)
                .or(self.hot.as_ref())
                .unwrap();
            // In the hot turn, avoid cloning its large item array just to clear it.
            let mut metadata = source
                .as_object()
                .map(|object| {
                    object
                        .iter()
                        .filter(|(key, _)| key.as_str() != "items")
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect::<serde_json::Map<String, Value>>()
                })
                .unwrap_or_default();
            metadata.insert("items".into(), json!([]));
            metadata.insert("itemsView".into(), json!("notLoaded"));
            result.push(Value::Object(metadata));
        }
        if descending {
            result.reverse();
        }
        result
    }

    /// A bounded viewport at an absolute ordinal. Only intersecting cold pages
    /// are decoded; turn metadata is found by binary search in the same index.
    pub fn window(&mut self, start: Option<usize>, limit: usize) -> Result<Window> {
        let limit = limit.clamp(1, 100);
        let total = self.item_count();
        let start = start
            .unwrap_or_else(|| total.saturating_sub(limit))
            .min(total.saturating_sub(1));
        let items = self.items_page(start, limit, false)?;
        let end = start + items.len();
        let mut turns = Vec::new();
        let mut previous = None;
        for ordinal in start..end {
            let index = self.turns.partition_point(|turn| turn.items.end <= ordinal);
            if previous != Some(index) {
                turns.extend(self.turns_page(index, 1, false));
                previous = Some(index);
            }
        }
        Ok(Window {
            start,
            total,
            items,
            turns,
        })
    }

    /// Explicit full export/legacy read. Ordinary scrolling uses bounded pages.
    pub fn all_turns(&mut self) -> Result<Vec<Value>> {
        let mut result = Vec::with_capacity(self.len());
        for index in 0..self.turns.len() {
            let mut turn = self.turns[index].metadata.clone();
            turn["items"] = Value::Array(self.raw_items(self.turns[index].items.clone())?);
            result.push(turn);
        }
        result.extend(self.hot.iter().cloned());
        Ok(result)
    }
}

fn page_range(total: usize, offset: usize, limit: usize, descending: bool) -> Range<usize> {
    let start = offset.min(total);
    let end = start.saturating_add(limit).min(total);
    if descending {
        total - end..total - start
    } else {
        start..end
    }
}

// Conservative retained-allocation estimate, including JSON tree overhead.
fn value_bytes(value: &Value) -> usize {
    std::mem::size_of::<Value>()
        + match value {
            Value::String(s) => s.capacity(),
            Value::Array(a) => {
                a.capacity() * std::mem::size_of::<Value>()
                    + a.iter().map(value_bytes).sum::<usize>()
            }
            Value::Object(o) => o
                .iter()
                .map(|(k, v)| k.capacity() + 128 + value_bytes(v))
                .sum(),
            _ => 0,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v53_absolute_windows_are_bounded_and_keep_streaming_ordinals() {
        let mut history = History::default();
        for turn in 0..5000 {
            history.push(json!({"id":format!("turn-{turn}"),"items":[
                {"id":format!("user-{turn}"),"text":"fixture".repeat(200)},
                {"id":format!("reply-{turn}"),"text":"answer".repeat(200)}]}));
        }
        let before = history.decompressions;
        let middle = history.window(Some(5000), 100).unwrap();
        assert_eq!(middle.total, 10000);
        assert_eq!(middle.items.len(), 100);
        assert_eq!(middle.items[0]["item"]["id"], "user-2500");
        assert_eq!(middle.turns.len(), 50);
        assert_eq!(middle.turns[0]["id"], "turn-2500");
        assert!(
            history.decompressions - before <= 50,
            "never decode unrelated history"
        );
        history.last_mut().unwrap()["items"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"streamed"}));
        let same = history.window(Some(5000), 100).unwrap();
        assert_eq!(middle.items, same.items);
        assert_eq!(same.total, 10001);
        let tail = history.window(None, usize::MAX).unwrap();
        assert_eq!(tail.start, 9901);
        assert_eq!(tail.items.len(), 100);
        assert_eq!(tail.items.last().unwrap()["item"]["id"], "streamed");
        assert_eq!(tail.turns.last().unwrap()["id"], "turn-4999");
        let last = history.window(Some(usize::MAX), 0).unwrap();
        assert_eq!(last.start, 10000);
        assert_eq!(last.items.len(), 1);
        assert!(history.cache_bytes <= CACHE_BYTES && history.cache.len() <= CACHE_PAGES);
        assert!(
            History::default()
                .window(None, 100)
                .unwrap()
                .items
                .is_empty()
        );
    }
    #[test]
    fn v43_roundtrip_stream_boundaries_and_random_pages() {
        let turns: Vec<_> = (0..150).map(|turn| json!({"id":format!("t{turn}"),"status":"completed","error":null,
            "items":(0..73).map(|item| json!({"id":format!("i{turn}-{item}"),"text":format!("λ café 👩‍💻 {turn}:{item} {}", "distinctive source ".repeat(50)),"arguments":{"path":"README.md"}})).collect::<Vec<_>>()})).collect();
        let flat: Vec<_> = turns
            .iter()
            .flat_map(|t| {
                t["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| json!({"turnId":t["id"],"item":item}))
            })
            .collect();
        let mut history = History::default();
        for turn in &turns {
            history.push(turn.clone());
        }
        for descending in [false, true] {
            for offset in [
                0,
                72,
                73,
                74,
                511,
                7819,
                flat.len() - 1,
                flat.len(),
                usize::MAX,
            ] {
                let expected: Vec<_> = if descending {
                    flat.iter()
                        .rev()
                        .skip(offset.min(flat.len()))
                        .take(100)
                        .cloned()
                        .collect()
                } else {
                    flat.iter()
                        .skip(offset.min(flat.len()))
                        .take(100)
                        .cloned()
                        .collect()
                };
                assert_eq!(
                    history.items_page(offset, 100, descending).unwrap(),
                    expected
                );
                assert!(history.cache_bytes <= CACHE_BYTES && history.cache.len() <= CACHE_PAGES);
            }
        }
        assert_eq!(history.all_turns().unwrap(), turns);
        history.last_mut().unwrap()["items"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"streamed","text":"complete streamed bytes"}));
        assert_eq!(
            history.items_page(0, 1, true).unwrap()[0]["item"]["id"],
            "streamed"
        );
        assert_eq!(history.turns_page(0, 1, true)[0]["id"], "t149");
        let before = history.decompressions;
        history.items_page(100, 3, false).unwrap();
        let once = history.decompressions;
        history.items_page(100, 3, false).unwrap();
        assert!(once <= before + 1);
        assert_eq!(history.decompressions, once);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while history
            .pages
            .iter()
            .any(|p| matches!(&*p.encoded.lock().unwrap(), Encoded::Raw(_)))
        {
            assert!(
                std::time::Instant::now() < deadline,
                "History cache did not spill pages"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let encoded: usize = history
            .pages
            .iter()
            .map(|p| match &*p.encoded.lock().unwrap() {
                Encoded::Stored { .. } => 8,
                Encoded::Raw(bytes) => bytes.len(),
                Encoded::Values(v) => v.iter().map(value_bytes).sum(),
            })
            .sum();
        assert!(encoded < serde_json::to_vec(&flat).unwrap().len() / 4);
        history.clear();
        assert!(history.is_empty());
        assert_eq!(history.cache_bytes, 0);
    }

    #[test]
    fn v43_empty_turns_and_oversized_items_preserve_order() {
        let mut history = History::default();
        history.push(json!({"id":"empty","items":[]}));
        history.push(
            json!({"id":"large","items":[{"id":"large-item","text":"X".repeat(CACHE_BYTES+1)}]}),
        );
        history.push(json!({"id":"tail","items":[]}));
        assert_eq!(
            history.items_page(0, 100, true).unwrap()[0]["turnId"],
            "large"
        );
        assert_eq!(history.cache_bytes, 0);
        assert_eq!(history.all_turns().unwrap().len(), 3);
        assert!(
            history
                .items_page(usize::MAX, 100, false)
                .unwrap()
                .is_empty()
        );
    }
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    #[test]
    fn v95_engine_compresses_pages_and_releases_them() {
        let items = vec![json!({"id":"page","text":"repeated transcript ".repeat(50000)})];
        let page = Page::new(0, items.clone());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let (store, key) = loop {
            if let Encoded::Stored { store, key } = &*page.encoded.lock().unwrap() {
                break (store.clone(), *key);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Fjall page storage must become available"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        store.pages.rotate_memtable_and_wait().unwrap();
        assert_eq!(page.decode().unwrap(), items);
        assert!(
            store.pages.disk_space() < 600_000,
            "Fjall LZ4 must compress repeated pages"
        );
        drop(page);
        while store.pages.get(key).unwrap().is_some() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(store.pages.get(key).unwrap().is_none());
    }

    #[test]
    fn v100_slow_cold_cache_does_not_block_history_reads() {
        let items = vec![json!({"text":"Saved answer"})];
        let page = Page {
            first: 0,
            count: 1,
            encoded: Arc::new(std::sync::Mutex::new(Encoded::Raw(
                serde_json::to_vec(&items).unwrap(),
            ))),
        };
        let weak = Arc::downgrade(&page.encoded);
        let (started, ready) = std::sync::mpsc::channel();
        let (resume, wait) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            spill_page(weak, || {
                started.send(()).unwrap();
                wait.recv().unwrap();
                anyhow::bail!("fixture: cache disk unavailable")
            })
        });
        ready
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let page = Arc::new(page);
        let reader_page = page.clone();
        let (read, result) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || read.send(reader_page.decode()).unwrap());
        let decoded = result.recv_timeout(std::time::Duration::from_secs(5));
        // Always release the worker, even when the read assertion fails.
        resume.send(()).unwrap();
        worker.join().unwrap();
        reader.join().unwrap();
        assert_eq!(decoded.unwrap().unwrap(), items);
        assert_eq!(page.decode().unwrap(), items);
    }
}

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachePointTtl {
    pub seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheProfile {
    pub provider_prompt_cache_enabled: bool,
    pub local_context_cache_enabled: bool,
    pub tool_output_dedup_enabled: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CacheProviderPlane {
    BedrockMantle,
    BedrockRuntime,
}

impl CacheProviderPlane {
    pub fn label(self) -> &'static str {
        match self {
            Self::BedrockMantle => "bedrock_mantle",
            Self::BedrockRuntime => "bedrock_runtime",
        }
    }

    pub fn from_label(value: &str) -> Option<Self> {
        match value {
            "bedrock_mantle" => Some(Self::BedrockMantle),
            "bedrock_runtime" => Some(Self::BedrockRuntime),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachePlannerConfig {
    pub provider_plane: CacheProviderPlane,
    pub min_tokens_per_cache_point: u32,
    pub max_cache_points: usize,
    pub ttl: Option<CachePointTtl>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptSegment {
    pub id: String,
    pub estimated_tokens: u32,
    pub cacheable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannedCachePoint {
    pub segment_id: String,
    pub provider_plane: CacheProviderPlane,
    pub estimated_tokens: u32,
    pub ttl: Option<CachePointTtl>,
}

pub fn plan_cache_points(
    profile: &CacheProfile,
    config: &CachePlannerConfig,
    segments: &[PromptSegment],
) -> Vec<PlannedCachePoint> {
    if !profile.provider_prompt_cache_enabled || config.max_cache_points == 0 {
        return Vec::new();
    }

    segments
        .iter()
        .filter(|segment| segment.cacheable)
        .filter(|segment| segment.estimated_tokens >= config.min_tokens_per_cache_point)
        .take(config.max_cache_points)
        .map(|segment| PlannedCachePoint {
            segment_id: segment.id.clone(),
            provider_plane: config.provider_plane,
            estimated_tokens: segment.estimated_tokens,
            ttl: config.ttl.clone(),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CacheUsage {
    pub prompt_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheUsageRecord {
    pub provider_plane: CacheProviderPlane,
    pub prompt_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_hit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheUsageSnapshot {
    pub provider_plane: CacheProviderPlane,
    pub usage: CacheUsage,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CacheUsageLedger {
    usage_by_provider: BTreeMap<CacheProviderPlane, CacheUsage>,
}

impl CacheUsageLedger {
    pub fn record(&mut self, record: CacheUsageRecord) {
        let usage = self
            .usage_by_provider
            .entry(record.provider_plane)
            .or_default();
        usage.prompt_tokens += record.prompt_tokens;
        usage.cache_read_tokens += record.cache_read_tokens;
        usage.cache_write_tokens += record.cache_write_tokens;
        if record.cache_hit {
            usage.cache_hits += 1;
        } else {
            usage.cache_misses += 1;
        }
    }

    pub fn usage(&self, provider_plane: CacheProviderPlane) -> CacheUsage {
        self.usage_by_provider
            .get(&provider_plane)
            .copied()
            .unwrap_or_default()
    }

    pub fn snapshots(&self) -> Vec<CacheUsageSnapshot> {
        self.usage_by_provider
            .iter()
            .map(|(provider_plane, usage)| CacheUsageSnapshot {
                provider_plane: *provider_plane,
                usage: *usage,
            })
            .collect()
    }

    pub fn replace_snapshot(&mut self, snapshot: CacheUsageSnapshot) {
        self.usage_by_provider
            .insert(snapshot.provider_plane, snapshot.usage);
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ContextFragmentKind {
    Instruction,
    FileFragment,
    ToolSchema,
    McpManifest,
    SkillManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextFragmentInput {
    pub kind: ContextFragmentKind,
    pub source_id: String,
    pub bytes: Vec<u8>,
    pub estimated_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextFragmentRecord {
    pub kind: ContextFragmentKind,
    pub source_id: String,
    pub content_hash: String,
    pub byte_len: usize,
    pub estimated_tokens: u32,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextFragmentCacheResult {
    pub hit: bool,
    pub record: ContextFragmentRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocalContextFragmentCache {
    records_by_source: BTreeMap<(ContextFragmentKind, String), ContextFragmentRecord>,
}

impl LocalContextFragmentCache {
    pub fn remember_fragment(
        &mut self,
        input: ContextFragmentInput,
        now_ms: i64,
    ) -> ContextFragmentCacheResult {
        let source_key = (input.kind, input.source_id.clone());
        let content_hash = sha256_hex(&input.bytes);
        let hit = self
            .records_by_source
            .get(&source_key)
            .is_some_and(|record| record.content_hash == content_hash);
        let record = if hit {
            let record = self
                .records_by_source
                .get_mut(&source_key)
                .expect("record exists when hit");
            record.last_seen_ms = now_ms;
            record.estimated_tokens = input.estimated_tokens;
            record.clone()
        } else {
            let record = ContextFragmentRecord {
                kind: input.kind,
                source_id: input.source_id,
                content_hash,
                byte_len: input.bytes.len(),
                estimated_tokens: input.estimated_tokens,
                first_seen_ms: now_ms,
                last_seen_ms: now_ms,
            };
            self.records_by_source.insert(source_key, record.clone());
            record
        };

        ContextFragmentCacheResult { hit, record }
    }

    pub fn lookup(
        &self,
        kind: ContextFragmentKind,
        source_id: &str,
    ) -> Option<&ContextFragmentRecord> {
        self.records_by_source.get(&(kind, source_id.to_owned()))
    }

    pub fn invalidate_source(&mut self, kind: ContextFragmentKind, source_id: &str) -> bool {
        self.records_by_source
            .remove(&(kind, source_id.to_owned()))
            .is_some()
    }

    pub fn restore_record(&mut self, record: ContextFragmentRecord) {
        self.records_by_source
            .insert((record.kind, record.source_id.clone()), record);
    }

    pub fn records(&self) -> Vec<ContextFragmentRecord> {
        self.records_by_source.values().cloned().collect()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Observation,
    ProjectFact,
    UserPreference,
    PriorFix,
}

impl MemoryKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Observation => "observation",
            Self::ProjectFact => "project_fact",
            Self::UserPreference => "user_preference",
            Self::PriorFix => "prior_fix",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    Global,
    Project { project_folder_id: String },
}

impl MemoryScope {
    pub fn matches_project(&self, project_folder_id: Option<&str>) -> bool {
        match self {
            Self::Global => true,
            Self::Project {
                project_folder_id: scoped_id,
            } => project_folder_id.is_some_and(|candidate| candidate == scoped_id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRecord {
    pub id: String,
    pub kind: MemoryKind,
    pub scope: MemoryScope,
    pub text: String,
    pub source: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub enabled: bool,
}

impl MemoryRecord {
    pub fn new(
        id: impl Into<String>,
        kind: MemoryKind,
        scope: MemoryScope,
        text: impl Into<String>,
        source: impl Into<String>,
        now_ms: i64,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            scope,
            text: text.into(),
            source: source.into(),
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemorySearchResult {
    pub record: MemoryRecord,
    pub score: u32,
    pub matched_terms: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryInjection {
    pub record_id: String,
    pub citation: String,
    pub prompt_text: String,
    pub byte_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MemoryStore {
    records_by_id: BTreeMap<String, MemoryRecord>,
}

impl MemoryStore {
    pub fn upsert(&mut self, record: MemoryRecord) {
        self.records_by_id.insert(record.id.clone(), record);
    }

    pub fn delete(&mut self, record_id: &str) -> bool {
        self.records_by_id.remove(record_id).is_some()
    }

    pub fn set_enabled(&mut self, record_id: &str, enabled: bool, now_ms: i64) -> bool {
        let Some(record) = self.records_by_id.get_mut(record_id) else {
            return false;
        };
        record.enabled = enabled;
        record.updated_at_ms = now_ms;
        true
    }

    pub fn get(&self, record_id: &str) -> Option<&MemoryRecord> {
        self.records_by_id.get(record_id)
    }

    pub fn records(&self) -> Vec<MemoryRecord> {
        self.records_by_id.values().cloned().collect()
    }

    pub fn search(
        &self,
        query: &str,
        project_folder_id: Option<&str>,
        limit: usize,
    ) -> Vec<MemorySearchResult> {
        if limit == 0 {
            return Vec::new();
        }

        let query_terms = lexical_term_set(query);
        if query_terms.is_empty() {
            return Vec::new();
        }

        let mut results = self
            .records_by_id
            .values()
            .filter(|record| record.enabled)
            .filter(|record| record.scope.matches_project(project_folder_id))
            .filter_map(|record| {
                let record_terms = lexical_term_set(&format!(
                    "{} {} {}",
                    record.kind.label(),
                    record.source,
                    record.text
                ));
                let matched_terms = query_terms
                    .intersection(&record_terms)
                    .cloned()
                    .collect::<Vec<_>>();
                if matched_terms.is_empty() {
                    return None;
                }

                Some(MemorySearchResult {
                    record: record.clone(),
                    score: matched_terms.len() as u32,
                    matched_terms,
                })
            })
            .collect::<Vec<_>>();

        results.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| right.record.updated_at_ms.cmp(&left.record.updated_at_ms))
                .then_with(|| left.record.id.cmp(&right.record.id))
        });
        results.truncate(limit);
        results
    }

    pub fn bounded_injections(
        &self,
        query: &str,
        project_folder_id: Option<&str>,
        max_items: usize,
        max_bytes: usize,
    ) -> Vec<MemoryInjection> {
        if max_items == 0 || max_bytes == 0 {
            return Vec::new();
        }

        let mut used_bytes = 0usize;
        let mut injections = Vec::new();
        for result in self.search(query, project_folder_id, usize::MAX) {
            let citation = format!(
                "memory:{}:{}",
                result.record.kind.label(),
                result.record.source
            );
            let prompt_text = format!("[{citation}] {}", result.record.text);
            let byte_len = prompt_text.len();
            if used_bytes + byte_len > max_bytes {
                continue;
            }

            used_bytes += byte_len;
            injections.push(MemoryInjection {
                record_id: result.record.id,
                citation,
                prompt_text,
                byte_len,
            });
            if injections.len() == max_items {
                break;
            }
        }
        injections
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutputKind {
    CommandStdout,
    CommandStderr,
    FileRead,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolOutputDedupConfig {
    pub min_reference_bytes: usize,
    pub retain_raw_bytes: bool,
}

impl Default for ToolOutputDedupConfig {
    fn default() -> Self {
        Self {
            min_reference_bytes: 4096,
            retain_raw_bytes: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolOutputRecord {
    pub reference_id: String,
    pub conversation_id: String,
    pub kind: ToolOutputKind,
    pub content_hash: String,
    pub byte_len: usize,
    pub occurrences: u64,
    pub raw_bytes: Option<Vec<u8>>,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolOutputCacheAction {
    Inline,
    Stored { reference_id: String },
    Referenced { reference_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolOutputDedupResult {
    pub action: ToolOutputCacheAction,
    pub record: Option<ToolOutputRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutputDedupCache {
    config: ToolOutputDedupConfig,
    records_by_key: BTreeMap<(String, ToolOutputKind, String), ToolOutputRecord>,
}

impl ToolOutputDedupCache {
    pub fn new(config: ToolOutputDedupConfig) -> Self {
        Self {
            config,
            records_by_key: BTreeMap::new(),
        }
    }

    pub fn remember_output(
        &mut self,
        conversation_id: impl Into<String>,
        kind: ToolOutputKind,
        bytes: &[u8],
        now_ms: i64,
    ) -> ToolOutputDedupResult {
        if bytes.len() < self.config.min_reference_bytes {
            return ToolOutputDedupResult {
                action: ToolOutputCacheAction::Inline,
                record: None,
            };
        }

        let conversation_id = conversation_id.into();
        let content_hash = sha256_hex(bytes);
        let key = (conversation_id.clone(), kind, content_hash.clone());
        if let Some(record) = self.records_by_key.get_mut(&key) {
            record.occurrences += 1;
            record.last_seen_ms = now_ms;
            return ToolOutputDedupResult {
                action: ToolOutputCacheAction::Referenced {
                    reference_id: record.reference_id.clone(),
                },
                record: Some(record.clone()),
            };
        }

        let reference_id = format!(
            "tool-output:{conversation_id}:{kind_label}:{content_hash}",
            kind_label = kind.label()
        );
        let record = ToolOutputRecord {
            reference_id: reference_id.clone(),
            conversation_id,
            kind,
            content_hash,
            byte_len: bytes.len(),
            occurrences: 1,
            raw_bytes: self.config.retain_raw_bytes.then(|| bytes.to_vec()),
            first_seen_ms: now_ms,
            last_seen_ms: now_ms,
        };
        self.records_by_key.insert(key, record.clone());
        ToolOutputDedupResult {
            action: ToolOutputCacheAction::Stored { reference_id },
            record: Some(record),
        }
    }

    pub fn records(&self) -> Vec<ToolOutputRecord> {
        self.records_by_key.values().cloned().collect()
    }

    pub fn restore_record(&mut self, record: ToolOutputRecord) {
        self.records_by_key.insert(
            (
                record.conversation_id.clone(),
                record.kind,
                record.content_hash.clone(),
            ),
            record,
        );
    }
}

impl Default for ToolOutputDedupCache {
    fn default() -> Self {
        Self::new(ToolOutputDedupConfig::default())
    }
}

impl ToolOutputKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::CommandStdout => "command_stdout",
            Self::CommandStderr => "command_stderr",
            Self::FileRead => "file_read",
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn lexical_term_set(value: &str) -> BTreeSet<String> {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_profile(enabled: bool) -> CacheProfile {
        CacheProfile {
            provider_prompt_cache_enabled: enabled,
            local_context_cache_enabled: true,
            tool_output_dedup_enabled: true,
        }
    }

    #[test]
    fn t20_cache_planner_selects_threshold_segments() {
        let planned = plan_cache_points(
            &cache_profile(true),
            &CachePlannerConfig {
                provider_plane: CacheProviderPlane::BedrockMantle,
                min_tokens_per_cache_point: 100,
                max_cache_points: 2,
                ttl: Some(CachePointTtl { seconds: 300 }),
            },
            &[
                PromptSegment {
                    id: "tiny".to_owned(),
                    estimated_tokens: 20,
                    cacheable: true,
                },
                PromptSegment {
                    id: "system".to_owned(),
                    estimated_tokens: 120,
                    cacheable: true,
                },
                PromptSegment {
                    id: "tool".to_owned(),
                    estimated_tokens: 500,
                    cacheable: false,
                },
                PromptSegment {
                    id: "history".to_owned(),
                    estimated_tokens: 200,
                    cacheable: true,
                },
            ],
        );

        assert_eq!(
            planned,
            vec![
                PlannedCachePoint {
                    segment_id: "system".to_owned(),
                    provider_plane: CacheProviderPlane::BedrockMantle,
                    estimated_tokens: 120,
                    ttl: Some(CachePointTtl { seconds: 300 }),
                },
                PlannedCachePoint {
                    segment_id: "history".to_owned(),
                    provider_plane: CacheProviderPlane::BedrockMantle,
                    estimated_tokens: 200,
                    ttl: Some(CachePointTtl { seconds: 300 }),
                },
            ]
        );
    }

    #[test]
    fn t20_cache_planner_respects_profile_toggle() {
        let planned = plan_cache_points(
            &cache_profile(false),
            &CachePlannerConfig {
                provider_plane: CacheProviderPlane::BedrockRuntime,
                min_tokens_per_cache_point: 1,
                max_cache_points: 4,
                ttl: None,
            },
            &[PromptSegment {
                id: "system".to_owned(),
                estimated_tokens: 100,
                cacheable: true,
            }],
        );

        assert!(planned.is_empty());
    }

    #[test]
    fn v6_cache_usage_accounting_keeps_mantle_and_runtime_separate() {
        let mut ledger = CacheUsageLedger::default();
        ledger.record(CacheUsageRecord {
            provider_plane: CacheProviderPlane::BedrockMantle,
            prompt_tokens: 100,
            cache_read_tokens: 80,
            cache_write_tokens: 0,
            cache_hit: true,
        });
        ledger.record(CacheUsageRecord {
            provider_plane: CacheProviderPlane::BedrockRuntime,
            prompt_tokens: 200,
            cache_read_tokens: 0,
            cache_write_tokens: 150,
            cache_hit: false,
        });

        assert_eq!(
            ledger.usage(CacheProviderPlane::BedrockMantle),
            CacheUsage {
                prompt_tokens: 100,
                cache_read_tokens: 80,
                cache_write_tokens: 0,
                cache_hits: 1,
                cache_misses: 0,
            }
        );
        assert_eq!(
            ledger.usage(CacheProviderPlane::BedrockRuntime),
            CacheUsage {
                prompt_tokens: 200,
                cache_read_tokens: 0,
                cache_write_tokens: 150,
                cache_hits: 0,
                cache_misses: 1,
            }
        );
    }

    #[test]
    fn t20_cache_usage_snapshots_round_trip_by_provider_plane() {
        let mut ledger = CacheUsageLedger::default();
        ledger.record(CacheUsageRecord {
            provider_plane: CacheProviderPlane::BedrockMantle,
            prompt_tokens: 100,
            cache_read_tokens: 80,
            cache_write_tokens: 0,
            cache_hit: true,
        });
        ledger.record(CacheUsageRecord {
            provider_plane: CacheProviderPlane::BedrockRuntime,
            prompt_tokens: 200,
            cache_read_tokens: 0,
            cache_write_tokens: 150,
            cache_hit: false,
        });

        let snapshots = ledger.snapshots();
        let json = serde_json::to_string(&snapshots).unwrap();
        let decoded: Vec<CacheUsageSnapshot> = serde_json::from_str(&json).unwrap();
        let mut hydrated = CacheUsageLedger::default();
        for snapshot in decoded {
            hydrated.replace_snapshot(snapshot);
        }

        assert!(json.contains("bedrock_mantle"));
        assert_eq!(
            hydrated.usage(CacheProviderPlane::BedrockMantle),
            ledger.usage(CacheProviderPlane::BedrockMantle)
        );
        assert_eq!(
            hydrated.usage(CacheProviderPlane::BedrockRuntime),
            ledger.usage(CacheProviderPlane::BedrockRuntime)
        );
    }

    #[test]
    fn t20_local_context_fragment_cache_hits_until_content_hash_changes() {
        let mut cache = LocalContextFragmentCache::default();
        let input = ContextFragmentInput {
            kind: ContextFragmentKind::Instruction,
            source_id: "AGENTS.md".to_owned(),
            bytes: b"use rtk".to_vec(),
            estimated_tokens: 3,
        };

        let first = cache.remember_fragment(input.clone(), 10);
        let second = cache.remember_fragment(input, 20);
        let changed = cache.remember_fragment(
            ContextFragmentInput {
                kind: ContextFragmentKind::Instruction,
                source_id: "AGENTS.md".to_owned(),
                bytes: b"use rtk always".to_vec(),
                estimated_tokens: 4,
            },
            30,
        );

        assert!(!first.hit);
        assert!(second.hit);
        assert_eq!(second.record.first_seen_ms, 10);
        assert_eq!(second.record.last_seen_ms, 20);
        assert!(!changed.hit);
        assert_ne!(changed.record.content_hash, first.record.content_hash);
        assert_eq!(
            cache
                .lookup(ContextFragmentKind::Instruction, "AGENTS.md")
                .unwrap()
                .estimated_tokens,
            4
        );
    }

    #[test]
    fn t20_local_context_fragment_cache_invalidates_by_source() {
        let mut cache = LocalContextFragmentCache::default();
        cache.remember_fragment(
            ContextFragmentInput {
                kind: ContextFragmentKind::FileFragment,
                source_id: "src/main.rs:1-10".to_owned(),
                bytes: b"fn main() {}".to_vec(),
                estimated_tokens: 5,
            },
            10,
        );

        assert!(cache.invalidate_source(ContextFragmentKind::FileFragment, "src/main.rs:1-10"));
        assert!(
            cache
                .lookup(ContextFragmentKind::FileFragment, "src/main.rs:1-10")
                .is_none()
        );
    }

    #[test]
    fn t20_local_context_fragment_cache_restores_persisted_records() {
        let record = ContextFragmentRecord {
            kind: ContextFragmentKind::SkillManifest,
            source_id: "skills/release/SKILL.md".to_owned(),
            content_hash: "abc".to_owned(),
            byte_len: 42,
            estimated_tokens: 7,
            first_seen_ms: 10,
            last_seen_ms: 20,
        };
        let mut cache = LocalContextFragmentCache::default();

        cache.restore_record(record.clone());

        assert_eq!(
            cache.lookup(
                ContextFragmentKind::SkillManifest,
                "skills/release/SKILL.md"
            ),
            Some(&record)
        );
    }

    #[test]
    fn t20_tool_output_dedup_references_repeated_large_output_per_conversation() {
        let mut cache = ToolOutputDedupCache::new(ToolOutputDedupConfig {
            min_reference_bytes: 8,
            retain_raw_bytes: true,
        });
        let bytes = b"large repeated output";

        let first =
            cache.remember_output("conversation-1", ToolOutputKind::CommandStdout, bytes, 10);
        let second =
            cache.remember_output("conversation-1", ToolOutputKind::CommandStdout, bytes, 20);
        let other_conversation =
            cache.remember_output("conversation-2", ToolOutputKind::CommandStdout, bytes, 30);

        let ToolOutputCacheAction::Stored {
            reference_id: first_ref,
        } = first.action
        else {
            panic!("first large output should be stored");
        };
        assert_eq!(
            second.action,
            ToolOutputCacheAction::Referenced {
                reference_id: first_ref.clone()
            }
        );
        assert!(matches!(
            other_conversation.action,
            ToolOutputCacheAction::Stored { .. }
        ));
        assert_eq!(second.record.unwrap().occurrences, 2);
        assert_eq!(cache.records().len(), 2);
    }

    #[test]
    fn t20_tool_output_dedup_inlines_small_output_and_can_drop_raw_bytes() {
        let mut cache = ToolOutputDedupCache::new(ToolOutputDedupConfig {
            min_reference_bytes: 8,
            retain_raw_bytes: false,
        });

        let small = cache.remember_output("conversation-1", ToolOutputKind::FileRead, b"small", 10);
        let large = cache.remember_output(
            "conversation-1",
            ToolOutputKind::FileRead,
            b"large enough",
            20,
        );

        assert_eq!(small.action, ToolOutputCacheAction::Inline);
        assert!(small.record.is_none());
        assert!(matches!(large.action, ToolOutputCacheAction::Stored { .. }));
        assert_eq!(large.record.unwrap().raw_bytes, None);
    }

    #[test]
    fn t20_tool_output_dedup_restores_persisted_records() {
        let bytes = b"large repeated output";
        let mut original = ToolOutputDedupCache::new(ToolOutputDedupConfig {
            min_reference_bytes: 8,
            retain_raw_bytes: true,
        });
        let stored = original
            .remember_output("conversation-1", ToolOutputKind::CommandStdout, bytes, 10)
            .record
            .unwrap();
        let mut restored = ToolOutputDedupCache::new(ToolOutputDedupConfig {
            min_reference_bytes: 8,
            retain_raw_bytes: true,
        });
        restored.restore_record(stored.clone());

        let repeated =
            restored.remember_output("conversation-1", ToolOutputKind::CommandStdout, bytes, 20);

        assert_eq!(
            repeated.action,
            ToolOutputCacheAction::Referenced {
                reference_id: stored.reference_id
            }
        );
        assert_eq!(repeated.record.unwrap().occurrences, 2);
    }

    #[test]
    fn t20_memory_store_searches_lexically_with_project_scope() {
        let mut store = MemoryStore::default();
        store.upsert(MemoryRecord::new(
            "global-pref",
            MemoryKind::UserPreference,
            MemoryScope::Global,
            "Prefer short technical answers.",
            "settings:user",
            10,
        ));
        store.upsert(MemoryRecord::new(
            "project-fix",
            MemoryKind::PriorFix,
            MemoryScope::Project {
                project_folder_id: "project-a".to_owned(),
            },
            "Bedrock Mantle streaming fix: parse response output deltas.",
            "conversation:1",
            20,
        ));
        store.upsert(MemoryRecord::new(
            "other-project",
            MemoryKind::ProjectFact,
            MemoryScope::Project {
                project_folder_id: "project-b".to_owned(),
            },
            "Mantle endpoint is mocked differently here.",
            "conversation:2",
            30,
        ));

        let project_results = store.search("mantle streaming fix", Some("project-a"), 8);
        let ids = project_results
            .iter()
            .map(|result| result.record.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["project-fix"]);
        assert_eq!(
            project_results[0].matched_terms,
            vec![
                "fix".to_owned(),
                "mantle".to_owned(),
                "streaming".to_owned()
            ]
        );
        assert!(store.search("endpoint", Some("project-a"), 8).is_empty());
        assert!(
            store
                .search("technical answers", Some("project-a"), 8)
                .iter()
                .any(|result| result.record.id == "global-pref")
        );
    }

    #[test]
    fn t20_memory_store_disable_and_delete_control_visibility() {
        let mut store = MemoryStore::default();
        store.upsert(MemoryRecord::new(
            "observation",
            MemoryKind::Observation,
            MemoryScope::Global,
            "Runtime throttling was observed in us-east-1.",
            "conversation:3",
            10,
        ));

        assert_eq!(store.search("runtime throttling", None, 4).len(), 1);
        assert!(store.set_enabled("observation", false, 20));
        assert_eq!(store.get("observation").unwrap().updated_at_ms, 20);
        assert!(store.search("runtime throttling", None, 4).is_empty());

        assert!(store.set_enabled("observation", true, 30));
        assert_eq!(store.search("runtime throttling", None, 4).len(), 1);
        assert!(store.delete("observation"));
        assert!(store.records().is_empty());
        assert!(!store.delete("observation"));
    }

    #[test]
    fn t20_memory_injection_is_bounded_and_cited_by_source() {
        let mut store = MemoryStore::default();
        store.upsert(MemoryRecord::new(
            "short",
            MemoryKind::ProjectFact,
            MemoryScope::Project {
                project_folder_id: "project-a".to_owned(),
            },
            "Use Mantle store=false for local-state-only conversations.",
            "SPEC.md:bedrock",
            10,
        ));
        store.upsert(MemoryRecord::new(
            "long",
            MemoryKind::Observation,
            MemoryScope::Project {
                project_folder_id: "project-a".to_owned(),
            },
            "Mantle ".repeat(80),
            "conversation:large",
            20,
        ));

        let injections = store.bounded_injections("mantle store", Some("project-a"), 4, 120);

        assert_eq!(injections.len(), 1);
        assert_eq!(injections[0].record_id, "short");
        assert_eq!(
            injections[0].citation,
            "memory:project_fact:SPEC.md:bedrock"
        );
        assert!(
            injections[0]
                .prompt_text
                .starts_with("[memory:project_fact:SPEC.md:bedrock]")
        );
        assert!(injections[0].byte_len <= 120);
    }

    #[test]
    fn t20_memory_records_round_trip_json() {
        let record = MemoryRecord::new(
            "pref",
            MemoryKind::UserPreference,
            MemoryScope::Global,
            "Use rtk for shell commands.",
            "AGENTS.md",
            10,
        );

        let json = serde_json::to_string(&record).unwrap();
        let decoded: MemoryRecord = serde_json::from_str(&json).unwrap();

        assert!(json.contains("user_preference"));
        assert_eq!(decoded, record);
    }
}

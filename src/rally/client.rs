//! WSAPI v2: bounded, origin-bound, cancellable reads and explicit guarded writes.
use super::types::*;
use anyhow::{Context, Result, bail, ensure};
use futures::StreamExt;
use reqwest::{Method, Url};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};
const WSAPI: &str = "/slm/webservice/v2.0/";
const MAX_RESPONSE: usize = 32 * 1024 * 1024;
const CACHE_LIMIT: usize = 24 * 1024 * 1024;
#[derive(Clone)]
pub struct Client {
    base: Url,
    token: Arc<str>,
    http: reqwest::Client,
    slots: Arc<Semaphore>,
    cache: Arc<Mutex<Cache>>,
    cache_epoch: Arc<AtomicU64>,
    schemas: Arc<Mutex<HashMap<String, (Instant, Vec<Field>)>>>,
}
#[derive(Default)]
struct Cache {
    values: HashMap<String, (Instant, usize, Page)>,
    order: VecDeque<String>,
    bytes: usize,
}
impl Cache {
    fn insert(&mut self, key: String, page: Page) {
        let size = serde_json::to_vec(&page)
            .map(|v| v.len())
            .unwrap_or(CACHE_LIMIT + 1);
        if size > CACHE_LIMIT {
            return;
        }
        if let Some((_, old, _)) = self.values.remove(&key) {
            self.bytes = self.bytes.saturating_sub(old);
            self.order.retain(|k| k != &key);
        }
        while self.bytes + size > CACHE_LIMIT {
            if let Some(old) = self.order.pop_front() {
                if let Some((_, size, _)) = self.values.remove(&old) {
                    self.bytes = self.bytes.saturating_sub(size);
                }
            } else {
                break;
            }
        }
        self.bytes += size;
        self.order.push_back(key.clone());
        self.values.insert(key, (Instant::now(), size, page));
    }
}
impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RallyClient")
            .field("origin", &self.base.origin().ascii_serialization())
            .finish_non_exhaustive()
    }
}
impl Client {
    pub fn new(endpoint: &str, token: String) -> Result<Self> {
        let mut base =
            Url::parse(endpoint.trim().trim_end_matches('/')).context("Invalid Rally endpoint")?;
        ensure!(
            base.host_str().is_some()
                && base.username().is_empty()
                && base.password().is_none()
                && base.query().is_none()
                && base.fragment().is_none(),
            "Rally endpoint must be an HTTPS server URL"
        );
        let loopback = base.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        ensure!(
            base.scheme() == "https" || (base.scheme() == "http" && loopback),
            "Rally requires HTTPS; HTTP is allowed only for loopback fixtures"
        );
        ensure!(
            !token.trim().is_empty(),
            "Configure a Rally API token in Settings"
        );
        let prefix = base
            .path()
            .trim_end_matches('/')
            .trim_end_matches(WSAPI.trim_end_matches('/'))
            .trim_end_matches('/');
        base.set_path(&format!("{prefix}{WSAPI}"));
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .pool_max_idle_per_host(4)
            .build()?;
        Ok(Self {
            base,
            token: token.into(),
            http,
            slots: Arc::new(Semaphore::new(4)),
            cache: Default::default(),
            cache_epoch: Default::default(),
            schemas: Default::default(),
        })
    }
    pub fn resolve(&self, reference: &str) -> Result<Url> {
        ensure!(
            !reference.contains(['\\', '?', '#'])
                && !reference.split('/').any(|part| part == ".." || part == "."),
            "Invalid Rally reference"
        );
        // Reject encoded traversal/separators before URL normalization.
        let lower = reference.to_ascii_lowercase();
        ensure!(
            !["%2e", "%2f", "%5c"].iter().any(|s| lower.contains(s)),
            "Invalid Rally reference"
        );
        let url = self
            .base
            .join(reference)
            .context("Invalid Rally reference")?;
        ensure!(
            url.origin() == self.base.origin()
                && url.username().is_empty()
                && url.password().is_none(),
            "Refusing a Rally reference to another server"
        );
        ensure!(
            url.path().starts_with(self.base.path()),
            "Rally reference is outside WSAPI"
        );
        Ok(url)
    }
    pub fn reference_kind(&self, reference: &str) -> Result<&'static str> {
        let url = self.resolve(reference)?;
        let relative = url.path().strip_prefix(self.base.path()).unwrap_or("");
        let (kind, id) = relative
            .rsplit_once('/')
            .ok_or_else(|| anyhow::anyhow!("Expected a concrete Rally object reference"))?;
        ensure!(
            id.parse::<u64>().is_ok_and(|id| id > 0),
            "Expected positive Rally ObjectID"
        );
        canonical_kind(kind).context("Unsupported Rally kind")
    }
    pub fn same_reference(&self, a: &str, b: &str) -> bool {
        matches!((self.resolve(a),self.resolve(b)),(Ok(a),Ok(b)) if a==b)
    }
    pub async fn request(
        &self,
        method: Method,
        reference: &str,
        query: &[(String, String)],
        body: Option<Value>,
    ) -> Result<Value> {
        let mut url = self.resolve(reference)?;
        url.query_pairs_mut()
            .extend_pairs(query.iter().map(|(k, v)| (k, v)));
        for attempt in 0..=3u32 {
            let _slot = self.slots.acquire().await?;
            let mut request = self
                .http
                .request(method.clone(), url.clone())
                .header("ZSESSIONID", self.token.as_ref())
                .header("Accept", "application/json")
                .header("X-RallyIntegrationName", "Fastrock")
                .header("X-RallyIntegrationVendor", "Independent")
                .header("X-RallyIntegrationVersion", env!("CARGO_PKG_VERSION"));
            if let Some(body) = body.as_ref() {
                request = request.json(body);
            }
            let response = request.send().await.map_err(|e| {
                anyhow::anyhow!(
                    "Rally connection failed: {}",
                    e.to_string().replace(self.token.as_ref(), "[redacted]")
                )
            })?;
            let status = response.status();
            if method == Method::GET
                && attempt < 3
                && matches!(status.as_u16(), 429 | 502 | 503 | 504)
            {
                let delay = response
                    .headers()
                    .get("Retry-After")
                    .and_then(|s| s.to_str().ok())
                    .and_then(|s| {
                        s.parse::<u64>().ok().map(Duration::from_secs).or_else(|| {
                            chrono::DateTime::parse_from_rfc2822(s).ok().map(|date| {
                                Duration::from_secs(
                                    (date.timestamp() - chrono::Utc::now().timestamp()).max(0)
                                        as u64,
                                )
                            })
                        })
                    })
                    .unwrap_or_else(|| Duration::from_millis((1u64 << attempt) * 200))
                    .min(Duration::from_secs(30));
                drop(response);
                drop(_slot);
                tokio::time::sleep(delay).await;
                continue;
            }
            let mut bytes = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                ensure!(
                    bytes.len() + chunk.len() <= MAX_RESPONSE,
                    "Rally response exceeds 32 MiB; narrow the query"
                );
                bytes.extend_from_slice(&chunk);
            }
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
                anyhow::anyhow!("Rally returned invalid JSON ({})", status.as_u16())
            })?;
            let errors = value
                .as_object()
                .into_iter()
                .flat_map(|v| v.values())
                .filter_map(|v| v.get("Errors").and_then(Value::as_array))
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            ensure!(
                status.is_success() && errors.is_empty(),
                "Rally request failed ({}): {}",
                status.as_u16(),
                errors.join("; ").replace(self.token.as_ref(), "[redacted]")
            );
            return Ok(value);
        }
        bail!("Rally retry limit reached")
    }
    pub fn query_values(q: &Query) -> Vec<(String, String)> {
        let mut v = vec![
            (
                "fetch".into(),
                if q.fetch.is_empty() {
                    "true".into()
                } else {
                    q.fetch.clone()
                },
            ),
            ("start".into(), q.start.max(1).to_string()),
            (
                "pagesize".into(),
                if q.page_size == 0 {
                    200
                } else {
                    q.page_size.clamp(1, 2000)
                }
                .to_string(),
            ),
            ("projectScopeUp".into(), q.parents.to_string()),
            ("projectScopeDown".into(), q.children.to_string()),
        ];
        for (k, s) in [
            ("query", &q.expression),
            ("order", &q.order),
            ("workspace", &q.workspace),
            ("project", &q.project),
        ] {
            if !s.is_empty() {
                v.push((k.into(), s.clone()));
            }
        }
        if !q.artifact_types.is_empty() {
            v.push(("types".into(), q.artifact_types.join(",")));
        }
        v
    }
    pub async fn query(&self, kind: &str, q: &Query) -> Result<Page> {
        let kind = canonical_kind(kind).context("Unsupported Rally entity type")?;
        if kind == "Artifact" {
            ensure!(
                !q.artifact_types.is_empty()
                    && q.artifact_types
                        .iter()
                        .all(|k| ARTIFACT_KINDS.contains(&k.as_str())),
                "Choose supported Artifact types"
            );
        }
        let page = self.collection(&kind.to_ascii_lowercase(), q).await?;
        if kind == "Artifact" || ARTIFACT_KINDS.contains(&kind) {
            for o in &page.results {
                let actual = self.reference_kind(&o.text("_ref"))?;
                ensure!(
                    (if kind == "Artifact" {
                        q.artifact_types.iter().any(|k| k == actual)
                    } else {
                        kind == actual
                    }) && (o.text("_type").is_empty()
                        || canonical_kind(&o.text("_type")) == Some(actual)),
                    "Rally returned an artifact outside requested types"
                );
            }
        }
        Ok(page)
    }
    pub async fn collection(&self, reference: &str, q: &Query) -> Result<Page> {
        let value = self
            .request(Method::GET, reference, &Self::query_values(q), None)
            .await?;
        let result = &value["QueryResult"];
        let rows = result["Results"]
            .as_array()
            .context("Rally returned no QueryResult")?;
        let results = rows
            .iter()
            .map(|row| {
                row.as_object()
                    .cloned()
                    .context("Invalid Rally result record")
            })
            .collect::<Result<Vec<_>>>()?;
        let start = result["StartIndex"]
            .as_u64()
            .unwrap_or(q.start.max(1) as u64) as usize;
        ensure!(
            start == q.start.max(1),
            "Rally returned an unexpected page; refresh"
        );
        let total = result["TotalResultCount"]
            .as_u64()
            .unwrap_or(results.len() as u64) as usize;
        ensure!(
            !results.is_empty() || start > total || total == 0,
            "Rally returned an incomplete result"
        );
        let mut seen = std::collections::HashSet::new();
        for row in &results {
            let identity = row.text("_ref");
            if !identity.is_empty() {
                ensure!(seen.insert(identity), "Rally returned duplicate records");
            }
        }
        Ok(Page {
            results,
            total,
            start,
            page_size: result["PageSize"].as_u64().unwrap_or(q.page_size as u64) as usize,
        })
    }
    pub async fn cached_query(&self, kind: &str, q: &Query, refresh: bool) -> Result<Page> {
        let key = format!("{kind}:{}", serde_json::to_string(q)?);
        if !refresh {
            if let Some((stamp, _, page)) = self.cache.lock().await.values.get(&key) {
                if stamp.elapsed() < Duration::from_secs(60) {
                    return Ok(page.clone());
                }
            }
        }
        let epoch = self.cache_epoch.load(Ordering::SeqCst);
        let page = self.query(kind, q).await?;
        let mut cache = self.cache.lock().await;
        if epoch == self.cache_epoch.load(Ordering::SeqCst) {
            cache.insert(key, page.clone());
        }
        Ok(page)
    }
    pub async fn all(&self, kind: &str, q: &Query) -> Result<Vec<Object>> {
        let mut q = q.clone();
        q.start = 1;
        q.page_size = 200;
        let mut all = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut total = None;
        loop {
            let page = self.query(kind, &q).await?;
            ensure!(
                total.is_none_or(|total| total == page.total),
                "Rally result changed during paging; refresh"
            );
            total = Some(page.total);
            for o in &page.results {
                ensure!(seen.insert(o.text("_ref")), "Rally repeated a page");
            }
            let n = page.results.len();
            all.extend(page.results);
            ensure!(
                all.len() <= 100000,
                "Rally result exceeds 100,000 items; narrow query"
            );
            if all.len() >= page.total {
                return Ok(all);
            }
            ensure!(n > 0, "Incomplete Rally results");
            q.start += n;
        }
    }
    pub async fn get(&self, reference: &str) -> Result<Object> {
        let value = self
            .request(
                Method::GET,
                reference,
                &[("fetch".into(), "true".into())],
                None,
            )
            .await?;
        let object = value
            .as_object()
            .into_iter()
            .flat_map(|v| v.values())
            .filter_map(Value::as_object)
            .find(|o| o.contains_key("ObjectID"))
            .cloned()
            .context("Rally object not found")?;
        let returned = object.text("_ref");
        let url = self.resolve(&returned)?;
        let object_id = object
            .text("ObjectID")
            .parse::<u64>()
            .context("Rally object has invalid identity")?;
        ensure!(
            object_id > 0 && url.path().rsplit('/').next() == Some(&object_id.to_string()),
            "Rally object identity mismatch"
        );
        if reference
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .is_some_and(|id| id.parse::<u64>().is_ok())
        {
            ensure!(
                self.same_reference(reference, &returned),
                "Rally returned another object"
            );
        }
        Ok(object)
    }
    pub async fn current_user(&self) -> Result<Object> {
        let user = self.get("user").await?;
        ensure!(
            self.reference_kind(&user.text("_ref"))? == "User" && !user.text("ObjectID").is_empty(),
            "Invalid current-user identity"
        );
        Ok(user)
    }
    pub async fn create(&self, kind: &str, fields: Object) -> Result<Object> {
        let kind = canonical_kind(kind).context("Unsupported Rally entity type")?;
        self.mutate(
            &format!("{}/create", kind.to_ascii_lowercase()),
            kind,
            fields,
            &[],
        )
        .await
    }
    pub async fn mutate(
        &self,
        reference: &str,
        kind: &str,
        fields: Object,
        query: &[(String, String)],
    ) -> Result<Object> {
        self.cache_epoch.fetch_add(1, Ordering::SeqCst);
        *self.cache.lock().await = Cache::default();
        let key = kind.rsplit('/').next().unwrap_or(kind);
        let body = json!({key:fields});
        let value = self
            .request(Method::POST, reference, query, Some(body))
            .await?;
        value["OperationResult"]["Object"]
            .as_object()
            .or_else(|| value["CreateResult"]["Object"].as_object())
            .cloned()
            .context("Rally mutation returned no object")
    }
    pub async fn check_revision(&self, before: &Object) -> Result<Object> {
        ensure!(
            !before.text("LastUpdateDate").is_empty() || !before.text("VersionId").is_empty(),
            "Reload this item before editing; revision unavailable"
        );
        let current = self.get(&before.text("_ref")).await?;
        for key in ["LastUpdateDate", "VersionId"] {
            let stamp = before.text(key);
            ensure!(
                stamp.is_empty() || stamp == current.text(key),
                "{} changed on Rally; reload and review before saving",
                before.id()
            );
        }
        Ok(current)
    }
    pub async fn update(
        &self,
        before: &Object,
        fields: Object,
        position: Option<(&str, bool)>,
    ) -> Result<Object> {
        let kind = self.reference_kind(&before.text("_ref"))?;
        ensure!(
            !fields.contains_key("Rank") && !fields.contains_key("DragAndDropRank"),
            "Use relative ranking; Rally rank tokens are opaque"
        );
        let mut query = vec![("fetch".into(), "true".into())];
        if let Some((target, below)) = position {
            let target = self.resolve(target)?;
            ensure!(
                !self.same_reference(&before.text("_ref"), target.as_str()),
                "Cannot rank item against itself"
            );
            self.reference_kind(target.as_str())?;
            query.push((
                if below { "rankBelow" } else { "rankAbove" }.into(),
                target.path().into(),
            ));
        }
        self.check_revision(before).await?;
        self.mutate(&before.text("_ref"), kind, fields, &query)
            .await
    }
    pub async fn delete(&self, before: &Object) -> Result<()> {
        if self.reference_kind(&before.text("_ref"))? == "Attachment"
            && before.text("LastUpdateDate").is_empty()
            && before.text("VersionId").is_empty()
        {
            let current = self.get(&before.text("_ref")).await?;
            for field in [
                "ObjectID",
                "Name",
                "Content",
                "Size",
                "ContentType",
                "CreationDate",
                "Artifact",
            ] {
                ensure!(
                    before.get(field) == current.get(field),
                    "Attachment changed on Rally; reload before deleting"
                );
            }
        } else {
            self.check_revision(before).await?;
        }
        self.delete_reference(&before.text("_ref")).await
    }
    pub async fn delete_reference(&self, reference: &str) -> Result<()> {
        self.reference_kind(reference)?;
        self.cache_epoch.fetch_add(1, Ordering::SeqCst);
        *self.cache.lock().await = Cache::default();
        self.request(Method::DELETE, reference, &[], None).await?;
        Ok(())
    }
    pub async fn fields(&self, kind: &str, workspace: &str) -> Result<Vec<Field>> {
        let kind = canonical_kind(kind).context("Unsupported schema type")?;
        let key = format!("{workspace}:{kind}");
        if let Some((stamp, fields)) = self.schemas.lock().await.get(&key) {
            if stamp.elapsed() < Duration::from_secs(600) {
                return Ok(fields.clone());
            }
        }
        let definitions = self
            .query(
                "TypeDefinition",
                &Query {
                    expression: eq("TypePath", kind),
                    workspace: workspace.into(),
                    page_size: 2,
                    ..Default::default()
                },
            )
            .await?;
        ensure!(
            definitions.results.len() == 1,
            "Rally schema type not found"
        );
        let page = self
            .collection(
                &definitions.results[0].reference("Attributes"),
                &Query {
                    workspace: workspace.into(),
                    page_size: 2000,
                    ..Default::default()
                },
            )
            .await?;
        ensure!(
            page.results.len() == page.total,
            "Rally schema attributes incomplete"
        );
        let mut fields = Vec::new();
        for o in page.results {
            let mut f = Field {
                name: o.text("ElementName"),
                display_name: o.text("Name"),
                attribute_type: o.text("AttributeType"),
                reference_type: o
                    .get("AllowedValueType")
                    .and_then(Value::as_object)
                    .map(|v| v.text("TypePath"))
                    .unwrap_or_else(|| o.text("Type")),
                required: o.flag("Required"),
                read_only: o.flag("ReadOnly"),
                allowed_values: Vec::new(),
            };
            let reference = o.reference("AllowedValues");
            if !reference.is_empty() {
                let values = self
                    .collection(
                        &reference,
                        &Query {
                            workspace: workspace.into(),
                            page_size: 2000,
                            ..Default::default()
                        },
                    )
                    .await?;
                ensure!(
                    values.results.len() == values.total,
                    "{} allowed values incomplete",
                    f.name
                );
                f.allowed_values = values
                    .results
                    .iter()
                    .map(|v| v.text("StringValue"))
                    .collect();
            }
            fields.push(f);
        }
        let mut cache = self.schemas.lock().await;
        if cache.len() >= 32 {
            cache.clear();
        }
        cache.insert(key, (Instant::now(), fields.clone()));
        Ok(fields)
    }
    pub async fn workflow(
        &self,
        kind: &str,
        workspace: &str,
        fields: &[Field],
    ) -> Result<Vec<Object>> {
        if kind.to_ascii_lowercase().starts_with("portfolioitem") {
            let page = self
                .query(
                    "State",
                    &Query {
                        workspace: workspace.into(),
                        expression: eq("TypeDef.TypePath", kind),
                        order: "OrderIndex ASC".into(),
                        page_size: 200,
                        ..Default::default()
                    },
                )
                .await?;
            ensure!(page.total == page.results.len(), "State catalog incomplete");
            Ok(page.results)
        } else {
            Ok(fields
                .iter()
                .find(|f| f.name == state_field(kind))
                .map(|f| {
                    f.allowed_values
                        .iter()
                        .map(|s| json!({"Name":s}).as_object().cloned().unwrap_or_default())
                        .collect()
                })
                .unwrap_or_default())
        }
    }
    pub async fn complete_selections(&self, object: &mut Object) -> Result<()> {
        for field in ["Tags", "Milestones"] {
            let Some(value) = object.get(field) else {
                continue;
            };
            if value.is_array() {
                continue;
            }
            let reference = object.reference(field);
            if reference.is_empty() {
                continue;
            }
            ensure!(
                object.count(field) <= 10000,
                "Too many collection selections"
            );
            let mut query = Query {
                workspace: object.reference("Workspace"),
                start: 1,
                page_size: 2000,
                ..Default::default()
            };
            let mut rows = vec![];
            let mut total = None;
            let mut seen = std::collections::BTreeSet::new();
            loop {
                let page = self.collection(&reference, &query).await?;
                ensure!(
                    total.is_none_or(|total| total == page.total),
                    "Collection changed while loading"
                );
                total = Some(page.total);
                for row in &page.results {
                    let kind = self.reference_kind(&row.text("_ref"))?;
                    ensure!(
                        kind == if field == "Tags" { "Tag" } else { "Milestone" },
                        "Collection contains invalid reference type"
                    );
                    ensure!(seen.insert(row.text("_ref")), "Repeated collection item");
                }
                let n = page.results.len();
                rows.extend(page.results);
                ensure!(rows.len() <= 10000, "Too many collection selections");
                if rows.len() >= page.total {
                    break;
                }
                ensure!(n > 0, "Incomplete collection selections");
                query.start += n;
            }
            object.insert(field.into(), json!(rows));
        }
        Ok(())
    }
    pub async fn upload(
        &self,
        parent: &str,
        name: &str,
        content_type: &str,
        data: &[u8],
    ) -> Result<Object> {
        use base64::Engine;
        ensure!(
            data.len() <= 5 * 1024 * 1024,
            "Rally attachment exceeds 5 MiB"
        );
        self.reference_kind(parent)?;
        let content = self
            .create(
                "AttachmentContent",
                json!({"Content":base64::engine::general_purpose::STANDARD.encode(data)})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            )
            .await?;
        let result=self.create("Attachment",json!({"Artifact":parent,"Content":content.text("_ref"),"Name":name,"ContentType":content_type,"Size":data.len()}).as_object().cloned().unwrap_or_default()).await;
        if result.is_err() {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                self.delete_reference(&content.text("_ref")),
            )
            .await;
        }
        result
    }
    pub async fn download(&self, attachment: &Object) -> Result<Vec<u8>> {
        use base64::Engine;
        ensure!(
            attachment.number("Size") <= 5.0 * 1024.0 * 1024.0,
            "Attachment exceeds 5 MiB"
        );
        let content = self.get(&attachment.reference("Content")).await?;
        let encoded = content.text("Content");
        ensure!(
            encoded.len() <= ((5 * 1024 * 1024 + 2) / 3) * 4,
            "Attachment exceeds 5 MiB"
        );
        let data = base64::engine::general_purpose::STANDARD.decode(encoded)?;
        ensure!(data.len() <= 5 * 1024 * 1024, "Attachment exceeds 5 MiB");
        Ok(data)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn references_never_leave_configured_origin_or_wsapi() {
        let c = Client::new("https://rally.example", "secret".into()).unwrap();
        for bad in [
            "https://evil.example/slm/webservice/v2.0/user/1",
            "//evil.example/slm/webservice/v2.0/user/1",
            "/outside",
            "../user/1",
            "user/%2e%2e/1",
            "user/1?x=1",
            "user/1#x",
            "user\\1",
        ] {
            assert!(c.resolve(bad).is_err(), "{bad}");
        }
        assert_eq!(
            c.reference_kind("hierarchicalrequirement/1").unwrap(),
            "HierarchicalRequirement"
        );
        assert!(c.reference_kind("user/0").is_err());
    }
    #[test]
    fn only_loopback_can_use_http() {
        for endpoint in [
            "http://localhost:99",
            "http://127.0.0.1:99",
            "http://[::1]:99",
        ] {
            assert!(Client::new(endpoint, "s".into()).is_ok());
        }
        assert!(Client::new("http://rally.example", "s".into()).is_err());
    }
    #[tokio::test]
    async fn mutations_never_retry_and_errors_redact_tokens() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/slm/webservice/v2.0/task/create"))
            .respond_with(
                ResponseTemplate::new(503)
                    .set_body_json(json!({"CreateResult":{"Errors":["bad secret"]}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let c = Client::new(&server.uri(), "secret".into()).unwrap();
        let error = c
            .create("Task", Object::new())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("[redacted]") && !error.contains("secret"));
    }
    #[tokio::test]
    async fn revisions_prevent_write_after_concurrent_change() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/slm/webservice/v2.0/task/1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"Task":{"_ref":format!("{}/slm/webservice/v2.0/task/1",server.uri()),"ObjectID":1,"VersionId":"2"}})),
            )
            .mount(&server)
            .await;
        let c = Client::new(&server.uri(), "secret".into()).unwrap();
        let before =
            json!({"_ref":format!("{}/slm/webservice/v2.0/task/1",server.uri()),"VersionId":"1"})
                .as_object()
                .cloned()
                .unwrap();
        assert!(
            c.update(&before, Object::new(), None)
                .await
                .unwrap_err()
                .to_string()
                .contains("changed on Rally")
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
    fn obj(value: Value) -> Object {
        value.as_object().unwrap().clone()
    }
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };
    #[tokio::test]
    async fn full_export_pages_preserve_scope_and_reject_changed_totals() {
        let server = MockServer::start().await;
        let c = Client::new(&server.uri(), "secret".into()).unwrap();
        let scope = Query {
            workspace: "workspace/9".into(),
            project: "project/8".into(),
            expression: "(Blocked = true)".into(),
            ..Default::default()
        };
        for (start, id) in [(1, 1), (2, 2)] {
            Mock::given(method("GET")).and(path("/slm/webservice/v2.0/task")).and(query_param("start",start.to_string()))
                .and(query_param("workspace","workspace/9")).and(query_param("project","project/8"))
                .and(query_param("query","(Blocked = true)"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"QueryResult":{"StartIndex":start,"TotalResultCount":2,"Results":[{"_ref":format!("task/{id}"),"ObjectID":id,"_type":"Task"}]}})))
                .expect(1).mount(&server).await;
        }
        assert_eq!(c.all("Task", &scope).await.unwrap().len(), 2);
        server.reset().await;
        for (start, total, id) in [(1, 2, 1), (2, 3, 2)] {
            Mock::given(method("GET")).and(query_param("start",start.to_string()))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"QueryResult":{"StartIndex":start,"TotalResultCount":total,"Results":[{"_ref":format!("task/{id}"),"ObjectID":id}]}}))).mount(&server).await;
        }
        assert!(
            c.all("Task", &scope)
                .await
                .unwrap_err()
                .to_string()
                .contains("changed during paging")
        );
    }
    #[tokio::test]
    async fn incomplete_and_repeated_pages_fail_closed() {
        let server = MockServer::start().await;
        let c = Client::new(&server.uri(), "secret".into()).unwrap();
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"QueryResult":{"StartIndex":1,"TotalResultCount":1,"Results":[]}}),
            ))
            .mount(&server)
            .await;
        assert!(c.query("Task", &Query::default()).await.is_err());
        server.reset().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"QueryResult":{"TotalResultCount":2,"Results":[{"_ref":"task/1","ObjectID":1}]}}))).mount(&server).await;
        assert!(
            c.all("Task", &Query::default())
                .await
                .unwrap_err()
                .to_string()
                .contains("repeated a page")
        );
    }
    #[tokio::test]
    async fn collections_expand_every_tag_and_reject_wrong_types() {
        let server = MockServer::start().await;
        let c = Client::new(&server.uri(), "secret".into()).unwrap();
        for (start, id) in [(1, 1), (2, 2)] {
            Mock::given(method("GET")).and(path("/slm/webservice/v2.0/task/1/Tags")).and(query_param("start",start.to_string()))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"QueryResult":{"StartIndex":start,"TotalResultCount":2,"Results":[{"_ref":format!("tag/{id}"),"ObjectID":id,"Name":format!("tag{id}")}]}}))).mount(&server).await;
        }
        let mut object = obj(json!({"Tags":{"_ref":"task/1/Tags","Count":2}}));
        c.complete_selections(&mut object).await.unwrap();
        assert_eq!(object["Tags"].as_array().unwrap().len(), 2);
        server.reset().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"QueryResult":{"TotalResultCount":1,"Results":[{"_ref":"user/1","ObjectID":1}]}}))).mount(&server).await;
        let mut wrong = obj(json!({"Tags":{"_ref":"task/1/Tags","Count":1}}));
        assert!(c.complete_selections(&mut wrong).await.is_err());
    }
    #[tokio::test]
    async fn attachment_failure_cleans_orphan_once_and_large_upload_writes_nothing() {
        let server = MockServer::start().await;
        let c = Client::new(&server.uri(), "secret".into()).unwrap();
        Mock::given(method("POST")).and(path("/slm/webservice/v2.0/attachmentcontent/create"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"CreateResult":{"Object":{"_ref":"attachmentcontent/1","ObjectID":1},"Errors":[]}}))).expect(1).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/slm/webservice/v2.0/attachment/create"))
            .respond_with(ResponseTemplate::new(503).set_body_string("Fixture failure"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/slm/webservice/v2.0/attachmentcontent/1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"OperationResult":{"Errors":[]}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert!(
            c.upload("task/2", "native.txt", "text/plain", b"fixture")
                .await
                .is_err()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
        assert!(
            c.upload(
                "task/2",
                "large",
                "text/plain",
                &vec![0; 5 * 1024 * 1024 + 1]
            )
            .await
            .is_err()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }
    #[tokio::test]
    async fn attachment_download_roundtrips_and_metadata_conflict_prevents_delete() {
        use base64::Engine;
        let server = MockServer::start().await;
        let c = Client::new(&server.uri(), "secret".into()).unwrap();
        let data = b"native attachment \xf0\x9f\x8c\x8d";
        let before = obj(
            json!({"_ref":"attachment/1","ObjectID":1,"Name":"original","Content":{"_ref":"attachmentcontent/2"},"Size":data.len()}),
        );
        Mock::given(method("GET")).and(path("/slm/webservice/v2.0/attachmentcontent/2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"AttachmentContent":{"_ref":"attachmentcontent/2","ObjectID":2,"Content":base64::engine::general_purpose::STANDARD.encode(data)}}))).mount(&server).await;
        assert_eq!(c.download(&before).await.unwrap(), data);
        Mock::given(method("GET")).and(path("/slm/webservice/v2.0/attachment/1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"Attachment":{"_ref":"attachment/1","ObjectID":1,"Name":"changed","Content":{"_ref":"attachmentcontent/2"},"Size":data.len()}}))).mount(&server).await;
        assert!(c.delete(&before).await.is_err());
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET")
        );
    }
    #[tokio::test]
    async fn in_flight_read_cannot_repopulate_cache_after_write_invalidation() {
        let server = MockServer::start().await;
        let c = Client::new(&server.uri(), "secret".into()).unwrap();
        Mock::given(method("GET")).and(path("/slm/webservice/v2.0/task"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(150)).set_body_json(json!({"QueryResult":{"TotalResultCount":1,"Results":[{"_ref":"task/1","ObjectID":1}]}}))).mount(&server).await;
        Mock::given(method("DELETE"))
            .and(path("/slm/webservice/v2.0/task/2"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"OperationResult":{"Errors":[]}})),
            )
            .mount(&server)
            .await;
        let query = Query::default();
        let read = c.cached_query("Task", &query, true);
        let delete = async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            c.delete_reference("task/2").await.unwrap();
        };
        let (result, ()) = tokio::join!(read, delete);
        result.unwrap();
        assert!(c.cache.lock().await.values.is_empty());
    }
}

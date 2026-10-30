use super::{model::Pattern, webhook_model, *};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Path, Request, State as HttpState},
    http::StatusCode,
    routing::post,
};
use serde_json::{Map, Value, json};
use tokio::sync::Semaphore;

#[derive(Default)]
pub(crate) struct Listener {
    pub address: Option<String>,
    pub error: Option<String>,
}
#[derive(Clone)]
struct Handler {
    adapter: Arc<Adapter>,
    authority: String,
    permits: Arc<Semaphore>,
}
pub(in crate::goose) async fn listen(adapter: Arc<Adapter>) -> Option<tokio::task::JoinHandle<()>> {
    let configured =
        std::env::var("FASTROCK_WEBHOOK_BIND").unwrap_or_else(|_| "127.0.0.1:43153".into());
    let result = async {
        let addr: std::net::SocketAddr = configured.parse()?;
        anyhow::ensure!(
            addr.ip().is_loopback(),
            "Webhook bind address must be loopback"
        );
        let socket = tokio::net::TcpListener::bind(addr).await?;
        Ok::<_, anyhow::Error>(socket)
    }
    .await;
    match result {
        Ok(socket) => {
            let authority = socket.local_addr().ok()?.to_string();
            adapter.webhook_listener.lock().await.address = Some(format!("http://{authority}"));
            let state = Handler {
                adapter,
                authority,
                permits: Arc::new(Semaphore::new(16)),
            };
            let router = Router::new()
                .route("/hooks/{id}", post(receive))
                .with_state(state);
            Some(tokio::spawn(async move {
                let _ = axum::serve(socket, router).await;
            }))
        }
        Err(_) => {
            adapter.webhook_listener.lock().await.error = Some("Webhook listener unavailable. FASTROCK_WEBHOOK_BIND must be a free loopback address (default 127.0.0.1:43153); restart after correcting it.".into());
            None
        }
    }
}
pub(super) async fn new_key(id: &str) -> Result<(), JSONRPCErrorError> {
    let id = id.to_owned();
    tokio::task::spawn_blocking(move || {
        let key = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        crate::vault::put("webhook", &id, &key)
    })
    .await
    .map_err(|_| error("Credential worker failed"))?
    .map_err(|e| error(e.to_string()))
}
type Response = (StatusCode, Json<Value>);
fn response(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"outcome":message})))
}
async fn receive(
    HttpState(state): HttpState<Handler>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let Ok(_permit) = state.permits.clone().try_acquire_owned() else {
        return response(StatusCode::SERVICE_UNAVAILABLE, "busy");
    };
    if request.headers().contains_key("origin")
        || request.headers().get("host").and_then(|h| h.to_str().ok()) != Some(&state.authority)
    {
        return response(StatusCode::FORBIDDEN, "invalid_origin");
    }
    if uuid::Uuid::parse_str(&id).is_err() {
        return response(StatusCode::NOT_FOUND, "not_found");
    }
    let (parts, body) = request.into_parts();
    let body = match tokio::time::timeout(
        Duration::from_secs(5),
        to_bytes(body, webhook_model::MAX_BODY),
    )
    .await
    {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return response(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large"),
        Err(_) => return response(StatusCode::REQUEST_TIMEOUT, "body_timeout"),
    };
    let adapter = &state.adapter;
    let _gate = adapter.schedule_gate.lock().await;
    let mut stored = adapter.schedules.lock().await;
    if stored.available().is_err() {
        return response(StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable");
    }
    let Some(task) = stored.data.tasks.get(&id) else {
        return response(StatusCode::NOT_FOUND, "not_found");
    };
    let Pattern::Webhook { hook } = &task.pattern else {
        return response(StatusCode::NOT_FOUND, "not_found");
    };
    let secret_id = id.clone();
    let key =
        match tokio::task::spawn_blocking(move || crate::vault::get("webhook", &secret_id)).await {
            Ok(Ok(key)) => key,
            _ => return response(StatusCode::SERVICE_UNAVAILABLE, "credential_unavailable"),
        };
    let signature = parts
        .headers
        .get(&hook.header)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if hook.verify(key.as_bytes(), signature, &body).is_err() {
        return response(StatusCode::UNAUTHORIZED, "invalid_signature");
    }
    let payload: Value = match serde_json::from_slice(&body) {
        Ok(payload) => payload,
        Err(_) => return response(StatusCode::BAD_REQUEST, "invalid_json"),
    };
    let headers: Map<_, _> = parts
        .headers
        .iter()
        .filter(|(name, _)| {
            let name = name.as_str();
            !name.eq_ignore_ascii_case(&hook.header)
                && !matches!(
                    name,
                    "authorization" | "proxy-authorization" | "cookie" | "set-cookie" | "x-api-key"
                )
        })
        .filter_map(|(name, value)| value.to_str().ok().map(|v| (name.to_string(), json!(v))))
        .collect();
    let query: Map<_, _> = url::form_urlencoded::parse(parts.uri.query().unwrap_or("").as_bytes())
        .filter(|(name, _)| !matches!(name.as_ref(), "token" | "access_token" | "api_key" | "key"))
        .map(|(k, v)| (k.into_owned(), json!(v)))
        .collect();
    let context = json!({"body":payload,"headers":headers,"query":query});
    let mut data = stored.data.clone();
    let task = data.tasks.get_mut(&id).unwrap();
    let Pattern::Webhook { hook } = &task.pattern else {
        unreachable!()
    };
    let outcome = match webhook_model::accept(
        &mut task.deliveries,
        hook,
        &context,
        &body,
        &task.prompt,
        chrono::Utc::now().timestamp(),
    ) {
        Ok(outcome) => outcome,
        Err(_) => return response(StatusCode::UNPROCESSABLE_ENTITY, "delivery_rejected"),
    };
    if outcome["outcome"] == "duplicate" {
        return (StatusCode::OK, Json(outcome));
    }
    task.revision += 1;
    if stored.commit(data).await.is_err() {
        return response(StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable");
    }
    adapter.schedules_changed();
    (StatusCode::ACCEPTED, Json(outcome))
}

impl Adapter {
    // schedule_cycle holds schedule_gate. The same atomic snapshot binds each
    // receipt to its occurrence before any child is created or queued.
    pub(super) async fn dispatch_webhook_deliveries(self: &Arc<Self>, now: i64) {
        let mut stored = self.schedules.lock().await;
        if stored.available().is_err() {
            return;
        }
        let mut data = stored.data.clone();
        let mut expired = false;
        let mut ready = Vec::new();
        for task in data
            .tasks
            .values_mut()
            .filter(|t| !t.paused && t.active.is_none())
        {
            let Pattern::Webhook { hook } = &task.pattern else {
                continue;
            };
            for d in task.deliveries.iter_mut().filter(|d| d.status == "pending") {
                if d.expired(hook, now) {
                    d.status = "expired".into();
                    d.prompt.clear();
                    expired = true;
                    task.revision += 1;
                }
            }
            if let Some(d) = task
                .deliveries
                .iter()
                .find(|d| d.status == "pending" && d.ready <= now)
            {
                ready.push((task.id.clone(), task.source.clone(), d.id.clone()));
            }
        }
        if expired {
            if stored.commit(data).await.is_err() {
                return;
            }
            self.schedules_changed();
        }
        drop(stored);
        for (id, source, delivery) in ready {
            if self
                .sessions
                .lock()
                .await
                .get(&source)
                .is_some_and(|s| s.active.is_some())
            {
                continue;
            }
            let _ = self.start_schedule(&id, true, Some(&delivery)).await;
        }
    }
}

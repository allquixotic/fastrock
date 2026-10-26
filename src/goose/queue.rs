//! Local unsent queue. Operations and dequeue share the same session gate.
use super::*;

impl Adapter {
    pub(super) async fn queue_request(self: &Arc<Self>, params: &Value, method: &str) -> Reply {
        let id = session_id(text(params, "threadId"))?.to_string();
        let gate = self.gate(&id).await;
        let lock = gate.lock().await;
        let mut queues = self.queue.lock().await;
        if !queues.contains_key(&id) {
            queues.insert(id.clone(), self.read_queue(&id)?);
        }
        let queue = queues
            .get_mut(&id)
            .ok_or_else(|| error("Queue unavailable"))?;
        let before = queue.clone();
        let result = match method {
            "thread/queue/list" => return Ok(json!({"data":queue,"nextCursor":null})),
            "thread/queue/add" => {
                let item = json!({"id":uuid::Uuid::new_v4().to_string(),"input":params["input"],"clientUserMessageId":params["clientUserMessageId"]});
                queue.push(item.clone());
                json!({"queuedSubmission":item})
            }
            "thread/queue/update" => {
                let item = queue
                    .iter_mut()
                    .find(|q| q["id"] == params["queuedSubmissionId"])
                    .ok_or_else(|| {
                        error("Queued message was already consumed; the edit was not applied")
                    })?;
                item["input"] = params["input"].clone();
                json!({"queuedSubmission":item})
            }
            "thread/queue/delete" => {
                let old = queue.len();
                queue.retain(|q| q["id"] != params["queuedSubmissionId"]);
                json!({"deleted":queue.len()!=old})
            }
            _ => return Err(unsupported(method)),
        };
        if let Err(e) = self.write_queue(&id, queue) {
            *queue = before;
            return Err(e);
        }
        drop(queues);
        drop(lock);
        self.emit(json!({"method":"thread/queue/changed","params":{"threadId":ui_id(&id)}}))?;
        if method == "thread/queue/add" {
            self.schedule_queue(id).await;
        }
        Ok(result)
    }
    pub(super) fn schedule_queue(
        self: &Arc<Self>,
        id: String,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            let adapter = self.clone();
            let _=self.jobs.send(Box::pin(async move {
            // start_turn takes the same gate; reserve the next message before releasing it.
            // Only the currently idle session can claim a queue drain.
            let gate=adapter.gate(&id).await;
            let lock=gate.lock().await;
            let idle=adapter.sessions.lock().await.get(&id).is_some_and(|s|s.loaded&&s.active.is_none());
            if !idle{return}
            let item = {
                let mut queues=adapter.queue.lock().await;
                let Some(queue)=queues.get_mut(&id).filter(|q|!q.is_empty()) else{return};
                let item=queue.remove(0);
                // Preserve dispatch evidence if the process exits between persistence and send.
                if let Err(e)=adapter.save_dispatch(&id,&item).and_then(|_|adapter.write_queue(&id,queue)) {
                    queue.insert(0,item);
                    adapter.queue_error(&id,e);
                    return;
                }
                item
            };
            let params=json!({"threadId":ui_id(&id),"input":item["input"],"clientUserMessageId":item["clientUserMessageId"]});
            let result=adapter.start_turn_locked(&params).await;
            if result.is_err(){
                let mut queues=adapter.queue.lock().await;
                let queue=queues.entry(id.clone()).or_default();
                queue.insert(0,item);
                if let Err(e)=adapter.write_queue(&id,queue){adapter.queue_error(&id,e);}
            }
            drop(lock);
            let _=adapter.emit(json!({"method":"thread/queue/changed","params":{"threadId":ui_id(&id)}}));
            if let Err(e)=result { adapter.queue_error(&id,e); }
        })).await;
        })
    }
}

impl Adapter {
    pub(super) fn queue_path(&self, id: &str) -> std::path::PathBuf {
        use base64::Engine;
        self.queue_root.join(format!(
            "{}.json",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id)
        ))
    }
    fn read_queue(&self, id: &str) -> Result<Vec<Value>, JSONRPCErrorError> {
        let path = self.queue_path(id);
        match std::fs::metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(error(format!("Read queued drafts: {e}"))),
            Ok(m) if m.len() > 16 * 1024 * 1024 => {
                return Err(error("Queued drafts exceed 16 MiB; original file retained"));
            }
            Ok(_) => {}
        }
        let bytes = std::fs::read(path).map_err(|e| error(format!("Read queued drafts: {e}")))?;
        serde_json::from_slice(&bytes).map_err(|e| {
            error(format!(
                "Invalid queued drafts; original file retained: {e}"
            ))
        })
    }
    fn write_queue(&self, id: &str, queue: &[Value]) -> Result<(), JSONRPCErrorError> {
        let bytes = serde_json::to_vec(queue).map_err(|e| error(e.to_string()))?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(error("Queued drafts exceed 16 MiB"));
        }
        std::fs::create_dir_all(&self.queue_root)
            .map_err(|e| error(format!("Save queued drafts: {e}")))?;
        crate::rally::store::atomic_write(&self.queue_path(id), &bytes)
            .map_err(|e| error(format!("Save queued drafts: {e}")))
    }
    fn save_dispatch(&self, id: &str, item: &Value) -> Result<(), JSONRPCErrorError> {
        let path = self.queue_path(id).with_extension("last-dispatch.json");
        let bytes = serde_json::to_vec(item).map_err(|e| error(e.to_string()))?;
        crate::rally::store::atomic_write(&path, &bytes)
            .map_err(|e| error(format!("Preserve queued dispatch: {e}")))
    }
    fn queue_error(&self, id: &str, e: JSONRPCErrorError) {
        let _=self.emit(json!({"method":"error","params":{"threadId":ui_id(id),"turnId":null,"willRetry":false,"error":{"message":format!("Queued message retained: {}",e.message)}}}));
    }
}

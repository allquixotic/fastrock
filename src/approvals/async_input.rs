//! Async questions are ordinary messages: retain their forms across turn completion
//! and deliver replies through normal send/steer/queue, never JSON-RPC resolution.
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadItem;

use super::user_input::UserInputForm;
use super::*;

fn pending(thread_id: &str, item: &ThreadItem) -> Option<PendingRequest> {
    let ThreadItem::AgentMessage {
        id,
        questions: Some(questions),
        ..
    } = item
    else {
        return None;
    };
    if questions.is_empty() {
        return None;
    }
    Some(PendingRequest {
        request_id: RequestId::String(
            serde_json::json!(["codex-gui-async", thread_id, id]).to_string(),
        ),
        thread_id: thread_id.to_string(),
        // Async questions outlive the requesting turn.
        turn_id: None,
        origin: None,
        kind: RequestKind::UserInput(UserInputForm::from_async(id, questions)),
    })
}

impl AppController {
    pub(super) fn approvals_async_item(&mut self, thread_id: &str, item: &ThreadItem) {
        if let ThreadItem::UserMessage { content, .. } = item
            && let Some(replies) = crate::async_questions::parse_input(content)
        {
            let ids = self
                .approvals
                .async_answered
                .entry(thread_id.into())
                .or_default();
            ids.extend(replies.into_iter().map(|reply| reply.question_item_id));
            let ids = ids.clone();
            for tab in &mut self.tabs {
                if let Some(thread) = tab.thread_mut() {
                    for request in &mut thread.approvals.requests {
                        if request.thread_id == thread_id
                            && let RequestKind::UserInput(form) = &mut request.kind
                            && form.is_async()
                        {
                            form.dismiss_answered(&ids);
                        }
                    }
                    thread.approvals.drop_where(|request| {
                        matches!(&request.kind, RequestKind::UserInput(form) if form.is_async() && form.is_empty())
                    });
                }
            }
            self.approvals_show();
            self.refresh_tabs();
            return;
        }
        let Some(mut request) = pending(thread_id, item) else {
            return;
        };
        if let RequestKind::UserInput(form) = &mut request.kind
            && let Some(ids) = self.approvals.async_answered.get(thread_id)
        {
            form.dismiss_answered(ids);
            if form.is_empty() {
                return;
            }
        }
        // Do not remember until a tab can actually receive the card.
        let Some(index) = self.tab_index_for_thread(thread_id) else {
            return;
        };
        if !self
            .approvals
            .async_seen
            .insert((thread_id.into(), request.request_id.clone()))
        {
            return;
        }
        self.approvals_attach(index, request);
    }

    pub(crate) fn approvals_restore_async(&mut self, index: usize, items: &[ThreadItem]) {
        let Some(thread_id) = self
            .thread_tab(index)
            .and_then(|thread| thread.thread_id.clone())
        else {
            return;
        };
        // History is paginated newest-first. Remember replies before considering
        // any questions on this page; later older pages consult the same IDs.
        for item in items
            .iter()
            .filter(|item| matches!(item, ThreadItem::UserMessage { .. }))
        {
            self.approvals_async_item(&thread_id, item);
        }
        for item in items
            .iter()
            .filter(|item| matches!(item, ThreadItem::AgentMessage { .. }))
        {
            self.approvals_async_item(&thread_id, item);
        }
    }

    /// Returns true for async forms, including failed/rejected sends. Keep drafts
    /// and unanswered fields until a local send/queue operation accepts the reply.
    pub(super) fn approvals_finish_async(
        &mut self,
        index: usize,
        request_id: &RequestId,
        answer: &serde_json::Result<Answer>,
    ) -> bool {
        let Some(request) = self
            .thread_tab(index)
            .and_then(|thread| {
                thread
                    .approvals
                    .requests
                    .iter()
                    .find(|request| &request.request_id == request_id)
            })
            .filter(|request| request.is_async_question())
            .cloned()
        else {
            return false;
        };
        let answer = match answer {
            Ok(answer) => answer,
            Err(err) => {
                self.toast(err.to_string());
                return true;
            }
        };
        if let Some(text) = answer.result.as_str() {
            let input = vec![crate::session::text_input(text)];
            if !self.send_user_input(index, input) {
                return true;
            }
            if let Some(replies) = crate::async_questions::parse(text) {
                let ids = self
                    .approvals
                    .async_answered
                    .entry(request.thread_id)
                    .or_default();
                ids.extend(replies.into_iter().map(|reply| reply.question_item_id));
                let ids = ids.clone();
                if let Some(pending) = self.thread_tab_mut(index).and_then(|thread| {
                    thread
                        .approvals
                        .requests
                        .iter_mut()
                        .find(|pending| &pending.request_id == request_id)
                }) && let RequestKind::UserInput(form) = &mut pending.kind
                {
                    form.dismiss_answered(&ids);
                }
            }
            if let Some(thread) = self.thread_tab_mut(index) {
                thread.approvals.drop_where(|request| {
                    matches!(&request.kind, RequestKind::UserInput(form) if form.is_async() && form.is_empty())
                });
            }
        } else {
            if let Some(thread) = self.thread_tab_mut(index) {
                thread.approvals.take(request_id);
            }
            if let Some((kind, text)) = &answer.notice {
                self.transcript_push_notice(index, *kind, text.clone());
            }
        }
        self.approvals_show();
        self.refresh_tabs();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_app_server_protocol::AsyncUserInputQuestion;

    #[test]
    fn v6_async_pending_uses_message_identity_and_survives_turn_completion() {
        let item = ThreadItem::AgentMessage {
            id: "call".into(),
            text: "Which ticket?".into(),
            phase: None,
            memory_citation: None,
            delivery: None,
            questions: Some(vec![AsyncUserInputQuestion {
                title: "Which ticket?".into(),
                options: None,
            }]),
        };
        let request = pending("thread", &item).expect("async question");
        assert!(request.is_async_question());
        assert!(request.turn_id.is_none());
        let mut queue = PendingApprovals::default();
        assert!(queue.push(request.clone()));
        assert!(!queue.push(request));
        queue.drop_where(|request| request.turn_id.as_deref() == Some("completed-turn"));
        assert_eq!(queue.len(), 1);
        assert!(
            pending(
                "thread",
                &ThreadItem::AgentMessage {
                    id: "plain".into(),
                    text: "Ordinary message".into(),
                    phase: None,
                    memory_citation: None,
                    delivery: None,
                    questions: None,
                }
            )
            .is_none()
        );
    }
}

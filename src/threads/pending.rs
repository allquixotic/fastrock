//! Pending edits use the queue's atomic update/delete boundary. Never rewrite history.
use crate::app::{AppController, TabId};
use crate::backend::BackendError;
use crate::ui::PendingMessageState;
use codex_app_server_protocol::{
    ClientRequest, ThreadQueueDeleteParams, ThreadQueueDeleteResponse, ThreadQueueListParams,
    ThreadQueueListResponse, ThreadQueueUpdateParams, ThreadQueueUpdateResponse, UserInput,
};
use slint::ComponentHandle;

#[derive(Clone, Debug)]
pub(crate) struct EditingPending {
    client_id: String,
    submission_id: Option<String>,
    input: Vec<UserInput>,
}

impl AppController {
    pub(crate) fn pending_message_open(&mut self, index: usize, client_id: String) {
        let Some(thread) = self.thread_tab(index) else {
            return;
        };
        let tab_id = self.tabs[index].id;
        if let Some(pending) = thread
            .pending_inputs
            .iter()
            .find(|p| p.client_id == client_id)
        {
            self.pending_message_show(
                tab_id,
                EditingPending {
                    client_id,
                    submission_id: None,
                    input: pending.input.clone(),
                },
            );
            return;
        }
        let Some(thread_id) = thread.thread_id.clone() else {
            return;
        };
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let result = async {
                let mut cursor = None;
                loop {
                    let page: ThreadQueueListResponse = backend
                        .request(ClientRequest::ThreadQueueList {
                            request_id: backend.next_request_id(),
                            params: ThreadQueueListParams {
                                thread_id: thread_id.clone(),
                                cursor,
                                limit: Some(100),
                            },
                        })
                        .await?;
                    if let Some(item) = page
                        .data
                        .into_iter()
                        .find(|item| item.client_user_message_id == client_id)
                    {
                        return Ok::<_, BackendError>(Some(EditingPending {
                            client_id: client_id.clone(),
                            submission_id: Some(item.id),
                            input: item.input,
                        }));
                    }
                    cursor = page.next_cursor;
                    if cursor.is_none() {
                        return Ok(None);
                    }
                }
            }
            .await;
            crate::ui_thread::post(move |app| match result {
                Ok(Some(edit)) => app.pending_message_show(tab_id, edit),
                Ok(None) => {
                    if let Some(index) = app.tab_index_by_id(tab_id)
                        && let Some(input) = app.transcript_pending_input(index, &client_id)
                    {
                        app.pending_message_show(
                            tab_id,
                            EditingPending {
                                submission_id: Some(format!("gui-steer:{client_id}")),
                                client_id,
                                input,
                            },
                        );
                    } else {
                        app.toast(
                            "This message has already been consumed and can no longer be edited",
                        );
                    }
                }
                Err(error) => app.toast(format!(
                    "Could not read pending message: {}",
                    error.user_message()
                )),
            });
        });
    }

    fn pending_message_show(&mut self, tab_id: TabId, edit: EditingPending) {
        let Some(index) = self.tab_index_by_id(tab_id) else {
            return;
        };
        let Some(thread) = self.thread_tab_mut(index) else {
            return;
        };
        let text = edit
            .input
            .iter()
            .filter_map(|input| match input {
                UserInput::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        thread.extras.editing_pending = Some(edit);
        let state = self.window.global::<PendingMessageState>();
        state.set_text(text.into());
        state.set_error("".into());
        state.set_saving(false);
        state.set_open(true);
        state.on_action(move |action| {
            let action = action.to_string();
            crate::ui_thread::with_app(move |app| app.pending_message_action(tab_id, &action));
        });
    }

    fn pending_message_action(&mut self, tab_id: TabId, action: &str) {
        let state = self.window.global::<PendingMessageState>();
        if state.get_saving() {
            return;
        }
        let Some(index) = self.tab_index_by_id(tab_id) else {
            state.set_open(false);
            return;
        };
        if action == "cancel" {
            if let Some(thread) = self.thread_tab_mut(index) {
                thread.extras.editing_pending = None;
            }
            self.window.global::<PendingMessageState>().set_open(false);
            self.composer_focus();
            return;
        }
        let text = state.get_text().trim().to_string();
        if action != "save" && action != "delete" {
            return;
        }
        if action == "save" && text.is_empty() {
            state.set_error("Enter text or use Delete.".into());
            return;
        }
        let Some(thread) = self.thread_tab(index) else {
            return;
        };
        let Some(edit) = thread.extras.editing_pending.clone() else {
            return;
        };
        let input = replace_text(&edit.input, &text);
        let deleting = action == "delete";
        let Some(submission_id) = edit.submission_id else {
            let Some(thread) = self.thread_tab_mut(index) else {
                return;
            };
            let Some(position) = thread
                .pending_inputs
                .iter()
                .position(|p| p.client_id == edit.client_id)
            else {
                self.window.global::<PendingMessageState>().set_error("The message was dispatched while this editor was open. Your edits have not been sent.".into());
                return;
            };
            if deleting {
                thread.pending_inputs.remove(position);
            } else {
                thread.pending_inputs[position].input = input.clone();
            }
            self.pending_message_finished(tab_id, &edit.client_id, input, deleting);
            return;
        };
        let Some(thread_id) = thread.thread_id.clone() else {
            return;
        };
        state.set_saving(true);
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let result: Result<bool, BackendError> = if deleting {
                backend
                    .request::<ThreadQueueDeleteResponse>(ClientRequest::ThreadQueueDelete {
                        request_id: backend.next_request_id(),
                        params: ThreadQueueDeleteParams {
                            thread_id,
                            queued_submission_id: submission_id,
                        },
                    })
                    .await
                    .map(|response| response.deleted)
            } else {
                backend
                    .request::<ThreadQueueUpdateResponse>(ClientRequest::ThreadQueueUpdate {
                        request_id: backend.next_request_id(),
                        params: ThreadQueueUpdateParams {
                            thread_id,
                            queued_submission_id: submission_id,
                            input: input.clone(),
                        },
                    })
                    .await
                    .map(|_| true)
            };
            crate::ui_thread::post(move |app| match result {
                Ok(true) => app.pending_message_finished(tab_id, &edit.client_id, input, deleting),
                result => {
                    let state = app.window.global::<PendingMessageState>();
                    state.set_saving(false);
                    let error = match result {
                        Err(error) => error.user_message(),
                        _ => "The message has already been dispatched".to_string(),
                    };
                    state.set_error(
                        format!(
                            "Changes were not applied: {error}. Your edited text is still here."
                        )
                        .into(),
                    );
                }
            });
        });
    }

    fn pending_message_finished(
        &mut self,
        tab_id: TabId,
        client_id: &str,
        input: Vec<UserInput>,
        deleting: bool,
    ) {
        if let Some(index) = self.tab_index_by_id(tab_id) {
            if deleting {
                self.transcript_remove_echo(index, client_id);
            } else {
                self.transcript_update_echo(index, client_id, input);
            }
            if let Some(thread) = self.thread_tab_mut(index) {
                thread.extras.editing_pending = None;
            }
        }
        let state = self.window.global::<PendingMessageState>();
        state.set_open(false);
        state.set_saving(false);
        self.composer_focus();
    }
}

fn replace_text(input: &[UserInput], text: &str) -> Vec<UserInput> {
    let mut result = vec![UserInput::Text {
        text: text.to_owned(),
        text_elements: Vec::new(),
    }];
    result.extend(
        input
            .iter()
            .filter(|item| !matches!(item, UserInput::Text { .. }))
            .cloned(),
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editing_pending_text_preserves_attachments() {
        let input = vec![
            UserInput::Text {
                text: "old".into(),
                text_elements: vec![],
            },
            UserInput::Image {
                image: codex_app_server_protocol::ImageReference::Inline {
                    url: "https://example.com/image.png".into(),
                },
                detail: None,
            },
        ];
        let updated = replace_text(&input, "new\nmessage");
        assert_eq!(updated[1], input[1]);
        assert!(matches!(&updated[0], UserInput::Text { text, .. } if text == "new\nmessage"));
    }
}

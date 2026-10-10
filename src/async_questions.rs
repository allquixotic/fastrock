//! Stable Codex async-question identity and ordinary user-message reply envelope.
use codex_app_server_protocol::UserInput;
use serde::Deserialize;
use serde::Serialize;

const OPEN: &str = "<send_user_message_question_reply>";
const CLOSE: &str = "</send_user_message_question_reply>";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Reply {
    pub(crate) question_item_id: String,
    pub(crate) question: String,
    pub(crate) answer: String,
}

pub(crate) fn question_id(message_id: &str, index: usize) -> String {
    serde_json::json!(["request_user_input_async", message_id, index]).to_string()
}

pub(crate) fn encode(replies: &[Reply]) -> serde_json::Result<String> {
    Ok(format!("{OPEN}{}{CLOSE}", serde_json::to_string(replies)?))
}

pub(crate) fn parse(text: &str) -> Option<Vec<Reply>> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Replies {
        Many(Vec<Reply>),
        One(Reply),
    }
    let text = text.trim();
    let text = if text.starts_with("# Context from my IDE setup:\n") {
        text.rsplit_once("\n## My request for Codex:\n")?.1.trim()
    } else {
        text
    };
    let payload = text.strip_prefix(OPEN)?.strip_suffix(CLOSE)?;
    let replies = match serde_json::from_str::<Replies>(payload).ok()? {
        Replies::Many(replies) => replies,
        Replies::One(reply) => vec![reply],
    };
    (!replies.is_empty()).then_some(replies)
}

pub(crate) fn parse_input(input: &[UserInput]) -> Option<Vec<Reply>> {
    let mut content = input
        .iter()
        .filter(|item| !matches!(item, UserInput::Skill { .. } | UserInput::Mention { .. }));
    let UserInput::Text { text, .. } = content.next()? else {
        return None;
    };
    if content.next().is_some() {
        return None;
    }
    parse(text)
}

pub(crate) fn display(text: &str) -> Option<String> {
    Some(
        parse(text)?
            .iter()
            .map(|reply| format!("{}\n{}", reply.question, reply.answer))
            .collect::<Vec<_>>()
            .join("\n\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v6_async_reply_roundtrips_identity_escaped_text_and_duplicate_titles() {
        let replies = vec![
            Reply {
                question_item_id: question_id("call", 0),
                question: "Which ticket?".into(),
                answer: "SMP-42\n\"ready\"".into(),
            },
            Reply {
                question_item_id: question_id("call", 1),
                question: "Which ticket?".into(),
                answer: "None yet".into(),
            },
        ];
        let text = encode(&replies).expect("encode");
        assert_eq!(parse(&text), Some(replies.clone()));
        assert_ne!(replies[0].question_item_id, replies[1].question_item_id);
        assert_eq!(
            replies[0].question_item_id,
            r#"["request_user_input_async","call",0]"#
        );
        assert_eq!(
            display(&text),
            Some("Which ticket?\nSMP-42\n\"ready\"\n\nWhich ticket?\nNone yet".into())
        );
        assert!(parse(&format!("Quoted: {text}")).is_none());
        assert!(parse(&format!("{text} trailing")).is_none());
        assert!(parse(&format!("{OPEN}[]{CLOSE}")).is_none());
        let input = vec![crate::session::text_input(text.clone())];
        assert_eq!(parse_input(&input), Some(replies));
        assert!(
            parse_input(&[
                crate::session::text_input(text),
                crate::session::text_input("additional")
            ])
            .is_none()
        );
    }
}

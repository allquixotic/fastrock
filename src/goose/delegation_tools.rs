//! App-owned delegation tools. Goose still owns provider/tool execution.
use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Create {
    pub task_id: String,
    pub instructions: String,
    pub model: Option<String>,
    pub effort: Option<String>,
}
impl Create {
    pub fn validate(&self) -> Result<(), JSONRPCErrorError> {
        if self.task_id.is_empty()
            || self.task_id.len() > 80
            || !self
                .task_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(error(
                "task_id must contain 1–80 ASCII letters, digits, hyphens or underscores",
            ));
        }
        if self.instructions.trim().is_empty() || self.instructions.len() > 32768 {
            return Err(error(
                "Delegate instructions must contain 1–32768 UTF-8 bytes",
            ));
        }
        for value in [&self.model, &self.effort].into_iter().flatten() {
            if value.trim().is_empty() || value.len() > 256 || value.contains(['\n', '\r', '\0']) {
                return Err(error("Invalid delegate model or effort"));
            }
        }
        Ok(())
    }
}

pub(super) fn add(tools: &mut Vec<Value>) {
    tools.retain(|t| t["name"] != "fastrock_agents");
    let function = |name: &str, description: &str, properties: Value, required: Value| {
        json!({"type":"function","name":name,"description":description,"deferLoading":false,
            "inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
    };
    let child =
        json!({"type":"string","description":"Exact child thread ID returned by delegate_task."});
    tools.push(json!({"type":"namespace","name":"fastrock_agents","description":"Coordinate isolated child Goose conversations when the user requests delegation. Child workspace changes require explicit reviewed integration.","tools":[
        function("delegate_task", "Create and start a child with fresh task context, inherited permissions/provider, and isolated vwork workspaces. Use a stable unique task_id: repeating it returns the existing child without rerunning work. Include all needed task context in instructions. No automatic integration.",
            json!({"task_id":{"type":"string","minLength":1,"maxLength":80},"instructions":{"type":"string","minLength":1,"maxLength":32768},"model":{"type":"string"},"effort":{"type":"string"}}),json!(["task_id","instructions"])),
        function("list_delegates", "List your direct child conversations, including durable task identities and recovery status. Use the returned cursor for more.",json!({"cursor":{"type":"string"}}),json!([])),
        function("wait_delegate", "Wait up to 30 seconds for your child to finish or need Resume. Returns its final textual answer when completed; a timeout does not cancel it. Recovered work stays paused for human Resume.",json!({"thread_id":child}),json!(["thread_id"])),
        function("send_to_delegate", "Queue a follow-up to your child. A paused child remains paused; do not replay uncertain tool calls. Each call is a new message.",json!({"thread_id":child,"message":{"type":"string","minLength":1,"maxLength":32768}}),json!(["thread_id","message"])),
        function("interrupt_delegate", "Stop your child and its descendants, preserving work and pausing queues. Interrupted work needs human review and Resume.",json!({"thread_id":child}),json!(["thread_id"]))
    ]}));
}

/// Preserve Summon's load/recipe capability while removing its default shared-cwd
/// delegation. This changes only session configuration through public Goose APIs.
impl Adapter {
    pub(super) async fn restrict_native_delegation(
        &self,
        id: &str,
    ) -> Result<(), JSONRPCErrorError> {
        if !self.embedded_runtime {
            return Ok(());
        }
        let selection = self
            .rpc
            .call(
                "_goose/unstable/session/extensions/list",
                json!({"sessionId":id}),
            )
            .await?;
        for entry in selection["extensions"]
            .as_array()
            .ok_or_else(|| error("Cannot inspect session delegation tools"))?
        {
            let extension = &entry["extension"];
            if extension["type"] != "platform" || extension["name"] != "summon" {
                continue;
            }
            let Some(replacement) = restrict_summon(extension) else {
                continue;
            };
            self.rpc
                .call(
                    "_goose/unstable/session/extensions/remove",
                    json!({"sessionId":id,"extensionKey":entry["extensionKey"]}),
                )
                .await?;
            if let Some(extension) = replacement {
                self.rpc
                    .call(
                        "_goose/unstable/session/extensions/add",
                        json!({"sessionId":id,"extension":extension}),
                    )
                    .await?;
            }
        }
        Ok(())
    }
}

fn restrict_summon(extension: &Value) -> Option<Option<Value>> {
    let allowed = extension["available_tools"].as_array();
    if allowed.is_some_and(|tools| !tools.is_empty() && !tools.iter().any(|t| t == "delegate")) {
        return None;
    }
    if allowed.is_some_and(|tools| !tools.is_empty() && !tools.iter().any(|t| t == "load")) {
        return Some(None);
    }
    let mut result = extension.clone();
    result["available_tools"] = json!(["load"]);
    Some(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v55_delegation_validation_and_summon_filter_preserve_restrictions() {
        let args = |id: &str, text: &str| Create {
            task_id: id.into(),
            instructions: text.into(),
            model: None,
            effort: None,
        };
        assert!(args("build_child", "Do the task").validate().is_ok());
        for id in ["", "../child", "child\n"] {
            assert!(args(id, "task").validate().is_err());
        }
        assert!(args("child", " ").validate().is_err());
        assert!(args("child", &"é".repeat(16385)).validate().is_err());
        let base = json!({"type":"platform","name":"summon"});
        assert_eq!(
            restrict_summon(&base).unwrap().unwrap()["available_tools"],
            json!(["load"])
        );
        assert!(restrict_summon(&json!({"available_tools":["load"]})).is_none());
        assert_eq!(
            restrict_summon(&json!({"available_tools":["delegate"]})),
            Some(None)
        );
        let mut tools = vec![];
        add(&mut tools);
        add(&mut tools);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["tools"].as_array().unwrap().len(), 5);
    }
}

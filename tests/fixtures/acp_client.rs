use anyhow::{Result, ensure};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

pub(super) struct Client {
    reader: BufReader<ReadHalf<DuplexStream>>,
    writer: WriteHalf<DuplexStream>,
    task: tokio::task::JoinHandle<()>,
    next: u64,
}
impl Drop for Client {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Client {
    pub(super) async fn connect() -> Result<Self> {
        let connection = super::embedded::connect().await?;
        let mut client = Self {
            reader: BufReader::new(connection.reader),
            writer: connection.writer,
            task: connection.task,
            next: 0,
        };
        client
            .call(
                "initialize",
                json!({"protocolVersion":1,"clientCapabilities":{},
            "clientInfo":{"name":"model-tuning-fixture","version":"1"}}),
            )
            .await?;
        Ok(client)
    }

    pub(super) async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next += 1;
        self.writer
            .write_all(
                format!(
                    "{}\n",
                    json!({"jsonrpc":"2.0","id":self.next,
            "method":method,"params":params})
                )
                .as_bytes(),
            )
            .await?;
        loop {
            let mut line = String::new();
            ensure!(
                self.reader.read_line(&mut line).await? > 0,
                "Runtime closed during {method}"
            );
            let reply: Value = serde_json::from_str(&line)?;
            if reply["id"] == self.next {
                ensure!(
                    reply.get("error").is_none(),
                    "{method} failed: {}",
                    reply["error"]
                );
                return Ok(reply["result"].clone());
            }
            ensure!(
                reply.get("id").is_none(),
                "Unexpected runtime request: {reply}"
            );
        }
    }

    pub(super) async fn model(&mut self, session: &str, model: &str) -> Result<()> {
        self.call(
            "session/set_config_option",
            json!({"sessionId":session,
            "configId":"model","value":model}),
        )
        .await?;
        Ok(())
    }
}

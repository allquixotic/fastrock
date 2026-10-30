//! Seed only an isolated benchmark directory through unchanged Goose APIs.
use anyhow::{Context, Result, ensure};
use fastrock_workspaces::{
    embedded::Connection,
    summaries::{Summaries, Summary},
};
use goose_provider_types::{
    conversation::{Conversation, message::Message},
    model::ModelConfig,
};
use goose_runtime::{
    config::{GooseMode, paths::Paths},
    session::{SessionManager, SessionType},
};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Deserialize)]
struct Parameters {
    sessions: usize,
    messages: usize,
    project: PathBuf,
    vwork_home: PathBuf,
    output: PathBuf,
}

pub fn run() -> Result<()> {
    let path = PathBuf::from(
        std::env::args()
            .nth(2)
            .context("Missing benchmark parameters")?,
    );
    let parameters: Parameters = serde_json::from_slice(&std::fs::read(&path)?)?;
    let root = path
        .parent()
        .context("Missing benchmark directory")?
        .canonicalize()?;
    let goose =
        PathBuf::from(std::env::var_os("GOOSE_PATH_ROOT").context("Missing isolated Goose root")?)
            .canonicalize()?;
    ensure!(
        goose.starts_with(&root),
        "Goose root is outside the fixture directory"
    );
    ensure!(
        !Paths::data_dir().join("sessions/sessions.db").exists(),
        "Refusing to overwrite existing sessions"
    );
    ensure!(
        (1..=1000).contains(&parameters.sessions),
        "Invalid benchmark session count"
    );
    ensure!(
        (2..=10000).contains(&parameters.messages) && parameters.messages % 2 == 0,
        "Invalid benchmark message count"
    );
    for directory in [&parameters.project, &parameters.vwork_home] {
        std::fs::create_dir_all(directory)?;
        ensure!(
            directory.canonicalize()?.starts_with(&root),
            "Directory outside fixture root"
        );
    }
    ensure!(
        parameters
            .output
            .parent()
            .context("Missing output directory")?
            .canonicalize()?
            .starts_with(&root),
        "Output outside fixture root"
    );
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(seed(parameters))
}

async fn seed(parameters: Parameters) -> Result<()> {
    let manager = SessionManager::new(Paths::data_dir());
    let summaries = Summaries(Connection::new(parameters.vwork_home));
    let mut ids = Vec::new();
    for session in 0..parameters.sessions {
        let id = format!("benchmark-{session:04}");
        manager
            .create_session_with_id(
                &id,
                parameters.project.clone(),
                format!("Benchmark conversation {session:04}"),
                SessionType::Acp,
                GooseMode::Auto,
            )
            .await?;
        manager
            .update(&id)
            .provider_name("aws_bedrock")
            .model_config(ModelConfig::new("openai.gpt-6.1-sol"))
            .apply()
            .await?;
        let count = if session == 0 { parameters.messages } else { 2 };
        let mut conversation = Conversation::empty();
        for index in 0..count {
            let message = if index % 2 == 0 {
                Message::user().with_text(format!("Fixture prompt {session}/{index}: café 👩‍💻"))
            } else {
                let body = format!("History {session}/{index}: **markdown**, Unicode café 👩‍💻 and a [link](https://example.com). ").repeat(20);
                Message::assistant().with_text(format!(
                    "{body}{}",
                    if index == count - 1 {
                        " HISTORY-END-SENTINEL"
                    } else {
                        ""
                    }
                ))
            };
            conversation.push(message.with_id(format!("{id}-{index}")));
        }
        manager.replace_conversation(&id, &conversation).await?;
        // Match the persisted representation, including upstream timestamp/ID handling.
        let stored = manager
            .get_session(&id, true)
            .await?
            .conversation
            .context("Missing seeded history")?;
        let summary = Summary {
            revision: 1,
            text: format!(
                "BenchmarkKey{session:04} source/history/scrolling; fixture compaction details. "
            )
            .repeat(100),
            profile: "benchmark fixture".into(),
            source_messages: stored.messages().len(),
            source_digest: format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(stored.messages())?)
            ),
        };
        ensure!(
            crate::summary_worker::snapshot(&manager, &id)
                .await?
                .context("Missing summary source")?
                .current(&summary),
            "Summary did not match seeded source"
        );
        let key = format!("goose:{id}");
        summaries.request(key.clone(), 1).await?;
        ensure!(
            summaries.replace(key, summary).await?,
            "Summary seed rejected"
        );
        ids.push(id);
    }
    ensure!(
        manager.list_all_sessions().await?.len() == parameters.sessions,
        "Session seed count mismatch"
    );
    std::fs::write(
        parameters.output,
        serde_json::to_vec_pretty(
            &json!({"sessions":ids,"messages":parameters.messages,"stored_conversations":parameters.sessions}),
        )?,
    )?;
    Ok(())
}

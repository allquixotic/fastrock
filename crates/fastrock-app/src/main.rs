use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{fs, io::Write, panic};

use fastrock_app::{
    FastrockWorkflow, GuiPersistenceWrite, GuiRuntimeWork,
    aws_static_credentials_from_secret_material, default_aws_shared_config_files,
    handle_gui_action, load_persisted_workflow_from_client, persist_gui_action_writes,
    run_bedrock_model_message_with_default_http_and_static_secrets,
};
use fastrock_core::{AppCommand, AppEvent, AppSupervisor, AppSupervisorConfig};
use fastrock_persistence::{
    PersistenceActorClient, PersistenceActorHandle, PersistenceConfig, SecretRef,
    default_database_path,
};
use fastrock_projects::{LocalProjectFolder, LocalProjectWatcher, ProjectWatcherConfig};
use fastrock_rtk::{RtkBinaryStatus, diagnose_default_rtk_binary};
use fastrock_ui::{LlmProfileForm, UiLlmProvider, UiMantleAuthMode};
use slint::{ModelRc, VecModel};

slint::include_modules!();

const PANIC_LOG_FILE_NAME: &str = "panic.log";
const PANIC_LOG_MAX_BYTES: u64 = 1024 * 1024;
const PANIC_LOG_ROTATIONS: usize = 4;

fn main() -> Result<(), slint::PlatformError> {
    install_local_panic_hook();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .thread_name("fastrock-worker")
        .enable_all()
        .build()
        .expect("create Fastrock Tokio runtime");
    let supervisor_config = AppSupervisorConfig::default();
    let (supervisor, mut supervisor_events) =
        AppSupervisor::spawn_on(runtime.handle(), supervisor_config)
            .expect("start Fastrock app supervisor");

    runtime.spawn(async move {
        while let Some(event) = supervisor_events.recv().await {
            if matches!(event, AppEvent::SupervisorStopped) {
                break;
            }
        }
    });

    let workflow = Arc::new(Mutex::new(FastrockWorkflow::default()));
    let persistence = Arc::new(Mutex::new(None::<PersistenceActorHandle>));
    let persistence_client = Arc::new(Mutex::new(None::<PersistenceActorClient>));
    let app = AppWindow::new()?;
    {
        let workflow = workflow.lock().expect("workflow lock poisoned");
        refresh_app_state(&app, &workflow);
    }
    app.set_status_text(
        format!(
            "Runtime ready. Loading persisted state. Default approval policy: {}. Command/event channels: {}/{}.",
            fastrock_core::DEFAULT_APPROVAL_POLICY_LABEL,
            supervisor_config.command_channel_capacity,
            supervisor_config.event_channel_capacity,
        )
        .into(),
    );
    start_persistence_hydration(
        &runtime,
        workflow.clone(),
        app.as_weak(),
        persistence.clone(),
        persistence_client.clone(),
    );
    start_rtk_startup_diagnostic(&runtime, app.as_weak());
    let app_weak = app.as_weak();
    let runtime_handle = runtime.handle().clone();
    app.on_app_action({
        let workflow = workflow.clone();
        let persistence_client = persistence_client.clone();
        move |action| {
            let Some(client) = persistence_client
                .lock()
                .expect("persistence client lock poisoned")
                .clone()
            else {
                if let Some(app) = app_weak.upgrade() {
                    app.set_status_text("Persistence still loading; retry action shortly".into());
                }
                return;
            };
            let result = {
                let mut workflow = workflow.lock().expect("workflow lock poisoned");
                handle_gui_action(&mut workflow, &action, now_ms())
            };
            let runtime_work = result.runtime_work.clone();
            let runtime_client = client.clone();
            persist_writes_async(&runtime_handle, client, result.writes, app_weak.clone());
            spawn_runtime_work_async(
                &runtime_handle,
                workflow.clone(),
                runtime_client,
                app_weak.clone(),
                runtime_work,
            );
            if let Some(app) = app_weak.upgrade() {
                app.set_status_text(result.status.into());
                let workflow = workflow.lock().expect("workflow lock poisoned");
                refresh_app_state(&app, &workflow);
            }
        }
    });
    let run_result = app.run();

    let _ = supervisor.try_send(AppCommand::Shutdown);
    let persistence = persistence
        .lock()
        .expect("persistence lock poisoned")
        .take();
    if let Some(persistence) = persistence {
        runtime
            .block_on(persistence.shutdown())
            .expect("shutdown Fastrock persistence actor");
    }
    runtime.shutdown_timeout(Duration::from_secs(2));

    run_result
}

fn install_local_panic_hook() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let message = format!("Fastrock panic: {panic_info}\n");
        let _ = write_local_panic_log(&message);
        eprint!("{message}");
        default_hook(panic_info);
    }));
}

fn write_local_panic_log(message: &str) -> std::io::Result<()> {
    let database_path =
        default_database_path().map_err(|error| std::io::Error::other(error.to_string()))?;
    let app_data_dir = database_path
        .parent()
        .ok_or_else(|| std::io::Error::other("database path has no parent"))?;
    let log_dir = app_data_dir.join("logs");
    fs::create_dir_all(&log_dir)?;
    let log_path = log_dir.join(PANIC_LOG_FILE_NAME);
    rotate_local_log_file(
        &log_path,
        message.len() as u64,
        PANIC_LOG_MAX_BYTES,
        PANIC_LOG_ROTATIONS,
    )?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    file.write_all(message.as_bytes())?;
    file.flush()
}

fn rotate_local_log_file(
    log_path: &std::path::Path,
    incoming_bytes: u64,
    max_bytes: u64,
    rotations: usize,
) -> std::io::Result<()> {
    if rotations == 0 || max_bytes == 0 {
        return Ok(());
    }
    let current_len = match fs::metadata(log_path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if current_len.saturating_add(incoming_bytes) <= max_bytes {
        return Ok(());
    }

    let oldest = rotated_log_path(log_path, rotations);
    let _ = fs::remove_file(oldest);
    for index in (1..rotations).rev() {
        let from = rotated_log_path(log_path, index);
        let to = rotated_log_path(log_path, index + 1);
        if from.exists() {
            fs::rename(from, to)?;
        }
    }
    fs::rename(log_path, rotated_log_path(log_path, 1))
}

fn rotated_log_path(log_path: &std::path::Path, index: usize) -> std::path::PathBuf {
    log_path.with_extension(format!("log.{index}"))
}

fn start_rtk_startup_diagnostic(
    runtime: &tokio::runtime::Runtime,
    app_weak: slint::Weak<AppWindow>,
) {
    runtime.spawn(async move {
        let diagnostic = tokio::task::spawn_blocking(diagnose_default_rtk_binary).await;
        match diagnostic {
            Ok(diagnostic) => {
                let (rtk_status, rtk_detail, message) = match diagnostic.status {
                    RtkBinaryStatus::Available => {
                        let resolved = diagnostic
                            .resolved_path
                            .as_ref()
                            .map(|path| path.display().to_string())
                            .unwrap_or_else(|| diagnostic.binary.clone());
                        (
                            "ready".to_owned(),
                            resolved.clone(),
                            format!("RTK ready: {resolved}"),
                        )
                    }
                    RtkBinaryStatus::Missing => {
                        let detail = diagnostic.install_guidance.unwrap_or_else(|| {
                            "Install rtk and ensure it is available on PATH.".to_owned()
                        });
                        (
                            "missing".to_owned(),
                            detail.clone(),
                            format!("RTK missing: {}. {}", diagnostic.detail, detail),
                        )
                    }
                };
                invoke_rtk_status(app_weak, rtk_status, rtk_detail, message);
            }
            Err(error) => {
                invoke_rtk_status(
                    app_weak,
                    "error".to_owned(),
                    error.to_string(),
                    format!("RTK diagnostic task failed: {error}"),
                );
            }
        }
    });
}

fn start_persistence_hydration(
    runtime: &tokio::runtime::Runtime,
    workflow: Arc<Mutex<FastrockWorkflow>>,
    app_weak: slint::Weak<AppWindow>,
    persistence_slot: Arc<Mutex<Option<PersistenceActorHandle>>>,
    persistence_client_slot: Arc<Mutex<Option<PersistenceActorClient>>>,
) {
    let runtime_handle = runtime.handle().clone();
    runtime.spawn(async move {
        let persistence = tokio::task::spawn_blocking(|| {
            PersistenceConfig::default_for_current_platform()
                .and_then(PersistenceActorHandle::spawn)
        })
        .await;
        let persistence = match persistence {
            Ok(Ok(persistence)) => persistence,
            Ok(Err(error)) => {
                invoke_status(app_weak, format!("Persistence startup failed: {error}"));
                return;
            }
            Err(error) => {
                invoke_status(
                    app_weak,
                    format!("Persistence startup task failed: {error}"),
                );
                return;
            }
        };
        let client = persistence.client();
        {
            let mut slot = persistence_slot.lock().expect("persistence lock poisoned");
            *slot = Some(persistence);
        }
        {
            let mut slot = persistence_client_slot
                .lock()
                .expect("persistence client lock poisoned");
            *slot = Some(client.clone());
        }

        match load_persisted_workflow_from_client(&client).await {
            Ok(loaded_workflow) => {
                {
                    let mut workflow = workflow.lock().expect("workflow lock poisoned");
                    *workflow = loaded_workflow;
                }
                start_project_watchers(
                    &runtime_handle,
                    workflow.clone(),
                    client.clone(),
                    app_weak.clone(),
                );
                invoke_workflow_refresh(app_weak, workflow, "Persisted state loaded".to_owned());
            }
            Err(error) => {
                invoke_status(app_weak, format!("Persisted state load failed: {error}"));
            }
        }
    });
}

fn start_project_watchers(
    runtime: &tokio::runtime::Handle,
    workflow: Arc<Mutex<FastrockWorkflow>>,
    persistence: PersistenceActorClient,
    app_weak: slint::Weak<AppWindow>,
) {
    let folders = match workflow
        .lock()
        .expect("workflow lock poisoned")
        .watchable_local_project_folders()
    {
        Ok(folders) => folders,
        Err(error) => {
            invoke_status(app_weak, format!("Project watcher startup failed: {error}"));
            return;
        }
    };

    for folder in folders {
        let local_folder = LocalProjectFolder {
            label: folder.label.clone(),
            root: folder.root.clone(),
        };
        let spawn = LocalProjectWatcher::spawn(local_folder, ProjectWatcherConfig::default());
        let (watcher, mut events) = match spawn {
            Ok(spawn) => spawn,
            Err(error) => {
                invoke_status(
                    app_weak.clone(),
                    format!("Project watcher failed for {}: {error}", folder.folder_id),
                );
                continue;
            }
        };
        let workflow = workflow.clone();
        let persistence = persistence.clone();
        let app_weak = app_weak.clone();
        runtime.spawn(async move {
            let _watcher = watcher;
            while let Some(event) = events.recv().await {
                let result = {
                    let mut workflow = workflow.lock().expect("workflow lock poisoned");
                    workflow.handle_local_project_file_event(
                        &folder.folder_id,
                        &folder.root,
                        event,
                        now_ms(),
                    )
                };
                match result {
                    Ok(action_result) => {
                        let status = action_result.status.clone();
                        if let Err(error) =
                            persist_gui_action_writes(&persistence, action_result.writes).await
                        {
                            invoke_status(
                                app_weak.clone(),
                                format!("Project hot reload persist failed: {error}"),
                            );
                            continue;
                        }
                        invoke_workflow_refresh(app_weak.clone(), workflow.clone(), status);
                    }
                    Err(error) => {
                        invoke_status(
                            app_weak.clone(),
                            format!("Project hot reload failed: {error}"),
                        );
                    }
                }
            }
        });
    }
}

fn persist_writes_async(
    runtime: &tokio::runtime::Handle,
    persistence: PersistenceActorClient,
    writes: Vec<GuiPersistenceWrite>,
    app_weak: slint::Weak<AppWindow>,
) {
    if writes.is_empty() {
        return;
    }
    runtime.spawn(async move {
        if let Err(error) = persist_gui_action_writes(&persistence, writes).await {
            invoke_status(app_weak, format!("Persistence write failed: {error}"));
        }
    });
}

fn spawn_runtime_work_async(
    runtime: &tokio::runtime::Handle,
    workflow: Arc<Mutex<FastrockWorkflow>>,
    persistence: PersistenceActorClient,
    app_weak: slint::Weak<AppWindow>,
    work: Vec<GuiRuntimeWork>,
) {
    for item in work {
        match item {
            GuiRuntimeWork::RunModel(request) => {
                let workflow = workflow.clone();
                let persistence = persistence.clone();
                let app_weak = app_weak.clone();
                runtime.spawn(async move {
                    run_model_work_async(workflow, persistence, app_weak, request).await;
                });
            }
        }
    }
}

async fn run_model_work_async(
    workflow: Arc<Mutex<FastrockWorkflow>>,
    persistence: PersistenceActorClient,
    app_weak: slint::Weak<AppWindow>,
    request: fastrock_app::BedrockModelRunRequest,
) {
    let profile = {
        let workflow = workflow.lock().expect("workflow lock poisoned");
        workflow.profiles().profile(&request.profile_id).cloned()
    };
    let Some(profile) = profile else {
        invoke_status(
            app_weak,
            format!("Model run failed: profile {} not found", request.profile_id),
        );
        return;
    };

    let shared_config = match default_aws_shared_config_files().load() {
        Ok(shared_config) => shared_config,
        Err(error) => {
            append_model_work_error(
                workflow,
                persistence,
                app_weak,
                &request,
                format!("AWS shared config load failed: {error}"),
            )
            .await;
            return;
        }
    };

    let bearer_api_key = match mantle_bearer_api_key(&profile, &persistence).await {
        Ok(bearer_api_key) => bearer_api_key,
        Err(error) => {
            append_model_work_error(workflow, persistence, app_weak, &request, error).await;
            return;
        }
    };

    let static_secret_credentials =
        match bedrock_static_secret_credentials(&profile, &persistence).await {
            Ok(static_secret_credentials) => static_secret_credentials,
            Err(error) => {
                append_model_work_error(workflow, persistence, app_weak, &request, error).await;
                return;
            }
        };

    let run_result = run_bedrock_model_message_with_default_http_and_static_secrets(
        request.clone(),
        profile,
        &shared_config,
        bearer_api_key,
        static_secret_credentials,
    )
    .await;
    let status = match run_result {
        Ok(result) => {
            let writes = {
                let mut workflow = workflow.lock().expect("workflow lock poisoned");
                workflow.append_model_run_result(result, now_ms())
            };
            if let Err(error) = persist_gui_action_writes(&persistence, writes).await {
                format!("Model response persisted in memory, database write failed: {error}")
            } else {
                "Model response appended".to_owned()
            }
        }
        Err(error) => {
            let message = error.to_string();
            let writes = {
                let mut workflow = workflow.lock().expect("workflow lock poisoned");
                workflow.append_model_run_error(&request, error, now_ms())
            };
            if let Err(error) = persist_gui_action_writes(&persistence, writes).await {
                format!("Model run failed and error persistence failed: {error}")
            } else {
                format!("Model run failed: {message}")
            }
        }
    };
    invoke_workflow_refresh(app_weak, workflow, status);
}

async fn append_model_work_error(
    workflow: Arc<Mutex<FastrockWorkflow>>,
    persistence: PersistenceActorClient,
    app_weak: slint::Weak<AppWindow>,
    request: &fastrock_app::BedrockModelRunRequest,
    error: String,
) {
    let writes = {
        let mut workflow = workflow.lock().expect("workflow lock poisoned");
        workflow.append_model_run_error(request, error.clone(), now_ms())
    };
    if let Err(error) = persist_gui_action_writes(&persistence, writes).await {
        invoke_status(
            app_weak.clone(),
            format!("Model run failed and error persistence failed: {error}"),
        );
    }
    invoke_workflow_refresh(app_weak, workflow, format!("Model run failed: {error}"));
}

async fn mantle_bearer_api_key(
    profile: &LlmProfileForm,
    persistence: &PersistenceActorClient,
) -> Result<Option<String>, String> {
    if profile.provider != UiLlmProvider::BedrockMantle {
        return Ok(None);
    }
    match &profile.mantle_settings.auth_mode {
        UiMantleAuthMode::AwsSigV4 => Ok(None),
        UiMantleAuthMode::BearerApiKey { secret_ref } => persistence
            .get_secret(SecretRef(secret_ref.clone()))
            .await
            .map_err(|error| format!("Mantle API key load failed: {error}"))?
            .ok_or_else(|| format!("Mantle API key secret not found: {secret_ref}"))
            .map(Some),
    }
}

async fn bedrock_static_secret_credentials(
    profile: &LlmProfileForm,
    persistence: &PersistenceActorClient,
) -> Result<BTreeMap<String, fastrock_bedrock::AwsStaticCredentials>, String> {
    let fastrock_ui::UiCredentialSource::SecretRef { secret_ref } = &profile.credential_source
    else {
        return Ok(BTreeMap::new());
    };
    let secret_material = persistence
        .get_secret(SecretRef(secret_ref.clone()))
        .await
        .map_err(|error| format!("AWS static secret load failed: {error}"))?
        .ok_or_else(|| format!("AWS static secret not found: {secret_ref}"))?;
    let credentials = aws_static_credentials_from_secret_material(&secret_material)
        .map_err(|error| format!("AWS static secret parse failed: {error}"))?;
    Ok(BTreeMap::from([(secret_ref.clone(), credentials)]))
}

fn invoke_workflow_refresh(
    app_weak: slint::Weak<AppWindow>,
    workflow: Arc<Mutex<FastrockWorkflow>>,
    status: String,
) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = app_weak.upgrade() {
            let workflow = workflow.lock().expect("workflow lock poisoned");
            refresh_app_state(&app, &workflow);
            app.set_status_text(status.into());
        }
    });
}

fn invoke_status(app_weak: slint::Weak<AppWindow>, status: String) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = app_weak.upgrade() {
            app.set_status_text(status.into());
        }
    });
}

fn invoke_rtk_status(
    app_weak: slint::Weak<AppWindow>,
    rtk_status: String,
    rtk_detail: String,
    status: String,
) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = app_weak.upgrade() {
            app.set_rtk_status(rtk_status.into());
            app.set_rtk_detail(rtk_detail.into());
            app.set_status_text(status.into());
        }
    });
}

fn refresh_app_state(app: &AppWindow, workflow: &FastrockWorkflow) {
    let snapshot = workflow.shell_snapshot();
    app.set_active_conversation_title(snapshot.active_conversation_title.into());
    app.set_active_folder(snapshot.active_folder.into());
    app.set_active_profile(snapshot.active_profile.into());
    app.set_active_mode(snapshot.active_mode.into());
    app.set_plan_mode_enabled(snapshot.plan_mode_enabled);
    app.set_rtk_enabled(snapshot.rtk_enabled);
    app.set_prompt_compression_enabled(snapshot.prompt_compression_enabled);
    app.set_prompt_compression_level(snapshot.prompt_compression_level.into());
    app.set_send_shortcut(snapshot.send_shortcut.into());
    app.set_send_shortcut_label(snapshot.send_shortcut_label.into());
    app.set_sidebar_tree_rows(model_from_vec(
        snapshot
            .sidebar_tree_rows
            .into_iter()
            .map(|row| SidebarTreeRowData {
                row_id: row.row_id.into(),
                kind: row.kind.into(),
                project_folder_id: row.project_folder_id.into(),
                conversation_id: row.conversation_id.into(),
                title: row.title.into(),
                subtitle: row.subtitle.into(),
                status: row.status.into(),
                depth: row.depth,
                expanded: row.expanded,
                selected: row.selected,
                tooltip: row.tooltip.into(),
                action: row.action.into(),
            })
            .collect(),
    ));
    app.set_conversation_rows(model_from_vec(
        snapshot
            .conversation_rows
            .into_iter()
            .map(|row| ConversationRowData {
                conversation_id: row.conversation_id.into(),
                title: row.title.into(),
                folder: row.folder.into(),
                profile: row.profile.into(),
                mode: row.mode.into(),
                status: row.status.into(),
                selected: row.selected,
            })
            .collect(),
    ));
    app.set_project_folder_rows(model_from_vec(
        snapshot
            .project_folders
            .into_iter()
            .map(|folder| ProjectFolderRowData {
                label: folder.label.into(),
                path: folder.path.into(),
                target: folder.target.into(),
                default_profile: folder.default_profile.into(),
                recent_conversations: folder.recent_conversations.to_string().into(),
            })
            .collect(),
    ));
    app.set_transcript_blocks(model_from_vec(
        snapshot
            .transcript_blocks
            .into_iter()
            .map(|block| TranscriptBlockData {
                speaker: block.speaker.into(),
                body: block.body.into(),
                accent_kind: block.accent_kind.into(),
            })
            .collect(),
    ));
    app.set_profile_rows(model_from_vec(
        snapshot
            .profile_rows
            .into_iter()
            .map(|profile| ProfileRowData {
                profile_id: profile.profile_id.into(),
                name: profile.name.into(),
                provider: profile.provider.into(),
                model: profile.model.into(),
                region: profile.region.into(),
                enabled: profile.enabled,
                default_profile: profile.default_for_new_conversations,
                selected: profile.selected,
                validation: profile.validation.into(),
            })
            .collect(),
    ));
    app.set_file_tab_rows(model_from_vec(
        snapshot
            .file_tabs
            .into_iter()
            .map(|tab| FileTabRowData {
                path: tab.path.into(),
                dirty: tab.dirty,
                selected: tab.selected,
            })
            .collect(),
    ));
    app.set_editor_line_rows(model_from_vec(
        snapshot
            .editor_lines
            .into_iter()
            .map(|line| EditorLineRowData {
                line_number: line.line_number.to_string().into(),
                text: line.text.into(),
            })
            .collect(),
    ));
    app.set_diff_rows(model_from_vec(
        snapshot
            .diff_rows
            .into_iter()
            .map(|row| DiffRowData {
                line_number: row.line_number.to_string().into(),
                before: row.before.into(),
                after: row.after.into(),
            })
            .collect(),
    ));
    app.set_mcp_server_rows(model_from_vec(
        snapshot
            .mcp_servers
            .into_iter()
            .map(|server| McpServerRowData {
                server_id: server.server_id.into(),
                name: server.name.into(),
                scope: server.scope.into(),
                transport: server.transport.into(),
                status: server.status.into(),
                details: server.details.into(),
            })
            .collect(),
    ));
    app.set_mcp_tool_rows(model_from_vec(
        snapshot
            .mcp_tools
            .into_iter()
            .map(|tool| McpToolRowData {
                server_id: tool.server_id.into(),
                tool_name: tool.tool_name.into(),
                policy: tool.policy.into(),
                discovered: tool.discovered,
                disabled: tool.disabled,
                always_allowed: tool.always_allowed,
                disable_action: tool.disable_action.into(),
                allow_action: tool.allow_action.into(),
            })
            .collect(),
    ));
    app.set_mcp_discovery_rows(model_from_vec(
        snapshot
            .mcp_discovery
            .into_iter()
            .map(|item| McpDiscoveryRowData {
                server_id: item.server_id.into(),
                kind: item.kind.into(),
                name: item.name.into(),
            })
            .collect(),
    ));
    app.set_skill_rows(model_from_vec(
        snapshot
            .skills
            .into_iter()
            .map(|skill| SkillRowData {
                name: skill.name.into(),
                scope: skill.scope.into(),
                modes: skill.modes.into(),
                status: skill.status.into(),
            })
            .collect(),
    ));
    app.set_memory_rows(model_from_vec(
        snapshot
            .memory_rows
            .into_iter()
            .map(|memory| MemoryRowData {
                record_id: memory.record_id.into(),
                kind: memory.kind.into(),
                scope: memory.scope.into(),
                source: memory.source.into(),
                status: memory.status.into(),
                preview: memory.preview.into(),
            })
            .collect(),
    ));
    app.set_settings_rows(model_from_vec(
        snapshot
            .settings_rows
            .into_iter()
            .map(|setting| SettingsRowData {
                name: setting.name.into(),
                value: setting.value.into(),
            })
            .collect(),
    ));
}

fn model_from_vec<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    std::rc::Rc::new(VecModel::from(items)).into()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

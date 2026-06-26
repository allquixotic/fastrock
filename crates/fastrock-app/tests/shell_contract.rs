const APP_SLINT: &str = include_str!("../ui/app.slint");
const APP_LIB: &str = include_str!("../src/lib.rs");
const APP_MAIN: &str = include_str!("../src/main.rs");

use std::collections::{BTreeMap, BTreeSet};

use fastrock_app::{
    BedrockProfileRequest, FastrockWorkflow, GuiPersistenceWrite, GuiRuntimeWork,
    handle_gui_action, load_persisted_workflow_from_client,
};
use fastrock_bedrock::AwsSharedConfig;
use fastrock_cache::{MemoryKind, MemoryRecord, MemoryScope};
use fastrock_core::{
    CommandPolicy, FastrockError, FastrockErrorKind, ThreadGoalStatus, TranscriptEventKind,
};
use fastrock_editor::EditorBuffer;
use fastrock_mcp::{McpConfigSet, McpRuntimeSnapshot, McpServerConfig, McpTransportConfig};
use fastrock_persistence::{
    ConversationEvent, ConversationMetadata, PersistenceActorHandle, PersistenceConfig,
    ProjectFolderRecord,
};
use fastrock_skills::{SkillScope, SkillWriteRequest};
use fastrock_ui::UiLlmProvider;
use tempfile::tempdir;

fn is_ulid(value: &str) -> bool {
    value.len() == 26
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'T' | b'V'..=b'Z'))
}

#[test]
fn t2_shell_declares_required_panes() {
    for required in [
        "SidebarTreeRow",
        "SidebarTreeRowData",
        "TranscriptBlock",
        "FileTabRow",
        "EditorLineRow",
        "DiffRow",
        "McpServerRow",
        "McpToolRow",
        "McpDiscoveryRow",
        "SkillRow",
        "MemoryRow",
        "ProfileRow",
        "SettingsRow",
        "ProfileActionButton",
        "HoverTip",
        "TextInput",
        "key-pressed(event)",
        "right-panel",
        "root.right-panel = label",
        "if root.right-panel == \"Files\": Rectangle",
        "if root.right-panel == \"Diff\": Rectangle",
        "if root.right-panel == \"MCP\": Rectangle",
        "if root.right-panel == \"Skills\": Rectangle",
        "if root.right-panel == \"Settings\": Rectangle",
        "profile-discover",
        "profile-test",
        "profile-draft-mantle",
        "profile-draft-runtime",
        "profile-save",
        "profile-select:",
        "profile-toggle:",
        "profile-default:",
        "profile-tuning-max-4096",
        "profile-tuning-max-off",
        "profile-tuning-temp-02",
        "profile-tuning-temp-off",
        "profile-tuning-topp-09",
        "profile-tuning-topp-off",
        "profile-tuning-timeout-120s",
        "profile-tuning-timeout-off",
        "profile-tuning-retry-2",
        "profile-tuning-retry-off",
        "plan-toggle",
        "goal-pause",
        "goal-resume",
        "goal-complete",
        "goal-clear",
        "command-policy-default",
        "command-policy-disabled",
        "command-policy-locked",
        "settings-toggle-rtk",
        "settings-toggle-prompt-compression",
        "settings-compression-level:lite",
        "settings-send-shortcut:control-enter",
        "conversation-send:",
        "editor-select:",
        "editor-close:",
        "editor-append",
        "editor-save",
        "editor-undo",
        "editor-redo",
        "editor-revert",
        "editor-diff",
        "editor-find:",
        "editor-goto:",
        "Open Tabs",
        "Active File",
        "No file change selected",
        "Settings",
        "LLM Profiles",
        "Request Tuning",
        "MCP Servers",
        "Runtime Preferences",
        "Skills",
        "mcp-add",
        "mcp-edit:",
        "mcp-toggle:",
        "mcp-restart:",
        "mcp-delete:",
        "skill-create",
        "skill-edit:",
        "skill-mode:",
        "skill-scope:",
        "skill-delete:",
        "memory-inspect:",
        "memory-toggle:",
        "memory-delete:",
        "conversation-rows",
        "sidebar-tree-rows",
        "conversation-id",
        "active-mode",
        "plan-mode-enabled",
        "compose-message",
        "project-folder-rows",
        "send-shortcut",
        "send-shortcut-label",
        "prompt-compression-enabled",
        "rtk-enabled",
        "transcript-blocks",
        "profile-rows",
        "file-tab-rows",
        "editor-line-rows",
        "diff-rows",
        "mcp-server-rows",
        "mcp-tool-rows",
        "mcp-discovery-rows",
        "skill-rows",
        "memory-rows",
        "settings-rows",
    ] {
        assert!(
            APP_SLINT.contains(required),
            "app shell must include {required}"
        );
    }
    for required in [
        "plan-start",
        "plan-execute",
        "mcp-tool-disable:",
        "mcp-tool-allow:",
        "command-policy-default",
        "command-policy-disabled",
        "command-policy-locked",
        "conversation-select:",
        "sidebar-toggle-folder:",
        "sidebar-toggle-unbound",
    ] {
        assert!(
            APP_LIB.contains(required),
            "GUI action router must include {required}"
        );
    }
}

#[test]
fn t22_right_pane_tabs_are_interactive_and_fit_sidebar_width() {
    for label in ["Files", "Diff", "MCP", "Skills", "Settings"] {
        assert!(
            APP_SLINT.contains(&format!("selected: root.right-panel == \"{label}\"")),
            "right pane tab must bind selection for {label}"
        );
        assert!(
            APP_SLINT.contains(&format!("if root.right-panel == \"{label}\": Rectangle")),
            "right pane panel must bind visibility for {label}"
        );
    }
    assert!(
        !APP_SLINT.contains("label == \"Settings\" ? 66px"),
        "right-pane tabs must not use the clipped prototype width"
    );
}

#[test]
fn t29_shell_is_dark_resizable_and_codex_like() {
    for required in [
        "preferred-width: 1320px;",
        "preferred-height: 860px;",
        "min-width: 1120px;",
        "min-height: 620px;",
        "width: 420px;",
        "composer-menu-open",
        "PlanToggleRow",
        "SidebarTreeRow",
        "SidebarActionButton",
        "global Tip",
        "component HoverTip inherits Rectangle",
        "Tip.visible",
        "root.right-panel = \"Settings\"",
        "rtk-status",
        "rtk-detail",
        "rtk-enabled",
        "prompt-compression-enabled",
        "send-shortcut-label",
        "key-pressed(event)",
        "event.modifiers.control",
        "if root.right-panel == \"Settings\": Rectangle",
        "Flickable",
        "has-hover",
        "pressed",
    ] {
        assert!(
            APP_SLINT.contains(required),
            "dark responsive shell must include {required}"
        );
    }
    for forbidden in [
        "width: 1200px;",
        "height: 760px;",
        "shown:",
        "import { LineEdit }",
        "action: \"plan-start\"",
        "action: \"plan-execute\"",
        "label: \"run\"",
        "label: \"rtk\"",
        "accepted =>",
        "Project Folders",
        "#f8f9fb",
        "#f5f6f8",
        "#eef1f4",
    ] {
        assert!(
            !APP_SLINT.contains(forbidden),
            "shell must remove prototype UI fragment: {forbidden}"
        );
    }
}

#[test]
fn t31_tooltips_and_settings_geometry_are_overlay_safe() {
    for required in [
        // V19: the active tooltip renders in one window-level overlay, not a
        // per-control popup, so it is above content and never clipped.
        "tooltip-overlay := Rectangle",
        "visible: Tip.visible && Tip.text",
        "drop-shadow-blur: 12px;",
        "if root.right-panel != \"Settings\": Rectangle",
        "if root.right-panel == \"Files\": Rectangle",
        "if root.right-panel == \"Diff\": Rectangle",
        "if root.right-panel == \"Settings\": Rectangle",
        "if root.right-panel == \"MCP\": Rectangle",
        "if root.right-panel == \"Skills\": Rectangle",
        "height: 72px;",
        "height: 328px;",
        "height: 168px;",
    ] {
        assert!(
            APP_SLINT.contains(required),
            "V19 shell layout must include {required}"
        );
    }
    for forbidden in [
        "in property <bool> shown",
        "shown:",
        "width: 320px;",
        "min-width: 980px;",
        "height: root.right-panel == \"Settings\" ? root.height",
        "height: root.right-panel == \"MCP\" ? root.height",
        "height: root.right-panel == \"Skills\" ? root.height",
    ] {
        assert!(
            !APP_SLINT.contains(forbidden),
            "V19 shell layout must not include {forbidden}"
        );
    }
}

#[test]
fn t2_shell_removes_static_sample_rows() {
    for forbidden in [
        "Bedrock settings",
        "Remote SSM session",
        "Implement the runtime spine",
        "crates/fastrock-core/src/lib.rs",
        "ttl optional",
    ] {
        assert!(
            !APP_SLINT.contains(forbidden),
            "app shell must render workflow data, not static sample row: {forbidden}"
        );
    }
}

#[test]
fn t2_app_actions_do_not_use_fixed_demo_ids() {
    for forbidden in [
        "mantle-default",
        "runtime-default",
        "create_conversation(\"local\"",
        "create_conversation(\"remote\"",
        "create_conversation(\"goal\"",
    ] {
        assert!(
            !APP_MAIN.contains(forbidden),
            "app action path must allocate real IDs, not fixed demo ID: {forbidden}"
        );
    }
}

#[test]
fn v1_main_window_starts_before_persistence_hydration() {
    let window_pos = APP_MAIN
        .find("let app = AppWindow::new()?")
        .expect("main must construct Slint window");
    let hydration_pos = APP_MAIN
        .find("start_persistence_hydration(")
        .expect("main must start async persistence hydration");
    let before_window = &APP_MAIN[..window_pos];

    assert!(
        hydration_pos > window_pos,
        "persistence hydration must start after Slint window construction"
    );
    assert!(
        !before_window.contains("PersistenceActorHandle::spawn("),
        "main thread must not synchronously start persistence before window creation"
    );
    assert!(
        !before_window.contains("load_persisted_workflow("),
        "main thread must not load SQLite state before window creation"
    );
    assert!(
        !APP_MAIN.contains("block_on(load_persisted_workflow"),
        "main thread must not block on persistence hydration"
    );
    assert!(
        APP_MAIN.contains("tokio::task::spawn_blocking"),
        "persistence actor startup must move off the Slint event loop"
    );
    assert!(
        APP_MAIN.contains("slint::invoke_from_event_loop"),
        "background hydration must refresh Slint through event-loop bridge"
    );
}

#[test]
fn v1_persistence_client_is_available_before_state_read_finishes() {
    let client_slot_pos = APP_MAIN
        .find("*slot = Some(client.clone());")
        .expect("hydration must expose persistence client for GUI writes");
    let load_pos = APP_MAIN
        .find("load_persisted_workflow_from_client(&client).await")
        .expect("hydration must read persisted workflow through cloneable client");

    assert!(
        client_slot_pos < load_pos,
        "GUI writes must not stay blocked if persisted state hydration read fails"
    );
}

#[test]
fn v14_main_starts_project_watchers_after_hydration() {
    let load_pos = APP_MAIN
        .find("load_persisted_workflow_from_client(&client).await")
        .expect("hydration must load persisted workflow");
    let watcher_pos = APP_MAIN
        .find("start_project_watchers(")
        .expect("hydration must start project watchers");
    let refresh_pos = APP_MAIN
        .find("Persisted state loaded")
        .expect("hydration must refresh UI after loading state");

    assert!(
        load_pos < watcher_pos && watcher_pos < refresh_pos,
        "project watchers must start from hydrated project folders before loaded-state refresh"
    );
    for required in [
        "LocalProjectWatcher::spawn",
        "ProjectWatcherConfig::default()",
        "watchable_local_project_folders()",
        "handle_local_project_file_event",
        "persist_gui_action_writes(&persistence",
        "invoke_workflow_refresh(app_weak.clone(), workflow.clone(), status)",
    ] {
        assert!(
            APP_MAIN.contains(required),
            "main watcher path must include {required}"
        );
    }
}

#[test]
fn v2_startup_detects_rtk_off_slint_thread() {
    let window_pos = APP_MAIN
        .find("let app = AppWindow::new()?")
        .expect("main must construct Slint window");
    let diagnostic_pos = APP_MAIN
        .find("start_rtk_startup_diagnostic(&runtime, app.as_weak())")
        .expect("main must start RTK startup diagnostic");

    assert!(
        diagnostic_pos > window_pos,
        "RTK detection must not delay initial Slint window construction"
    );
    assert!(APP_MAIN.contains("diagnose_default_rtk_binary"));
    assert!(APP_MAIN.contains("tokio::task::spawn_blocking(diagnose_default_rtk_binary)"));
    assert!(APP_MAIN.contains("RTK missing:"));
    assert!(APP_MAIN.contains("Install rtk"));
}

#[test]
fn v15_main_installs_local_only_panic_hook() {
    let main_pos = APP_MAIN.find("fn main()").expect("main function exists");
    let hook_pos = APP_MAIN
        .find("install_local_panic_hook();")
        .expect("main must install panic hook");
    assert!(
        hook_pos > main_pos,
        "panic hook must be installed from main startup"
    );
    assert!(APP_MAIN.contains("panic::set_hook"));
    assert!(APP_MAIN.contains("write_local_panic_log"));
    assert!(APP_MAIN.contains("logs"));
    assert!(APP_MAIN.contains("panic.log"));
    assert!(APP_MAIN.contains("PANIC_LOG_MAX_BYTES"));
    assert!(APP_MAIN.contains("PANIC_LOG_ROTATIONS"));
    assert!(APP_MAIN.contains("rotate_local_log_file"));
    assert!(APP_MAIN.contains("rotated_log_path"));

    let lower_main = APP_MAIN.to_ascii_lowercase();
    for forbidden in [
        "reqwest::Client",
        "https://fastrock",
        "fastrock.cloud",
        "sentry",
        "posthog",
        "telemetry",
        "crash upload",
    ] {
        assert!(
            !lower_main.contains(&forbidden.to_ascii_lowercase()),
            "panic path must stay local-only and avoid {forbidden}"
        );
    }
}

#[test]
fn t2_gui_actions_persist_through_async_write_queue() {
    for required in [
        "handle_gui_action(&mut workflow",
        "persist_writes_async(",
        "persist_gui_action_writes",
        "PersistenceActorClient",
        "set_diff_rows",
    ] {
        assert!(
            APP_MAIN.contains(required),
            "main app action path must include persisted async action wiring: {required}"
        );
    }

    for forbidden in [
        "create_default_bedrock_profile(",
        "edit_selected_or_default_profile_name(",
        "delete_selected_or_default_profile(",
        "create_unique_conversation(",
        "open_editor_tab(",
        "create_goal(",
    ] {
        assert!(
            !APP_MAIN.contains(forbidden),
            "main app actions must not mutate workflow through memory-only helper: {forbidden}"
        );
    }
}

#[test]
fn t2_shell_snapshot_projects_workflow_state() {
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(BedrockProfileRequest {
            id: "mantle".to_owned(),
            name: "Mantle Real".to_owned(),
            provider: UiLlmProvider::BedrockMantle,
            model_id: "anthropic.claude-3-5-sonnet-20241022-v2:0".to_owned(),
            region: "us-west-2".to_owned(),
            aws_profile: "dev".to_owned(),
            default_for_new_conversations: true,
        })
        .unwrap();
    let conversation_id = workflow.create_conversation("conversation", "Build UI", "folder-local");
    workflow.create_goal(conversation_id.clone(), "finish shell", Some(1000), 10);
    assert!(workflow.update_goal_status(&conversation_id, ThreadGoalStatus::Paused, 20));
    workflow.open_editor_tab(EditorBuffer::from_text("src/main.rs", "fn main() {}\n"));

    let snapshot = workflow.shell_snapshot();

    assert_eq!(snapshot.active_conversation_title, "Build UI");
    assert_eq!(snapshot.active_profile, "Mantle Real");
    assert_eq!(snapshot.active_mode, "Code");
    assert!(!snapshot.plan_mode_enabled);
    assert_eq!(snapshot.conversation_rows.len(), 1);
    assert_eq!(
        snapshot.conversation_rows[0].conversation_id,
        "conversation"
    );
    assert_eq!(snapshot.conversation_rows[0].folder, "folder-local");
    assert_eq!(snapshot.conversation_rows[0].mode, "Code");
    assert!(snapshot.conversation_rows[0].status.contains("Paused"));
    assert_eq!(snapshot.file_tabs[0].path, "src/main.rs");
    assert!(snapshot.file_tabs[0].selected);
    assert_eq!(snapshot.editor_lines[0].line_number, 1);
    assert_eq!(snapshot.editor_lines[0].text, "fn main() {}");
    assert!(
        snapshot
            .transcript_blocks
            .iter()
            .any(|block| block.speaker == "goal" && block.body.contains("Paused"))
    );

    let settings = snapshot
        .settings_rows
        .iter()
        .map(|row| (row.name.as_str(), row.value.as_str()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(settings["Provider"], "Bedrock Mantle");
    assert_eq!(settings["AWS profile"], "dev");
    assert_eq!(settings["Mantle store"], "off (not retained)");
    assert_eq!(settings["Runtime target"], "foundation model");
    assert_eq!(settings["Execution"], "approval never");
    assert_eq!(settings["Mode"], "Code");
    assert_eq!(settings["Bedrock probes"], "models + test");
}

#[test]
fn t29_default_shell_snapshot_has_no_seed_rows() {
    let workflow = FastrockWorkflow::default();
    let snapshot = workflow.shell_snapshot();

    assert_eq!(snapshot.active_conversation_title, "No conversation");
    assert_eq!(snapshot.active_folder, "No folder");
    assert_eq!(snapshot.active_profile, "No profile");
    assert!(snapshot.conversation_rows.is_empty());
    assert!(snapshot.project_folders.is_empty());
    assert!(snapshot.sidebar_tree_rows.is_empty());
}

#[test]
fn t24_conversation_rows_select_visible_without_stopping_hidden_work() {
    let mut workflow = FastrockWorkflow::default();
    workflow.create_conversation("first", "First", "folder-a");
    let second = workflow.create_conversation("second", "Second", "folder-b");
    assert!(workflow.switch_visible_conversation(&second));

    let selected = handle_gui_action(&mut workflow, "conversation-select:first", 10);
    let snapshot = workflow.shell_snapshot();

    assert_eq!(selected.status, "Conversation selected: First");
    assert_eq!(snapshot.active_conversation_title, "First");
    assert!(snapshot.conversation_rows[0].selected);
    assert!(!snapshot.conversation_rows[1].selected);
    assert_eq!(
        workflow.scheduler().conversation(&second).unwrap().status,
        fastrock_core::ConversationStatus::Running
    );

    let missing = handle_gui_action(&mut workflow, "conversation-select:missing", 11);
    assert!(missing.status.contains("failed"));
}

#[test]
fn t13_plan_toggle_updates_active_conversation_mode_and_persists_event() {
    let mut workflow = FastrockWorkflow::default();
    let conversation_id = workflow.create_conversation("conversation", "Plan Work", "folder-local");

    let enabled = handle_gui_action(&mut workflow, "plan-toggle", 10);
    let snapshot = workflow.shell_snapshot();

    assert!(enabled.status.contains("Plan"));
    assert_eq!(
        workflow.conversation_mode(&conversation_id),
        fastrock_core::AgentMode::Plan
    );
    assert_eq!(snapshot.active_mode, "Plan");
    assert!(snapshot.plan_mode_enabled);
    assert_eq!(snapshot.conversation_rows[0].mode, "Plan");
    assert!(enabled.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::AppendConversationEvent(event)
                if event.event_type == "conversation.mode_changed"
                    && event.payload_json.contains(r#""mode":"Plan""#))
    ));

    let disabled = handle_gui_action(&mut workflow, "plan-toggle", 11);

    assert!(disabled.status.contains("Code"));
    assert_eq!(
        workflow.conversation_mode(&conversation_id),
        fastrock_core::AgentMode::Code
    );
    assert!(!workflow.shell_snapshot().plan_mode_enabled);
}

#[test]
fn t13_gui_plan_create_and_execute_appends_plan_transcript_and_switches_code_mode() {
    let mut workflow = FastrockWorkflow::default();
    let conversation_id =
        workflow.create_conversation("conversation", "Plan Execute", "folder-local");

    let created = handle_gui_action(
        &mut workflow,
        "plan-start:Review current diff and produce steps",
        10,
    );
    let created_snapshot = workflow.shell_snapshot();

    assert_eq!(created.status, "Plan artifact created");
    assert_eq!(
        workflow.conversation_mode(&conversation_id),
        fastrock_core::AgentMode::Plan
    );
    assert!(created_snapshot.plan_mode_enabled);
    assert!(
        created_snapshot
            .transcript_blocks
            .iter()
            .any(|block| block.speaker == "plan"
                && block.body.contains("Review current diff and produce steps"))
    );
    assert!(created.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::AppendConversationEvent(event)
                if event.event_type == "transcript.plan_created"
                    && event.payload_json.contains("Review current diff"))
    ));

    let executed = handle_gui_action(&mut workflow, "plan-execute", 11);
    let executed_snapshot = workflow.shell_snapshot();

    assert!(executed.status.starts_with("Plan executed: plan-"));
    assert_eq!(
        workflow.conversation_mode(&conversation_id),
        fastrock_core::AgentMode::Code
    );
    assert!(!executed_snapshot.plan_mode_enabled);
    assert!(
        executed_snapshot
            .transcript_blocks
            .iter()
            .any(|block| block.speaker == "plan" && block.body.starts_with("Executed plan"))
    );
    assert!(executed.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::AppendConversationEvent(event)
                if event.event_type == "transcript.plan_executed")
    ));
    assert!(executed.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::AppendConversationEvent(event)
                if event.event_type == "conversation.mode_changed"
                    && event.payload_json.contains(r#""mode":"Code""#))
    ));

    let unavailable = handle_gui_action(&mut workflow, "plan-execute", 12);
    assert_eq!(
        unavailable.status,
        "Plan execute unavailable: no draft plan"
    );
}

#[test]
fn t21_goal_controls_update_active_goal_and_emit_persistence_writes() {
    let mut workflow = FastrockWorkflow::default();
    let conversation_id = workflow.create_conversation("goal", "Goal", "folder-local");

    let missing = handle_gui_action(&mut workflow, "goal-pause", 9);
    assert_eq!(missing.status, "Goal unavailable: no active goal");
    assert!(missing.writes.is_empty());

    workflow.create_goal(conversation_id.clone(), "finish", Some(1000), 10);

    let paused = handle_gui_action(&mut workflow, "goal-pause", 11);
    assert_eq!(paused.status, "Goal paused");
    assert_eq!(
        workflow.goal_status(&conversation_id),
        Some(ThreadGoalStatus::Paused)
    );
    assert!(paused.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::UpsertGoal(goal)
            if goal.conversation_id == "goal" && goal.status == "Paused")
    ));

    let resumed = handle_gui_action(&mut workflow, "goal-resume", 12);
    assert_eq!(resumed.status, "Goal resumed");
    assert_eq!(
        workflow.goal_status(&conversation_id),
        Some(ThreadGoalStatus::Active)
    );
    assert!(resumed.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::UpsertGoal(goal)
            if goal.conversation_id == "goal" && goal.status == "Active")
    ));

    let completed = handle_gui_action(&mut workflow, "goal-complete", 13);
    assert_eq!(completed.status, "Goal complete");
    assert_eq!(
        workflow.goal_status(&conversation_id),
        Some(ThreadGoalStatus::Complete)
    );
    assert!(completed.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::UpsertGoal(goal)
            if goal.conversation_id == "goal" && goal.status == "Complete")
    ));

    let cleared = handle_gui_action(&mut workflow, "goal-clear", 14);
    assert_eq!(cleared.status, "Goal cleared");
    assert_eq!(workflow.goal_status(&conversation_id), None);
    assert!(cleared.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::DeleteGoal(conversation_id)
            if conversation_id == "goal")
    ));
}

#[test]
fn t11_conversation_send_uses_message_payload_and_persists_user_transcript() {
    let mut workflow = FastrockWorkflow::default();

    let blank = handle_gui_action(&mut workflow, "conversation-send:   ", 9);
    assert_eq!(blank.status, "Message empty");
    assert!(blank.writes.is_empty());

    let result = handle_gui_action(
        &mut workflow,
        "conversation-send:Investigate Mantle cache behavior",
        10,
    );
    let snapshot = workflow.shell_snapshot();

    assert_eq!(result.status, "Message queued");
    assert_eq!(workflow.scheduler().conversation_count(), 1);
    assert_eq!(snapshot.active_conversation_title, "New chat");
    assert!(snapshot.project_folders.is_empty());
    assert_eq!(snapshot.conversation_rows[0].folder, "Home");
    assert_eq!(snapshot.sidebar_tree_rows[0].kind, "unbound");
    assert!(
        snapshot
            .transcript_blocks
            .iter()
            .any(|block| block.speaker == "user"
                && block.body == "Investigate Mantle cache behavior")
    );
    assert!(result.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::AppendConversationEvent(event)
                if event.event_type == "transcript.user_message"
                    && event.payload_json.contains("Investigate Mantle cache behavior"))
    ));
    assert!(
        !result
            .writes
            .iter()
            .any(|write| matches!(write, GuiPersistenceWrite::UpsertGoal(_))),
        "first send must not create a goal-loop demo"
    );
}

#[test]
fn t11_payload_send_appends_to_selected_conversation_without_creating_new_one() {
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(BedrockProfileRequest {
            id: "mantle".to_owned(),
            name: "Mantle".to_owned(),
            provider: UiLlmProvider::BedrockMantle,
            model_id: "anthropic.claude-test".to_owned(),
            region: "us-east-1".to_owned(),
            aws_profile: "dev".to_owned(),
            default_for_new_conversations: true,
        })
        .unwrap();
    workflow.create_conversation("first", "First", "folder-a");
    workflow.create_conversation("second", "Second", "folder-b");
    let selected = handle_gui_action(&mut workflow, "conversation-select:first", 10);
    assert!(selected.status.contains("First"));

    let result = handle_gui_action(&mut workflow, "conversation-send:hello active thread", 11);
    let snapshot = workflow.shell_snapshot();

    assert_eq!(result.status, "Message queued");
    assert_eq!(workflow.scheduler().conversation_count(), 2);
    assert!(result.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::AppendConversationEvent(event)
                if event.conversation_id == "first"
                    && event.event_type == "transcript.user_message"
                    && event.payload_json.contains("hello active thread"))
    ));
    assert!(matches!(
        result.runtime_work.as_slice(),
        [GuiRuntimeWork::RunModel(request)]
            if request.conversation_id.0 == "first"
                && request.profile_id == "mantle"
                && request.message == "hello active thread"
                && request.prompt_compression_enabled
                && request.prompt_compression_level == "full"
    ));
    assert!(
        snapshot
            .transcript_blocks
            .iter()
            .any(|block| block.speaker == "user" && block.body == "hello active thread")
    );
}

#[test]
fn t10_mantle_store_is_permanently_disabled() {
    // SPEC §V5: Bedrock Mantle store is always off. The per-conversation store
    // override (and its GUI option) were removed; the wire request always sends
    // store=false (asserted in the release_workflow Mantle wire test).
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(BedrockProfileRequest {
            id: "mantle".to_owned(),
            name: "Mantle".to_owned(),
            provider: UiLlmProvider::BedrockMantle,
            model_id: "anthropic.claude-test".to_owned(),
            region: "us-east-1".to_owned(),
            aws_profile: "dev".to_owned(),
            default_for_new_conversations: true,
        })
        .unwrap();
    workflow.create_conversation("first", "First", "folder-a");

    // The read-only summary states the store is off and not retained.
    let settings = workflow
        .shell_snapshot()
        .settings_rows
        .into_iter()
        .map(|row| (row.name, row.value))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(settings["Mantle store"], "off (not retained)");
    assert!(!settings.contains_key("Mantle conversation store"));

    // The store-override option is gone from the UI and the action router.
    for removed in [
        "mantle-store-profile",
        "mantle-store-local",
        "mantle-store-on",
    ] {
        assert!(
            !APP_SLINT.contains(removed),
            "Mantle store option must be removed from the shell: {removed}"
        );
    }

    // Sending still produces a Mantle model run (with no store concept attached).
    let send = handle_gui_action(&mut workflow, "conversation-send:hello", 21);
    assert!(matches!(
        send.runtime_work.as_slice(),
        [GuiRuntimeWork::RunModel(request)] if request.profile_id == "mantle"
    ));
}

#[test]
fn t17_shell_snapshot_projects_configured_project_folders() {
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(BedrockProfileRequest {
            id: "mantle".to_owned(),
            name: "Mantle Real".to_owned(),
            provider: UiLlmProvider::BedrockMantle,
            model_id: "anthropic.claude-3-5-sonnet-20241022-v2:0".to_owned(),
            region: "us-west-2".to_owned(),
            aws_profile: "dev".to_owned(),
            default_for_new_conversations: true,
        })
        .unwrap();
    workflow.upsert_project_folder(ProjectFolderRecord {
        id: "folder-local".to_owned(),
        label: "Fastrock".to_owned(),
        target_kind: "local".to_owned(),
        path: "/Users/sean/dev/fastrock".to_owned(),
        remote_target_id: None,
        document_json: serde_json::json!({
            "default_profile_id": "mantle",
            "recent_conversation_ids": ["conversation-old"]
        })
        .to_string(),
        updated_at_ms: 10,
    });
    workflow.upsert_project_folder(ProjectFolderRecord {
        id: "folder-ssh".to_owned(),
        label: "Remote API".to_owned(),
        target_kind: "ssh".to_owned(),
        path: "/srv/api".to_owned(),
        remote_target_id: Some("prod-bastion".to_owned()),
        document_json: serde_json::json!({
            "recent_conversation_ids": []
        })
        .to_string(),
        updated_at_ms: 11,
    });

    let snapshot = workflow.shell_snapshot();

    assert_eq!(snapshot.project_folders.len(), 2);
    assert_eq!(snapshot.project_folders[0].label, "Fastrock");
    assert_eq!(snapshot.project_folders[0].target, "local");
    assert_eq!(snapshot.project_folders[0].default_profile, "Mantle Real");
    assert_eq!(snapshot.project_folders[0].recent_conversations, 1);
    assert_eq!(snapshot.project_folders[1].label, "Remote API");
    assert_eq!(snapshot.project_folders[1].target, "ssh:prod-bastion");
    assert_eq!(
        snapshot.project_folders[1].default_profile,
        "No default profile"
    );
    assert_eq!(snapshot.sidebar_tree_rows.len(), 2);
    assert_eq!(snapshot.sidebar_tree_rows[0].kind, "folder");
    assert_eq!(snapshot.sidebar_tree_rows[0].title, "Fastrock");
    assert_eq!(
        snapshot.sidebar_tree_rows[0].action,
        "sidebar-toggle-folder:666f6c6465722d6c6f63616c"
    );
    assert_eq!(snapshot.sidebar_tree_rows[1].kind, "folder");
    assert_eq!(snapshot.sidebar_tree_rows[1].title, "Remote API");
}

#[test]
fn t25_error_transcript_blocks_show_operational_fields_without_secret_leak() {
    let mut workflow = FastrockWorkflow::default();
    workflow.create_conversation("conversation", "Errors", "folder-local");
    let error = FastrockError::new(
        FastrockErrorKind::ProviderAuth,
        "api_key=sk-test token=secret aws_secret_access_key=abc123",
    )
    .with_profile_id("profile-1");

    workflow
        .append_transcript_event_to_visible(TranscriptEventKind::Error {
            error: Box::new(error),
        })
        .unwrap();

    let snapshot = workflow.shell_snapshot();
    let block = snapshot
        .transcript_blocks
        .iter()
        .find(|block| block.speaker == "error")
        .expect("error block must render");

    assert!(block.body.contains("FR_PROVIDER_AUTH"));
    assert!(block.body.contains("Provider authentication failed"));
    assert!(block.body.contains("not retryable"));
    assert!(block.body.contains("source=Bedrock"));
    assert!(block.body.contains("profile=profile-1"));
    assert!(block.body.contains("Action: Check credentials"));
    assert!(block.body.contains("[REDACTED]"));
    assert!(!block.body.contains("sk-test"));
    assert!(!block.body.contains("abc123"));
    assert_eq!(block.accent_kind, "error");
}

#[test]
fn t22_shell_snapshot_projects_active_editor_visible_lines() {
    let mut workflow = FastrockWorkflow::default();
    let text = (0..100)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    workflow.open_editor_tab(EditorBuffer::from_text("large.txt", &text));

    let snapshot = workflow.shell_snapshot();

    assert!(!snapshot.editor_lines.is_empty());
    assert!(
        snapshot.editor_lines.len() <= 24,
        "shell snapshot must expose only a bounded visible editor range"
    );
    assert_eq!(snapshot.editor_lines[0].line_number, 1);
    assert_eq!(snapshot.editor_lines[0].text, "line 0");
    assert!(
        !snapshot
            .editor_lines
            .iter()
            .any(|line| line.text == "line 99"),
        "shell snapshot must not expose the whole file"
    );
}

#[test]
fn t22_editor_tab_actions_select_and_close_visible_buffers() {
    let mut workflow = FastrockWorkflow::default();
    workflow.open_editor_tab(EditorBuffer::from_text("first.rs", "first\n"));
    workflow.open_editor_tab(EditorBuffer::from_text("second.rs", "second\n"));
    assert_eq!(workflow.shell_snapshot().editor_lines[0].text, "second");

    let selected = handle_gui_action(&mut workflow, "editor-select:first.rs", 10);
    let selected_snapshot = workflow.shell_snapshot();

    assert_eq!(selected.status, "Editor tab selected: first.rs");
    assert_eq!(selected_snapshot.editor_lines[0].text, "first");
    assert!(selected_snapshot.file_tabs[0].selected);
    assert!(selected.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::UpsertEditorTab(tab)
            if tab.path == "first.rs" && tab.active)
    ));

    let closed = handle_gui_action(&mut workflow, "editor-close:first.rs", 11);
    let closed_snapshot = workflow.shell_snapshot();

    assert_eq!(closed.status, "Editor tab closed: first.rs");
    assert_eq!(closed_snapshot.file_tabs.len(), 1);
    assert_eq!(closed_snapshot.file_tabs[0].path, "second.rs");
    assert_eq!(closed_snapshot.editor_lines[0].text, "second");
    assert!(closed.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::DeleteEditorTab(path)
            if path == "first.rs")
    ));
}

#[test]
fn t22_editor_actions_edit_save_find_undo_redo_active_buffer() {
    let mut workflow = FastrockWorkflow::default();
    workflow.open_editor_tab(EditorBuffer::from_text("note.txt", "alpha\n"));

    let appended = handle_gui_action(&mut workflow, "editor-append:beta", 10);
    let snapshot = workflow.shell_snapshot();

    assert_eq!(appended.status, "Editor appended: note.txt dirty=true");
    assert_eq!(snapshot.editor_lines[0].text, "alpha");
    assert_eq!(snapshot.editor_lines[1].text, "beta");
    assert_eq!(snapshot.diff_rows.len(), 1);
    assert_eq!(snapshot.diff_rows[0].line_number, 2);
    assert_eq!(snapshot.diff_rows[0].before, "");
    assert_eq!(snapshot.diff_rows[0].after, "beta");
    assert!(snapshot.file_tabs[0].dirty);
    assert!(appended.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::UpsertEditorTab(tab)
            if tab.path == "note.txt" && tab.document_json.contains("\"dirty\":true"))
    ));

    let found = handle_gui_action(&mut workflow, "editor-find:beta", 11);
    assert_eq!(found.status, "Editor find: 1 matches in note.txt");

    let jumped = handle_gui_action(&mut workflow, "editor-goto:2", 12);
    assert_eq!(jumped.status, "Editor goto: note.txt:2");

    let undone = handle_gui_action(&mut workflow, "editor-undo", 13);
    let undo_snapshot = workflow.shell_snapshot();
    assert_eq!(undone.status, "Editor undo: note.txt dirty=false");
    assert_eq!(undo_snapshot.editor_lines[0].text, "alpha");
    assert!(undo_snapshot.diff_rows.is_empty());
    assert!(!undo_snapshot.file_tabs[0].dirty);

    let redone = handle_gui_action(&mut workflow, "editor-redo", 14);
    let redo_snapshot = workflow.shell_snapshot();
    assert_eq!(redone.status, "Editor redo: note.txt dirty=true");
    assert_eq!(redo_snapshot.editor_lines[1].text, "beta");
    assert_eq!(redo_snapshot.diff_rows[0].after, "beta");

    let reverted = handle_gui_action(&mut workflow, "editor-revert", 15);
    let revert_snapshot = workflow.shell_snapshot();
    assert_eq!(reverted.status, "Editor reverted: note.txt dirty=false");
    assert_eq!(revert_snapshot.editor_lines[0].text, "alpha");
    assert!(revert_snapshot.diff_rows.is_empty());
    assert!(!revert_snapshot.file_tabs[0].dirty);

    let appended_again = handle_gui_action(&mut workflow, "editor-append:beta", 16);
    assert_eq!(
        appended_again.status,
        "Editor appended: note.txt dirty=true"
    );

    let diffed = handle_gui_action(&mut workflow, "editor-diff", 17);
    assert_eq!(diffed.status, "Editor diff: 1 changed lines in note.txt");

    let saved = handle_gui_action(&mut workflow, "editor-save", 18);
    let save_snapshot = workflow.shell_snapshot();
    assert_eq!(saved.status, "Editor save queued: note.txt");
    assert!(save_snapshot.diff_rows.is_empty());
    assert!(!save_snapshot.file_tabs[0].dirty);
    assert!(saved.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::SaveEditorFile { path, contents }
            if path == "note.txt" && contents.contains("beta"))
    ));
}

#[test]
fn t18_t19_shell_snapshot_projects_mcp_and_skill_rows() {
    let mut workflow = FastrockWorkflow::default();
    let server = McpServerConfig {
        id: "filesystem".to_owned(),
        name: "Filesystem".to_owned(),
        scope: Default::default(),
        enabled: true,
        transport: McpTransportConfig::Stdio {
            command: "rtk".to_owned(),
            args: vec!["mcp".to_owned()],
            cwd: None,
            env: BTreeMap::new(),
        },
        timeout_ms: 30_000,
        always_allow_tools: BTreeSet::from(["read_file".to_owned()]),
        disabled_tools: BTreeSet::from(["delete_file".to_owned()]),
        oauth: None,
    };
    workflow
        .configure_mcp(McpConfigSet {
            servers: vec![server.clone()],
        })
        .unwrap();
    workflow.record_mcp_status(
        McpRuntimeSnapshot::stopped(&server, 10)
            .with_discovery(
                ["read_file", "write_file"],
                ["file://repo"],
                ["file://{path}"],
            )
            .mark_running(11),
    );

    let skills_root = tempdir().unwrap();
    let mut request = SkillWriteRequest::new("rust", "Rust workflow", "Use cargo test.\n");
    request.mode_slugs = vec!["code".to_owned()];
    workflow
        .create_skill(
            skills_root.path(),
            SkillScope::Project("folder-local".to_owned()),
            request,
        )
        .unwrap();

    let snapshot = workflow.shell_snapshot();

    assert_eq!(snapshot.mcp_servers.len(), 1);
    assert_eq!(snapshot.mcp_servers[0].server_id, "filesystem");
    assert_eq!(snapshot.mcp_servers[0].name, "Filesystem");
    assert_eq!(snapshot.mcp_servers[0].scope, "global");
    assert_eq!(snapshot.mcp_servers[0].transport, "stdio:rtk");
    assert_eq!(snapshot.mcp_servers[0].status, "running");
    assert!(
        snapshot.mcp_servers[0]
            .details
            .contains("2 tools / 1 resources / 1 templates")
    );
    assert_eq!(snapshot.mcp_tools.len(), 3);
    let read_tool = snapshot
        .mcp_tools
        .iter()
        .find(|tool| tool.tool_name == "read_file")
        .unwrap();
    assert_eq!(read_tool.policy, "always allowed");
    assert!(read_tool.discovered);
    assert!(read_tool.always_allowed);
    assert!(read_tool.allow_action.starts_with("mcp-tool-allow:"));
    let delete_tool = snapshot
        .mcp_tools
        .iter()
        .find(|tool| tool.tool_name == "delete_file")
        .unwrap();
    assert_eq!(delete_tool.policy, "disabled");
    assert!(!delete_tool.discovered);
    assert!(delete_tool.disabled);
    assert!(delete_tool.disable_action.starts_with("mcp-tool-disable:"));
    assert_eq!(snapshot.mcp_discovery.len(), 2);
    assert!(
        snapshot
            .mcp_discovery
            .iter()
            .any(|item| item.kind == "resource" && item.name == "file://repo")
    );
    assert!(
        snapshot
            .mcp_discovery
            .iter()
            .any(|item| item.kind == "template" && item.name == "file://{path}")
    );
    assert_eq!(snapshot.skills.len(), 1);
    assert_eq!(snapshot.skills[0].name, "rust");
    assert_eq!(snapshot.skills[0].scope, "project:folder-local");
    assert_eq!(snapshot.skills[0].modes, "code");
    assert_eq!(snapshot.skills[0].status, "enabled");
}

#[test]
fn t18_gui_mcp_actions_manage_servers_and_hot_reload_status() {
    let mut workflow = FastrockWorkflow::default();

    let added = handle_gui_action(&mut workflow, "mcp-add", 100);
    assert!(added.status.contains("MCP server added"));
    assert_eq!(workflow.mcp_config().servers.len(), 1);
    let server_id = workflow.mcp_config().servers[0].id.clone();
    assert!(added.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::StoreSettingsDocument(document)
            if document.key == "mcp.config" && document.document_json.contains(&server_id))
    ));
    assert_eq!(
        workflow.shell_snapshot().mcp_servers[0].server_id,
        server_id
    );
    let server = workflow.mcp_config().servers[0].clone();
    workflow.record_mcp_status(
        McpRuntimeSnapshot::stopped(&server, 100)
            .with_discovery(["read_file"], Vec::<&str>::new(), Vec::<&str>::new())
            .mark_running(100),
    );
    let allow_action = workflow.shell_snapshot().mcp_tools[0].allow_action.clone();
    let allowed = handle_gui_action(&mut workflow, &allow_action, 100);
    assert!(allowed.status.contains("always allowed"));
    assert!(
        workflow.mcp_config().servers[0]
            .always_allow_tools
            .contains("read_file")
    );
    assert!(allowed.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::StoreSettingsDocument(document)
            if document.key == "mcp.config" && document.document_json.contains("read_file"))
    ));

    let disable_action = workflow.shell_snapshot().mcp_tools[0]
        .disable_action
        .clone();
    let disabled_tool = handle_gui_action(&mut workflow, &disable_action, 100);
    assert!(disabled_tool.status.contains("disabled"));
    assert!(
        workflow.mcp_config().servers[0]
            .disabled_tools
            .contains("read_file")
    );
    assert!(
        !workflow.mcp_config().servers[0]
            .always_allow_tools
            .contains("read_file")
    );

    let edited = handle_gui_action(&mut workflow, &format!("mcp-edit:{server_id}"), 101);
    assert!(edited.status.contains("MCP server edited"));
    assert!(workflow.mcp_config().servers[0].name.contains("Edited"));

    let disabled = handle_gui_action(&mut workflow, &format!("mcp-toggle:{server_id}"), 102);
    assert!(disabled.status.contains("disabled"));
    assert!(!workflow.mcp_config().servers[0].enabled);
    assert_eq!(workflow.shell_snapshot().mcp_servers[0].status, "disabled");

    let restart = handle_gui_action(&mut workflow, &format!("mcp-restart:{server_id}"), 103);
    assert_eq!(workflow.mcp_status(&server_id).unwrap().restart_count, 1);
    assert!(restart.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::UpsertMcpStatus(status)
            if status.server_id == server_id && status.status == "starting")
    ));

    let deleted = handle_gui_action(&mut workflow, &format!("mcp-delete:{server_id}"), 104);
    assert!(deleted.status.contains("MCP server deleted"));
    assert!(workflow.mcp_config().servers.is_empty());
    assert!(workflow.mcp_status(&server_id).is_none());
    assert!(deleted.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::DeleteMcpStatus(id)
            if id == &server_id)
    ));
}

#[test]
fn t18_gui_command_policy_actions_update_persisted_settings() {
    fn command_policy_setting(workflow: &FastrockWorkflow) -> String {
        workflow
            .shell_snapshot()
            .settings_rows
            .into_iter()
            .find(|row| row.name == "Command policy")
            .unwrap()
            .value
    }

    let mut workflow = FastrockWorkflow::default();

    let disabled = handle_gui_action(&mut workflow, "command-policy-disabled", 300);
    assert_eq!(disabled.status, "Command policy set: disabled");
    assert!(!workflow.command_policy().commands_enabled);
    assert_eq!(command_policy_setting(&workflow), "commands disabled");
    assert!(disabled.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::StoreSettingsDocument(document)
            if document.key == "command.policy"
                && document.updated_at_ms == 300
                && document.document_json.contains(r#""commands_enabled":false"#))
    ));

    let locked = handle_gui_action(&mut workflow, "command-policy-locked", 301);
    assert_eq!(locked.status, "Command policy set: locked down");
    assert!(workflow.command_policy().commands_enabled);
    assert_eq!(workflow.command_policy().allow_prefixes.len(), 3);
    assert_eq!(workflow.command_policy().max_concurrent_commands, Some(1));
    let locked_display = command_policy_setting(&workflow);
    assert!(locked_display.contains("allowlist 3 prefixes"));
    assert!(locked_display.contains("runtime cap 120000 ms"));
    assert!(locked_display.contains("output cap 65536 bytes"));
    assert!(locked_display.contains("concurrency cap 1"));
    assert!(locked.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::StoreSettingsDocument(document)
            if document.key == "command.policy"
                && document.updated_at_ms == 301
                && document.document_json.contains(r#""max_concurrent_commands":1"#)
                && document.document_json.contains(r#"["rtk","git","status"]"#))
    ));

    let defaulted = handle_gui_action(&mut workflow, "command-policy-default", 302);
    assert_eq!(defaulted.status, "Command policy set: default");
    assert_eq!(workflow.command_policy(), &CommandPolicy::default());
    assert_eq!(
        command_policy_setting(&workflow),
        "commands enabled; outside-project labels on"
    );
    assert!(defaulted.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::StoreSettingsDocument(document)
            if document.key == "command.policy"
                && document.updated_at_ms == 302
                && document.document_json.contains(r#""commands_enabled":true"#))
    ));
}

#[test]
fn t30_gui_app_preferences_update_persisted_settings() {
    let mut workflow = FastrockWorkflow::default();
    let snapshot = workflow.shell_snapshot();
    assert!(snapshot.rtk_enabled);
    assert!(snapshot.prompt_compression_enabled);
    assert_eq!(snapshot.prompt_compression_level, "Full");
    assert_eq!(snapshot.send_shortcut, "control-enter");
    assert_eq!(snapshot.send_shortcut_label, "Ctrl+Enter");

    let rtk = handle_gui_action(&mut workflow, "settings-toggle-rtk", 400);
    assert_eq!(rtk.status, "RTK disabled");
    assert!(!workflow.app_preferences().rtk_enabled);
    assert!(rtk.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::StoreSettingsDocument(document)
            if document.key == "app.preferences"
                && document.updated_at_ms == 400
                && document.document_json.contains(r#""rtk_enabled":false"#))
    ));

    let compression = handle_gui_action(&mut workflow, "settings-toggle-prompt-compression", 401);
    assert_eq!(compression.status, "Prompt compression disabled");
    assert!(!workflow.app_preferences().prompt_compression_enabled);

    let level = handle_gui_action(&mut workflow, "settings-compression-level:ultra", 402);
    assert_eq!(level.status, "Prompt compression level set: Ultra");
    assert_eq!(workflow.shell_snapshot().prompt_compression_level, "Ultra");

    let shortcut = handle_gui_action(&mut workflow, "settings-send-shortcut:meta-enter", 403);
    assert_eq!(shortcut.status, "Send shortcut set: Meta+Enter");
    assert_eq!(workflow.shell_snapshot().send_shortcut, "meta-enter");
    assert_eq!(workflow.shell_snapshot().send_shortcut_label, "Meta+Enter");

    let rows = workflow
        .shell_snapshot()
        .settings_rows
        .into_iter()
        .map(|row| (row.name, row.value))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(rows["RTK"], "disabled");
    assert_eq!(rows["Prompt compression"], "disabled");
    assert_eq!(rows["Compression level"], "Ultra");
    assert_eq!(rows["Send shortcut"], "Meta+Enter");
}

#[test]
fn t19_gui_skill_actions_create_edit_and_delete_editor_backed_skills() {
    let mut workflow = FastrockWorkflow::default();

    let created = handle_gui_action(&mut workflow, "skill-create", 200);
    assert_eq!(workflow.skills().len(), 1);
    let skill_name = workflow.skills()[0].manifest.name.clone();
    assert_eq!(created.status, format!("Skill created: {skill_name}"));
    assert_eq!(workflow.shell_snapshot().skills[0].name, skill_name);
    assert!(
        workflow
            .shell_snapshot()
            .file_tabs
            .iter()
            .any(|tab| tab.path.ends_with("SKILL.md"))
    );

    let mode = handle_gui_action(&mut workflow, &format!("skill-mode:{skill_name}"), 201);
    assert!(mode.status.contains("Skill modes set"));
    assert_eq!(
        workflow.skills()[0].manifest.mode_slugs,
        vec!["code".to_owned()]
    );
    assert_eq!(workflow.shell_snapshot().skills[0].modes, "code");
    assert!(mode.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::UpsertEditorTab(record)
            if record.path.ends_with("SKILL.md"))
    ));
    assert!(
        workflow
            .shell_snapshot()
            .editor_lines
            .iter()
            .any(|line| line.text.contains("modes: [code]"))
    );

    let scope = handle_gui_action(&mut workflow, &format!("skill-scope:{skill_name}"), 202);
    assert!(scope.status.contains("project:folder-local"));
    assert_eq!(
        workflow.shell_snapshot().skills[0].scope,
        "project:folder-local"
    );

    let edited = handle_gui_action(&mut workflow, &format!("skill-edit:{skill_name}"), 203);
    assert_eq!(
        edited.status,
        format!("Skill opened for edit: {skill_name}")
    );
    assert!(workflow.skills()[0].manifest.description.contains("Edited"));
    assert!(
        workflow
            .shell_snapshot()
            .editor_lines
            .iter()
            .any(|line| line.text.contains("Updated from Fastrock GUI"))
    );

    let deleted = handle_gui_action(&mut workflow, &format!("skill-delete:{skill_name}"), 204);
    assert_eq!(deleted.status, format!("Skill deleted: {skill_name}"));
    assert!(workflow.skills().is_empty());
    assert!(workflow.shell_snapshot().skills.is_empty());
    assert!(deleted.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::DeleteEditorTab(path)
            if path.ends_with("SKILL.md"))
    ));
}

#[test]
fn t20_gui_memory_rows_inspect_toggle_and_delete_records() {
    let mut workflow = FastrockWorkflow::default();
    workflow.remember_memory(MemoryRecord::new(
        "memory-1",
        MemoryKind::ProjectFact,
        MemoryScope::Project {
            project_folder_id: "folder-local".to_owned(),
        },
        "Use Bedrock Mantle store=false when local state must stay local.",
        "SPEC.md:20",
        300,
    ));

    let snapshot = workflow.shell_snapshot();
    assert_eq!(snapshot.memory_rows.len(), 1);
    assert_eq!(snapshot.memory_rows[0].record_id, "memory-1");
    assert_eq!(snapshot.memory_rows[0].kind, "project fact");
    assert_eq!(snapshot.memory_rows[0].scope, "project:folder-local");
    assert_eq!(snapshot.memory_rows[0].status, "enabled");
    assert!(snapshot.memory_rows[0].preview.contains("Bedrock Mantle"));

    let inspected = handle_gui_action(&mut workflow, "memory-inspect:memory-1", 301);
    assert_eq!(inspected.status, "Memory opened: memory-1");
    assert!(
        workflow
            .shell_snapshot()
            .editor_lines
            .iter()
            .any(|line| line.text.contains("store=false"))
    );
    assert!(inspected.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::UpsertEditorTab(record)
            if record.path == "memory://memory-1")
    ));

    let disabled = handle_gui_action(&mut workflow, "memory-toggle:memory-1", 302);
    assert_eq!(disabled.status, "Memory disabled: memory-1");
    assert_eq!(workflow.shell_snapshot().memory_rows[0].status, "disabled");
    assert!(disabled.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::UpsertMemoryRecord(record)
            if record.id == "memory-1" && !record.enabled && record.updated_at_ms == 302)
    ));

    let deleted = handle_gui_action(&mut workflow, "memory-delete:memory-1", 303);
    assert_eq!(deleted.status, "Memory deleted: memory-1");
    assert!(workflow.shell_snapshot().memory_rows.is_empty());
    assert!(deleted.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::DeleteMemoryRecord(id)
            if id == "memory-1")
    ));
}

#[test]
fn t12_profile_probe_actions_report_selected_profile_endpoint() {
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(BedrockProfileRequest {
            id: "mantle".to_owned(),
            name: "Mantle Real".to_owned(),
            provider: UiLlmProvider::BedrockMantle,
            model_id: "anthropic.claude-3-5-sonnet-20241022-v2:0".to_owned(),
            region: "us-west-2".to_owned(),
            aws_profile: "dev".to_owned(),
            default_for_new_conversations: true,
        })
        .unwrap();

    let discovery = handle_gui_action(&mut workflow, "profile-discover", 10);
    assert!(discovery.status.contains("Model discovery ready"));
    assert!(
        discovery
            .status
            .contains("bedrock-mantle.us-west-2.api.aws")
    );
    assert!(discovery.status.contains("/models"));

    let test = handle_gui_action(&mut workflow, "profile-test", 11);
    assert!(test.status.contains("Mantle test connection ready"));
    assert!(test.status.contains("Mantle Real"));

    workflow
        .create_bedrock_profile(BedrockProfileRequest {
            id: "runtime".to_owned(),
            name: "Runtime Real".to_owned(),
            provider: UiLlmProvider::BedrockRuntime,
            model_id: "amazon.nova-pro-v1:0".to_owned(),
            region: "us-east-1".to_owned(),
            aws_profile: "runtime".to_owned(),
            default_for_new_conversations: true,
        })
        .unwrap();

    let runtime_test = handle_gui_action(&mut workflow, "profile-test", 12);
    assert!(
        runtime_test
            .status
            .contains("Runtime test connection ready")
    );
    assert!(
        runtime_test
            .status
            .contains("bedrock-runtime.us-east-1.amazonaws.com")
    );
    assert!(runtime_test.status.contains("/converse"));
    assert_eq!(runtime_test.writes.len(), 0);
}

#[test]
fn t6_workflow_duplicates_selected_profile_into_shell_state() {
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(BedrockProfileRequest {
            id: "mantle".to_owned(),
            name: "Mantle".to_owned(),
            provider: UiLlmProvider::BedrockMantle,
            model_id: "anthropic.claude-3-5-sonnet-20241022-v2:0".to_owned(),
            region: "us-east-1".to_owned(),
            aws_profile: "default".to_owned(),
            default_for_new_conversations: true,
        })
        .unwrap();

    let duplicate = workflow.duplicate_selected_or_default_profile().unwrap();

    assert_ne!(duplicate.id, "mantle");
    assert!(is_ulid(&duplicate.id));
    assert_eq!(duplicate.name, "Mantle Copy");
    assert_eq!(workflow.profiles().profiles().len(), 2);
    assert_eq!(workflow.shell_snapshot().active_profile, "Mantle Copy");
}

#[test]
fn t6_workflow_repeated_profile_actions_allocate_unique_state() {
    let mut workflow = FastrockWorkflow::default();

    let first_mantle = workflow
        .create_default_bedrock_profile(UiLlmProvider::BedrockMantle)
        .unwrap();
    let second_mantle = workflow
        .create_default_bedrock_profile(UiLlmProvider::BedrockMantle)
        .unwrap();
    let runtime = workflow
        .create_default_bedrock_profile(UiLlmProvider::BedrockRuntime)
        .unwrap();

    assert_ne!(first_mantle.id, second_mantle.id);
    assert!(is_ulid(&first_mantle.id));
    assert!(is_ulid(&second_mantle.id));
    assert!(is_ulid(&runtime.id));
    assert_eq!(first_mantle.name, "Bedrock Mantle");
    assert_eq!(second_mantle.name, "Bedrock Mantle 2");
    assert_eq!(runtime.name, "Bedrock Runtime");

    let edited = workflow.edit_selected_or_default_profile_name().unwrap();
    assert_eq!(edited.name, "Bedrock Runtime Edited");
    assert_eq!(
        workflow.delete_selected_or_default_profile(),
        Some(edited.id.clone())
    );
    assert!(workflow.profiles().profile(&edited.id).is_none());
    assert_eq!(workflow.profiles().profiles().len(), 2);
}

#[test]
fn t6_gui_profile_rows_select_toggle_default_and_delete_profiles() {
    let mut workflow = FastrockWorkflow::default();

    let mantle = handle_gui_action(&mut workflow, "profile-add-mantle", 10);
    assert!(mantle.status.contains("Mantle profile ready"));
    let runtime = handle_gui_action(&mut workflow, "profile-add-runtime", 11);
    assert!(runtime.status.contains("Runtime profile saved"));

    let snapshot = workflow.shell_snapshot();
    assert_eq!(snapshot.profile_rows.len(), 2);
    assert_eq!(snapshot.profile_rows[0].provider, "Bedrock Mantle");
    assert_eq!(snapshot.profile_rows[1].provider, "Bedrock Runtime");
    assert!(snapshot.profile_rows[0].default_for_new_conversations);
    assert!(
        snapshot
            .profile_rows
            .iter()
            .all(|row| row.validation == "valid")
    );

    let runtime_id = snapshot.profile_rows[1].profile_id.clone();
    let selected = handle_gui_action(&mut workflow, &format!("profile-select:{runtime_id}"), 12);
    assert_eq!(selected.status, format!("Profile selected: {runtime_id}"));
    assert!(
        workflow
            .shell_snapshot()
            .profile_rows
            .iter()
            .any(|row| row.profile_id == runtime_id && row.selected)
    );

    let disabled = handle_gui_action(&mut workflow, &format!("profile-toggle:{runtime_id}"), 13);
    assert!(disabled.status.contains("Profile disabled"));
    assert!(disabled.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::UpsertLlmProfile(profile)
            if profile.id == runtime_id && !profile.enabled)
    ));
    assert!(!workflow.profiles().profile(&runtime_id).unwrap().enabled);

    let defaulted = handle_gui_action(&mut workflow, &format!("profile-default:{runtime_id}"), 14);
    assert!(defaulted.status.contains("Default profile set"));
    assert_eq!(
        defaulted
            .writes
            .iter()
            .filter(|write| matches!(write, GuiPersistenceWrite::UpsertLlmProfile(_)))
            .count(),
        2
    );
    let defaults = workflow
        .profiles()
        .profiles()
        .iter()
        .filter(|profile| profile.default_for_new_conversations)
        .count();
    assert_eq!(defaults, 1);
    assert!(
        workflow
            .profiles()
            .profile(&runtime_id)
            .unwrap()
            .default_for_new_conversations
    );

    let deleted = handle_gui_action(&mut workflow, "profile-delete", 15);
    assert!(deleted.status.contains(&runtime_id));
    assert!(deleted.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::DeleteLlmProfile(id)
            if id == &runtime_id)
    ));
    assert!(workflow.profiles().profile(&runtime_id).is_none());
}

#[test]
fn t6_gui_profile_drafts_do_not_persist_until_save() {
    let mut workflow = FastrockWorkflow::default();

    let draft = handle_gui_action(&mut workflow, "profile-draft-mantle", 20);
    assert!(draft.status.contains("Mantle profile draft"));
    assert!(draft.writes.is_empty());
    assert!(workflow.profiles().profiles().is_empty());
    assert!(workflow.profiles().draft().is_some());
    let draft_snapshot = workflow.shell_snapshot();
    assert_eq!(draft_snapshot.profile_rows.len(), 1);
    assert_eq!(draft_snapshot.profile_rows[0].validation, "unsaved");
    let settings = draft_snapshot
        .settings_rows
        .iter()
        .map(|row| (row.name.as_str(), row.value.as_str()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert!(settings["Profile draft"].contains("unsaved"));

    let saved = handle_gui_action(&mut workflow, "profile-save", 21);
    assert!(saved.status.contains("Profile saved"));
    assert_eq!(workflow.profiles().profiles().len(), 1);
    assert!(workflow.profiles().draft().is_none());
    assert!(saved.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::UpsertLlmProfile(profile)
            if profile.provider == "bedrock_mantle")
    ));

    let profile_id = workflow.profiles().profiles()[0].id.clone();
    let saved_name = workflow.profiles().profiles()[0].name.clone();
    let edit = handle_gui_action(&mut workflow, "profile-edit", 22);
    assert!(edit.status.contains("Profile edit draft"));
    assert!(edit.writes.is_empty());
    assert_eq!(
        workflow.profiles().profile(&profile_id).unwrap().name,
        saved_name
    );
    assert!(workflow.profiles().draft().unwrap().name.contains("Edited"));

    let saved_edit = handle_gui_action(&mut workflow, "profile-save", 23);
    assert!(saved_edit.status.contains("Profile saved"));
    assert!(
        workflow
            .profiles()
            .profile(&profile_id)
            .unwrap()
            .name
            .contains("Edited")
    );
}

#[test]
fn t6_profile_tuning_actions_persist_selected_profile() {
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(BedrockProfileRequest {
            id: "profile-1".to_owned(),
            name: "Mantle".to_owned(),
            provider: UiLlmProvider::BedrockMantle,
            model_id: "anthropic.claude-test".to_owned(),
            region: "us-east-1".to_owned(),
            aws_profile: "default".to_owned(),
            default_for_new_conversations: true,
        })
        .unwrap();

    let max = handle_gui_action(&mut workflow, "profile-tuning-max-4096", 30);
    assert_eq!(max.status, "Request max output set: 4096");
    assert!(max.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::UpsertLlmProfile(profile)
            if profile.id == "profile-1" && profile.document_json.contains("\"max_output_tokens\":4096"))
    ));
    assert_eq!(
        handle_gui_action(&mut workflow, "profile-tuning-temp-02", 31).status,
        "Request temperature set: 0.2"
    );
    assert_eq!(
        handle_gui_action(&mut workflow, "profile-tuning-topp-09", 32).status,
        "Request top-p set: 0.9"
    );
    assert_eq!(
        handle_gui_action(&mut workflow, "profile-tuning-timeout-120s", 33).status,
        "Request timeout set: 120s"
    );
    assert_eq!(
        handle_gui_action(&mut workflow, "profile-tuning-retry-2", 34).status,
        "Request retry set: 2"
    );

    let profile = workflow.profiles().profile("profile-1").unwrap();
    assert_eq!(profile.request_tuning.max_output_tokens, Some(4096));
    assert_eq!(profile.request_tuning.temperature_milli, Some(200));
    assert_eq!(profile.request_tuning.top_p_milli, Some(900));
    assert_eq!(profile.request_tuning.timeout_ms, Some(120_000));
    assert_eq!(profile.request_tuning.retry_max_attempts, Some(2));
    assert!(
        workflow
            .shell_snapshot()
            .settings_rows
            .iter()
            .any(|row| row.name == "Request tuning"
                && row.value.contains("max 4096")
                && row.value.contains("temp 0.200")
                && row.value.contains("top-p 0.900")
                && row.value.contains("timeout 120000 ms")
                && row.value.contains("retry 2"))
    );

    handle_gui_action(&mut workflow, "profile-tuning-max-off", 35);
    handle_gui_action(&mut workflow, "profile-tuning-temp-off", 36);
    handle_gui_action(&mut workflow, "profile-tuning-topp-off", 37);
    handle_gui_action(&mut workflow, "profile-tuning-timeout-off", 38);
    handle_gui_action(&mut workflow, "profile-tuning-retry-off", 39);

    let profile = workflow.profiles().profile("profile-1").unwrap();
    assert_eq!(
        profile.request_tuning,
        fastrock_ui::UiRequestTuning::default()
    );
}

#[test]
fn t24_workflow_repeated_conversation_actions_allocate_unique_ids() {
    let mut workflow = FastrockWorkflow::default();

    let first = workflow.create_unique_conversation("Local folder", "~/dev/fastrock");
    let second = workflow.create_unique_conversation("Local folder", "~/dev/fastrock");
    let third = workflow.create_unique_conversation("Goal loop", "~/dev/fastrock");

    assert!(is_ulid(&first.0));
    assert!(is_ulid(&second.0));
    assert!(is_ulid(&third.0));
    assert_ne!(first, second);
    assert_ne!(second, third);
    assert_eq!(workflow.scheduler().conversation_count(), 3);
}

#[test]
fn t17_gui_conversation_new_creates_unbound_chat_without_seed_folder() {
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(BedrockProfileRequest {
            id: "mantle".to_owned(),
            name: "Mantle".to_owned(),
            provider: UiLlmProvider::BedrockMantle,
            model_id: "anthropic.claude-3-5-sonnet-20241022-v2:0".to_owned(),
            region: "us-east-1".to_owned(),
            aws_profile: "dev".to_owned(),
            default_for_new_conversations: true,
        })
        .unwrap();

    let result = handle_gui_action(&mut workflow, "conversation-new", 42);

    assert_eq!(result.status, "New conversation created");
    assert_eq!(workflow.scheduler().conversation_count(), 1);
    assert_eq!(workflow.project_folders().len(), 0);
    assert_eq!(
        result
            .writes
            .iter()
            .filter(|write| matches!(write, GuiPersistenceWrite::UpsertProjectFolder(_)))
            .count(),
        0
    );
    let snapshot = workflow.shell_snapshot();
    assert_eq!(snapshot.conversation_rows.len(), 1);
    assert_eq!(snapshot.conversation_rows[0].title, "New chat");
    assert_eq!(snapshot.conversation_rows[0].status, "Idle");
    assert_eq!(snapshot.conversation_rows[0].folder, "Home");
    assert!(snapshot.project_folders.is_empty());
    assert_eq!(snapshot.sidebar_tree_rows.len(), 2);
    assert_eq!(snapshot.sidebar_tree_rows[0].kind, "unbound");
    assert_eq!(snapshot.sidebar_tree_rows[1].kind, "conversation");
    assert_eq!(snapshot.sidebar_tree_rows[1].title, "New chat");
    assert!(
        snapshot
            .project_folders
            .iter()
            .all(|folder| !folder.target.starts_with("ssh:") && !folder.target.starts_with("ssm:"))
    );
}

#[test]
fn t30_gui_conversation_new_under_selected_project_folder() {
    let mut workflow = FastrockWorkflow::default();
    workflow.upsert_project_folder(ProjectFolderRecord {
        id: "folder-local".to_owned(),
        label: "Fastrock".to_owned(),
        target_kind: "local".to_owned(),
        path: "/Users/sean/dev/fastrock".to_owned(),
        remote_target_id: None,
        document_json: "{}".to_owned(),
        updated_at_ms: 1,
    });

    let selected = handle_gui_action(
        &mut workflow,
        "sidebar-toggle-folder:666f6c6465722d6c6f63616c",
        41,
    );
    assert!(selected.status.contains("Project folder toggled"));
    let selected = handle_gui_action(
        &mut workflow,
        "sidebar-toggle-folder:666f6c6465722d6c6f63616c",
        41,
    );
    assert!(selected.status.contains("Project folder toggled"));

    let result = handle_gui_action(&mut workflow, "conversation-new", 42);
    assert_eq!(result.status, "New conversation created");
    assert!(result.writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::UpsertConversation(metadata)
            if metadata.project_folder_id.as_deref() == Some("folder-local"))
    ));
    let snapshot = workflow.shell_snapshot();
    assert_eq!(snapshot.sidebar_tree_rows[0].kind, "folder");
    assert_eq!(snapshot.sidebar_tree_rows[0].title, "Fastrock");
    assert_eq!(snapshot.sidebar_tree_rows[1].kind, "conversation");
    assert_eq!(
        snapshot.sidebar_tree_rows[1].project_folder_id,
        "folder-local"
    );
}

#[tokio::test]
async fn t29_hydration_purges_empty_legacy_demo_rows() {
    let temp_dir = tempdir().unwrap();
    let database_path = temp_dir.path().join("fastrock.sqlite");
    let persistence = PersistenceActorHandle::spawn(PersistenceConfig::new(database_path))
        .expect("spawn persistence");
    let client = persistence.client();

    client
        .upsert_project_folder(ProjectFolderRecord {
            id: "legacy-local-folder".to_owned(),
            label: "Local workspace".to_owned(),
            target_kind: "local".to_owned(),
            path: ".".to_owned(),
            remote_target_id: None,
            document_json: serde_json::json!({
                "recent_conversation_ids": ["legacy-local"]
            })
            .to_string(),
            updated_at_ms: 1,
        })
        .await
        .unwrap();
    client
        .upsert_conversation(ConversationMetadata {
            id: "legacy-local".to_owned(),
            title: "Local folder".to_owned(),
            project_folder_id: Some("legacy-local-folder".to_owned()),
            status: "Running".to_owned(),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .await
        .unwrap();
    client
        .append_conversation_event(ConversationEvent {
            conversation_id: "legacy-local".to_owned(),
            event_type: "conversation.created".to_owned(),
            payload_json: "{}".to_owned(),
            created_at_ms: 1,
        })
        .await
        .unwrap();

    let loaded = load_persisted_workflow_from_client(&client).await.unwrap();

    assert!(loaded.shell_snapshot().conversation_rows.is_empty());
    assert!(loaded.shell_snapshot().project_folders.is_empty());
    assert!(client.list_conversations().await.unwrap().is_empty());
    assert!(client.list_project_folders().await.unwrap().is_empty());

    persistence.shutdown().await.unwrap();
}

#[test]
fn t6_workflow_discovers_aws_profiles_and_populates_safe_previews() {
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(BedrockProfileRequest {
            id: "mantle".to_owned(),
            name: "Mantle".to_owned(),
            provider: UiLlmProvider::BedrockMantle,
            model_id: "anthropic.claude-3-5-sonnet-20241022-v2:0".to_owned(),
            region: "us-east-1".to_owned(),
            aws_profile: "dev".to_owned(),
            default_for_new_conversations: true,
        })
        .unwrap();

    let shared = AwsSharedConfig::from_ini_documents(
        r#"
        [profile dev]
        region = us-west-2
        sso_session = corp

        [profile role]
        region = eu-central-1
        role_arn = arn:aws:iam::123456789012:role/Bedrock
        source_profile = dev
        "#,
        r#"
        [dev]
        aws_access_key_id = AKIAEXAMPLE
        aws_secret_access_key = super-secret-value
        "#,
    );

    let summaries = FastrockWorkflow::discover_aws_cli_profiles(&shared);
    assert_eq!(
        summaries
            .iter()
            .map(|summary| summary.name.as_str())
            .collect::<Vec<_>>(),
        vec!["dev", "role"]
    );
    assert_eq!(summaries[0].source_type, "sso");
    assert!(summaries[0].can_resolve_credentials);

    let updates = workflow.refresh_aws_credential_previews(&shared);
    assert_eq!(updates.len(), 1);
    assert!(updates[0].profile_found);
    assert_eq!(updates[0].preview.source_type, "sso");
    assert_eq!(updates[0].preview.profile_name.as_deref(), Some("dev"));

    let profile = workflow.profiles().profile("mantle").unwrap();
    let preview_json = serde_json::to_string(&profile.credential_preview).unwrap();
    assert!(!preview_json.contains("super-secret-value"));
    assert!(!preview_json.contains("AKIAEXAMPLE"));

    let snapshot = workflow.shell_snapshot();
    let settings = snapshot
        .settings_rows
        .iter()
        .map(|row| (row.name.as_str(), row.value.as_str()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(settings["AWS credential"], "sso");
    assert_eq!(
        settings["AWS region source"],
        "fastrock_profile:us-east-1;aws_profile:us-west-2"
    );
}

#[test]
fn v1_shell_is_presentation_only() {
    for forbidden in ["rtk ", "std::process", "tokio"] {
        assert!(
            !APP_SLINT.contains(forbidden),
            "Slint shell must not perform runtime work: {forbidden}"
        );
    }
}

#[test]
fn t32_controls_are_typed_scrollable_and_prerequisite_aware() {
    // V20: distinct, self-describing control types exist.
    for required in [
        "component ToggleRow inherits Rectangle",
        "component SegmentButton inherits Rectangle",
        "component SegmentTrack inherits Rectangle",
        "component ScrollArea inherits Rectangle",
        "component NoticeBanner inherits Rectangle",
        // Segmented controls highlight the active selection from real state.
        "in property <bool> active: false;",
        "active: root.prompt-compression-level == \"Full\"",
        "active: root.send-shortcut == \"control-enter\"",
        // Boolean settings render as labeled switches bound to real state.
        "checked: root.rtk-enabled",
        "checked: root.prompt-compression-enabled",
        // Scroll regions expose viewport geometry to draw a scrollbar thumb.
        "viewport-y",
        "viewport-height",
        // Disabled / read-only controls are visually and behaviorally distinct.
        "in property <bool> enabled: true;",
        "MouseCursor.not-allowed",
        // Prerequisite gating: no model profile -> gated actions + CTA + summary.
        "has-profile",
        "can-send",
        "enabled: root.has-profile",
        "Current configuration",
    ] {
        assert!(
            APP_SLINT.contains(required),
            "V20 control-clarity shell must include {required}"
        );
    }

    // Profile-dependent and send controls must be gated on a configured profile.
    assert!(
        APP_SLINT.contains(
            "property <bool> can-send: root.compose-message != \"\" && root.has-profile;"
        ),
        "send must require a non-empty message and a configured profile"
    );
    assert!(
        APP_SLINT.contains("enabled: root.can-send;"),
        "send touch area must be disabled until prerequisites are met"
    );
}

#[test]
fn t33_tooltips_are_delayed_overlay_and_focus_safe() {
    // V21: one delayed, window-level overlay tooltip — never a per-control
    // PopupWindow that grabs focus/pointer and flickers on movement.
    for required in [
        "global Tip",
        "component HoverTip inherits Rectangle",
        "Timer {",
        "interval: 550ms;",
        "running: root.hovered && root.label",
        "Tip.visible = true;",
        "changed hovered =>",
        "absolute-position",
        "tooltip-overlay := Rectangle",
        "visible: Tip.visible && Tip.text",
    ] {
        assert!(
            APP_SLINT.contains(required),
            "V21 tooltip model must include {required}"
        );
    }
    // The popup approach (focus/pointer grab) and per-move re-show (flicker)
    // must not return.
    for forbidden in [
        "PopupWindow",
        "tooltip-popup",
        "moved =>",
        ".show();",
        ".close();",
    ] {
        assert!(
            !APP_SLINT.contains(forbidden),
            "V21 tooltips must not regress to focus-stealing popups: {forbidden}"
        );
    }
}

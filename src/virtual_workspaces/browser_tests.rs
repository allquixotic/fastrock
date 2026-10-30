use super::browser::*;
use fastrock_workspaces::vwork::Installation;
use fastrock_workspaces::{Document, FileEntry, FilePage, PAGE_SIZE, Reply, Request, Workspace};

fn installation() -> Installation {
    Installation {
        id: "fixture".into(),
        label: "vwork fixture".into(),
        home: std::env::temp_dir().join("unused-fixture"),
    }
}

fn catalog(count: usize) -> Event {
    let provider = installation();
    Event::Discovery {
        providers: vec![provider.clone()],
        errors: vec![],
        catalog: (0..count)
            .map(|i| CatalogItem {
                provider: provider.clone(),
                repo: "mono".into(),
                workspace: format!("task-{i:06}"),
            })
            .collect(),
    }
}

fn epoch(work: &[Work]) -> u64 {
    let [Work::Call { epoch, .. }] = work else {
        panic!("expected one provider call: {work:?}")
    };
    *epoch
}

fn opened() -> Browser {
    let mut browser = Browser::default();
    browser.apply(catalog(2));
    let select = browser.snapshot().catalog[0].action.clone();
    let work = browser.action(&select);
    browser.apply(Event::Complete {
        epoch: epoch(&work),
        result: Ok(Reply::Files(FilePage {
            revision: "r1".into(),
            entries: vec![FileEntry {
                path: "src/main.rs".into(),
                size: 12,
                mode: "100755".into(),
            }],
            total: 1,
            next: None,
        })),
    });
    let open = browser.snapshot().files[0].action.clone();
    let work = browser.action(&open);
    browser.apply(Event::Complete {
        epoch: epoch(&work),
        result: Ok(Reply::Document(Document {
            path: "src/main.rs".into(),
            text: "old\r\n".into(),
            revision: "r1".into(),
            mode: "100755".into(),
        })),
    });
    browser
}

#[test]
fn v30_compaction_preserves_drafts() {
    let mut browser = opened();
    browser.action("virtual-edit:keep this draft\r\n");
    let work = browser.action("virtual-compact");
    assert_eq!(
        work.len(),
        1,
        "compaction must enqueue a provider operation"
    );
    assert!(browser.snapshot().busy);
    browser.apply(Event::Complete {
        epoch: epoch(&work),
        result: Ok(Reply::Report("Reclaimed 1.2 GB".into())),
    });
    assert_eq!(browser.snapshot().text, "keep this draft\r\n");
    assert!(browser.snapshot().dirty);
    assert_eq!(browser.snapshot().report, "Reclaimed 1.2 GB");
    let work = browser.action("virtual-compact");
    browser.apply(Event::Complete {
        epoch: epoch(&work),
        result: Err("Database is busy".into()),
    });
    assert!(browser.snapshot().status.contains("Database is busy"));
    assert!(browser.snapshot().dirty);
}

#[test]
fn v36_compaction_selects_database_without_workspace() {
    let mut browser = Browser::default();
    browser.apply(catalog(0));
    assert!(browser.snapshot().can_compact);
    let work = browser.action("virtual-compact");
    assert!(matches!(
        &work[..],
        [Work::Call {
            request: Request::Compact,
            ..
        }]
    ));
    browser.apply(Event::Complete {
        epoch: epoch(&work),
        result: Ok(Reply::Report("done".into())),
    });
    let mut second = installation();
    second.id = "second".into();
    second.home = second.home.join("second");
    browser.apply(Event::Discovery {
        providers: vec![installation(), second.clone()],
        catalog: vec![],
        errors: vec![],
    });
    let select = browser.snapshot().databases[1].action.clone();
    browser.action(&select);
    let work = browser.action("virtual-compact");
    let [
        Work::Call {
            provider,
            request: Request::Compact,
            ..
        },
    ] = &work[..]
    else {
        panic!()
    };
    assert_eq!(provider, &second);
    browser.apply(Event::Discovery {
        providers: vec![],
        catalog: vec![],
        errors: vec![],
    });
    browser.apply(Event::Complete {
        epoch: epoch(&work),
        result: Ok(Reply::Report("stale".into())),
    });
    assert_ne!(browser.snapshot().report, "stale");
    assert!(!browser.snapshot().can_compact);
    assert!(browser.action(&select).is_empty());
    assert!(browser.action("virtual-compact").is_empty());
}

#[test]
fn v22_controls_follow_configuration_without_losing_drafts() {
    assert!(!Browser::default().snapshot().visible);
    let mut browser = opened();
    assert!(browser.snapshot().visible);
    browser.action("virtual-edit:my draft\r\n");
    browser.apply(Event::Discovery {
        providers: vec![],
        catalog: vec![],
        errors: vec![],
    });
    assert!(!browser.snapshot().visible);
    assert!(!browser.snapshot().editable);
    assert!(browser.action("virtual-save").is_empty());
    browser.apply(catalog(2));
    assert!(browser.snapshot().visible);
    assert_eq!(browser.snapshot().text, "my draft\r\n");
    assert!(browser.snapshot().dirty);
}

#[test]
fn v23_save_uses_provider_only_with_loaded_revision_and_mode() {
    let mut browser = opened();
    browser.action("virtual-edit:b\r\n");
    let work = browser.action("virtual-save");
    let [
        Work::Call {
            request: Request::Write { document, .. },
            ..
        },
    ] = &work[..]
    else {
        panic!()
    };
    assert_eq!(document.text, "b\r\n");
    assert_eq!(document.revision, "r1");
    assert_eq!(document.mode, "100755");
    assert!(browser.snapshot().dirty);
}

#[test]
fn v24_conflicts_keep_text_and_navigation_requires_save_or_discard() {
    let mut browser = opened();
    browser.action("virtual-edit:new\r\n");
    let other = browser.snapshot().catalog[1].action.clone();
    assert!(browser.action(&other).is_empty());
    assert!(browser.snapshot().status.contains("Save or Discard"));
    let work = browser.action("virtual-save");
    assert!(!browser.snapshot().editable);
    assert!(browser.action("virtual-edit:lost update").is_empty());
    browser.apply(Event::Complete {
        epoch: epoch(&work),
        result: Err("revision conflict".into()),
    });
    assert_eq!(browser.snapshot().text, "new\r\n");
    assert!(browser.snapshot().dirty);
    let work = browser.action("virtual-save");
    browser.apply(Event::Complete {
        epoch: epoch(&work),
        result: Ok(Reply::Saved {
            revision: "r2".into(),
        }),
    });
    assert!(!browser.snapshot().dirty);
    assert!(browser.snapshot().title.contains("r2"));
    browser.action("virtual-edit:discard this");
    browser.action("virtual-discard");
    assert_eq!(browser.snapshot().text, "new\r\n");
}

#[test]
fn v24_stale_responses_and_stale_row_clicks_cannot_change_selection() {
    let mut browser = opened();
    let stale_row = browser.snapshot().catalog[1].action.clone();
    let work = browser.action("virtual-reload");
    browser.apply(Event::Discovery {
        providers: vec![],
        catalog: vec![],
        errors: vec![],
    });
    browser.apply(Event::Complete {
        epoch: epoch(&work),
        result: Ok(Reply::Document(Document {
            path: "wrong".into(),
            text: "stale".into(),
            revision: "r9".into(),
            mode: "100644".into(),
        })),
    });
    assert_eq!(browser.snapshot().text, "old\r\n");
    browser.apply(catalog(3));
    assert!(browser.action(&stale_row).is_empty());
    assert_eq!(browser.snapshot().text, "old\r\n");
}

#[test]
fn v24_large_catalog_and_file_pages_keep_models_bounded() {
    let mut browser = Browser::default();
    browser.apply(catalog(100_000));
    assert_eq!(browser.snapshot().catalog.len(), PAGE_SIZE);
    browser.action("virtual-catalog-filter:TASK-099999");
    let snapshot = browser.snapshot();
    assert_eq!(snapshot.catalog.len(), 1);
    assert!(snapshot.catalog[0].label.contains("task-099999"));
    let work = browser.action(&snapshot.catalog[0].action);
    browser.apply(Event::Complete {
        epoch: epoch(&work),
        result: Ok(Reply::Files(FilePage {
            revision: "r1".into(),
            entries: (0..PAGE_SIZE)
                .map(|i| FileEntry {
                    path: format!("src/{i}.rs"),
                    size: 0,
                    mode: "100644".into(),
                })
                .collect(),
            total: 1_000_000,
            next: Some(PAGE_SIZE),
        })),
    });
    assert_eq!(browser.snapshot().files.len(), PAGE_SIZE);
    let work = browser.action("virtual-next");
    let [
        Work::Call {
            request: Request::Files { offset, .. },
            ..
        },
    ] = &work[..]
    else {
        panic!()
    };
    assert_eq!(*offset, PAGE_SIZE);
}

#[test]
fn v24_create_and_fork_switch_only_after_provider_success() {
    let mut browser = opened();
    let work = browser.action("virtual-fork:branch-two");
    let followup = browser.apply(Event::Complete {
        epoch: epoch(&work),
        result: Ok(Reply::Created(Workspace {
            id: "branch-two".into(),
            repo: "mono".into(),
            revision: "r2".into(),
        })),
    });
    let [
        Work::Call {
            request: Request::Files {
                workspace, offset, ..
            },
            ..
        },
    ] = &followup[..]
    else {
        panic!()
    };
    assert_eq!(workspace, "branch-two");
    assert_eq!(*offset, 0);
    assert!(browser.snapshot().busy);
}

#[test]
fn v24_rejected_paste_preserves_the_previous_editor_buffer() {
    let mut browser = opened();
    browser.action("virtual-edit:keep this");
    browser.action("virtual-edit:binary\0text");
    assert_eq!(browser.text(), "keep this");
    browser.action(&format!(
        "virtual-edit:{}",
        "x".repeat(fastrock_workspaces::MAX_TEXT_BYTES + 1)
    ));
    assert_eq!(browser.text(), "keep this");
    assert!(browser.snapshot().dirty);
}

#[test]
fn v40_virtual_gui_keeps_storage_off_ui_thread_and_separate_from_local_files() {
    let main_ui = include_str!("../../ui/app.slint");
    assert!(main_ui.contains("if AppState.virtual-workspaces-available: LinkButton"));
    let ui = include_str!("../../ui/virtual-workspaces.slint");
    assert!(ui.contains("read-only: !root.workspace-editable"));
    let mut browser = opened();
    let work = browser.action("virtual-compact");
    assert!(matches!(
        &work[..],
        [Work::Call {
            request: Request::Compact,
            ..
        }]
    ));
}

#[test]
fn v24_renaming_extension_keeps_dirty_workspace_editable() {
    let mut browser = opened();
    browser.action("virtual-edit:keep this draft");
    let mut renamed = installation();
    renamed.label = "renamed Goose extension".into();
    browser.apply(Event::Discovery {
        providers: vec![renamed.clone()],
        catalog: vec![CatalogItem {
            provider: renamed,
            repo: "mono".into(),
            workspace: "task-000000".into(),
        }],
        errors: vec![],
    });
    assert!(browser.snapshot().dirty);
    assert!(browser.snapshot().editable);
    assert_eq!(browser.snapshot().text, "keep this draft");
    assert!(!browser.action("virtual-save").is_empty());
}

fn reviewed(browser: &mut Browser, complete: bool) {
    browser.action("virtual-merge-source:child");
    let work = browser.action("virtual-merge-review");
    assert!(matches!(
        &work[..],
        [Work::Call {
            request: Request::ReviewMerge { .. },
            ..
        }]
    ));
    browser.apply(Event::Complete {
        epoch: epoch(&work),
        result: Ok(Reply::MergeReview(
            fastrock_workspaces::integration::MergeReview {
                target: "task-000000".into(),
                source: "child".into(),
                target_revision: "r1".into(),
                source_revision: "r2".into(),
                report: "reviewed changes".into(),
                complete,
            },
        )),
    });
}

#[test]
fn v49_merge_requires_current_complete_review_and_consumes_it() {
    let mut browser = opened();
    assert!(browser.action("virtual-merge-apply").is_empty());
    reviewed(&mut browser, false);
    assert!(!browser.snapshot().can_merge);
    assert!(browser.action("virtual-merge-apply").is_empty());
    reviewed(&mut browser, true);
    assert!(browser.snapshot().can_merge);
    let work = browser.action("virtual-merge-apply");
    assert!(matches!(
        &work[..],
        [Work::Call {
            request: Request::Merge(_),
            ..
        }]
    ));
    assert!(!browser.snapshot().can_merge);
    browser.apply(Event::Complete {
        epoch: epoch(&work),
        result: Err("changed after review".into()),
    });
    assert!(browser.action("virtual-merge-apply").is_empty());
    assert_eq!(browser.snapshot().text, "old\r\n");
}

#[test]
fn v49_edit_source_change_and_report_change_invalidate_review() {
    for action in [
        "virtual-edit:unsaved",
        "virtual-merge-source:other",
        "virtual-compact",
        "virtual-diff",
    ] {
        let mut browser = opened();
        reviewed(&mut browser, true);
        browser.action(action);
        assert!(!browser.snapshot().can_merge, "{action}");
    }
    let mut browser = opened();
    reviewed(&mut browser, true);
    browser.action("virtual-edit:unsaved");
    assert!(browser.action("virtual-merge-review").is_empty());
    assert!(browser.action("virtual-merge-apply").is_empty());
    assert_eq!(browser.snapshot().text, "unsaved");
}

#[test]
fn v50_workspace_chat_requires_clean_draft_and_current_completion() {
    let mut browser = opened();
    browser.action("virtual-edit:unsaved");
    assert!(browser.action("virtual-chat").is_empty());
    browser.action("virtual-discard");
    let work = browser.action("virtual-chat");
    assert!(
        matches!(&work[0], Work::Call {request:Request::OpenConversation {workspace}, ..} if workspace == "task-000000")
    );
    let current = epoch(&work);
    assert!(browser.accepts_completion(current));
    assert!(!browser.accepts_completion(current + 1));
    browser.apply(Event::Complete {
        epoch: current,
        result: Err("fixture".into()),
    });
    assert!(!browser.accepts_completion(current));
}

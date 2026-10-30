//! Read-only review targets over the owning compressed store or a local Git repository.
use crate::{embedded::Connection, session::Selection};
use anyhow::{Context, Result, ensure};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
};
use vwork_engine::{engine::Engine, git, model};

const MAX_FILE: usize = 1024 * 1024;
const MAX_INPUT: usize = 8 * MAX_FILE;
const MAX_OUTPUT: usize = 256 * 1024;
const MAX_FILES: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Commit {
    pub sha: String,
    pub timestamp: i64,
    pub subject: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Catalog {
    pub branches: Vec<String>,
    pub current: Option<String>,
    pub commits: Vec<Commit>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Target {
    UncommittedChanges,
    BaseBranch { branch: String },
    Commit { sha: String },
    Custom { instructions: String },
}

fn catalog(
    repo: &git2::Repository,
    mut branches: Vec<String>,
    current: Option<String>,
    head: git2::Oid,
) -> Result<Catalog> {
    ensure!(
        branches.len() <= 10_000,
        "Repository has too many branches for the review picker"
    );
    branches.sort();
    branches.dedup();
    if let Some(index) = ["main", "master"]
        .iter()
        .find_map(|name| branches.iter().position(|b| b == name))
    {
        let first = branches.remove(index);
        branches.insert(0, first);
    }
    let mut walk = repo.revwalk()?;
    walk.set_sorting(git2::Sort::TIME)?;
    walk.push(head)?;
    let commits = walk
        .take(100)
        .map(|id| {
            let commit = repo.find_commit(id?)?;
            Ok(Commit {
                sha: commit.id().to_string(),
                timestamp: commit.time().seconds(),
                subject: commit
                    .summary()?
                    .unwrap_or("Untitled commit")
                    .chars()
                    .take(512)
                    .collect(),
            })
        })
        .collect::<Result<_>>()?;
    Ok(Catalog {
        branches,
        current,
        commits,
    })
}

pub async fn list(selection: Option<Selection>, cwd: PathBuf) -> Result<Catalog> {
    if let Some(selection) = selection {
        return Connection::new(selection.home)
            .run(move |store| {
                store.with_engine(|engine| {
                    let ws = engine.workspace(&selection.workspace)?;
                    let revision = engine.revision(&ws.head)?;
                    let registered = engine.repo(&ws.repo)?;
                    let repo = git::open(&engine.db, &registered, engine.config.compression_level)?;
                    let branches = registered
                        .refs
                        .keys()
                        .filter_map(|name| name.strip_prefix("refs/heads/").map(str::to_owned))
                        .collect();
                    // Virtual workspaces have a base commit, not a checked-out branch.
                    catalog(&repo, branches, None, git2::Oid::from_str(&revision.base)?)
                })
            })
            .await;
    }
    tokio::task::spawn_blocking(move || {
        let repo = git2::Repository::discover(cwd)?;
        let branches = repo
            .branches(Some(git2::BranchType::Local))?
            .take(10_001)
            .map(|branch| {
                Ok(branch?
                    .0
                    .name()?
                    .context("Branch name is not Unicode")?
                    .to_owned())
            })
            .collect::<Result<Vec<_>>>()?;
        let head = repo.head()?;
        let current = head
            .is_branch()
            .then(|| head.shorthand().map(str::to_owned))
            .transpose()?;
        catalog(&repo, branches, current, head.peel_to_commit()?.id())
    })
    .await?
}

fn append(output: &mut String, text: &str) -> Result<()> {
    ensure!(
        output.len().saturating_add(text.len()) <= MAX_OUTPUT,
        "Review exceeds 256 KiB of patch text; use custom instructions to narrow the review"
    );
    output.push_str(text);
    Ok(())
}
fn patch(
    output: &mut String,
    path: &str,
    old: &[u8],
    new: &[u8],
    old_mode: &str,
    new_mode: &str,
) -> Result<()> {
    append(
        output,
        &format!(
            "\nFile {} · mode {old_mode} → {new_mode}\n",
            serde_json::to_string(path)?
        ),
    )?;
    if old_mode == "160000" || new_mode == "160000" {
        return append(
            output,
            &format!(
                "Submodule commit: {} → {}\n",
                String::from_utf8_lossy(old),
                String::from_utf8_lossy(new)
            ),
        );
    }
    if old.contains(&0)
        || new.contains(&0)
        || std::str::from_utf8(old).is_err()
        || std::str::from_utf8(new).is_err()
    {
        return append(
            output,
            &format!(
                "Binary contents changed ({} → {} bytes); content is not included.\n",
                old.len(),
                new.len()
            ),
        );
    }
    let mut options = git2::DiffOptions::new();
    options.context_lines(3);
    let mut patch = git2::Patch::from_buffers(
        old,
        Some(Path::new(path)),
        new,
        Some(Path::new(path)),
        Some(&mut options),
    )?;
    // Avoid allocating an unbounded formatted patch after the bounded inputs.
    let mut exceeded = false;
    patch
        .print(&mut |_, _, line| {
            let prefix = match line.origin() {
                '+' => "+",
                '-' => "-",
                ' ' => " ",
                _ => "",
            };
            let text = String::from_utf8_lossy(line.content());
            if output.len() + prefix.len() + text.len() > MAX_OUTPUT {
                exceeded = true;
                return false;
            }
            output.push_str(prefix);
            output.push_str(&text);
            true
        })
        .or_else(|e| if exceeded { Ok(()) } else { Err(e) })?;
    ensure!(
        !exceeded,
        "Review exceeds 256 KiB of patch text; use custom instructions to narrow the review"
    );
    Ok(())
}
fn account(total: &mut usize, size: u64) -> Result<()> {
    ensure!(
        size <= MAX_FILE as u64,
        "Review includes a file over 1 MiB; use custom instructions to narrow the review"
    );
    *total = total.saturating_add(size as usize);
    ensure!(
        *total <= MAX_INPUT,
        "Review inputs exceed 8 MiB; use custom instructions to narrow the review"
    );
    Ok(())
}
fn blob(repo: &git2::Repository, oid: git2::Oid, mode: &str, total: &mut usize) -> Result<Vec<u8>> {
    if mode == "160000" {
        return Ok(oid.to_string().into_bytes());
    }
    let (size, kind) = repo.odb()?.read_header(oid)?;
    ensure!(kind == git2::ObjectType::Blob, "Expected file blob");
    account(total, size as u64)?;
    Ok(repo.find_blob(oid)?.content().to_vec())
}
fn commit_pair(
    repo: &git2::Repository,
    target: &Target,
    head: git2::Oid,
    resolve: impl Fn(&str) -> Result<git2::Oid>,
) -> Result<(Option<git2::Oid>, git2::Oid)> {
    match target {
        Target::UncommittedChanges => Ok((Some(head), head)),
        Target::BaseBranch { branch } => {
            ensure!(
                !branch.is_empty() && branch.len() <= 4096,
                "Invalid review branch"
            );
            Ok((Some(repo.merge_base(head, resolve(branch)?)?), head))
        }
        Target::Commit { sha } => {
            ensure!(
                sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
                "Review requires a full commit ID"
            );
            let commit = repo.find_commit(resolve(sha)?)?;
            Ok((
                if commit.parent_count() == 0 {
                    None
                } else {
                    Some(commit.parent_id(0)?)
                },
                commit.id(),
            ))
        }
        Target::Custom { .. } => unreachable!(),
    }
}

fn virtual_context(engine: &Engine, workspace: &str, target: &Target) -> Result<String> {
    let ws = engine.workspace(workspace)?;
    let (revision, current) = engine.snapshot(&ws)?;
    let registered = engine.repo(&ws.repo)?;
    let repo = git::open(&engine.db, &registered, engine.config.compression_level)?;
    let (base, selected) = commit_pair(
        &repo,
        target,
        git2::Oid::from_str(&revision.base)?,
        |reference| {
            Ok(git2::Oid::from_str(&git::resolve(
                &engine.db,
                &registered,
                reference,
            )?)?)
        },
    )?;
    let before = match base {
        Some(id) => git::tree(&engine.db, &registered, &id.to_string())?,
        None => model::Tree::new(),
    };
    let after = if matches!(target, Target::Commit { .. }) {
        git::tree(&engine.db, &registered, &selected.to_string())?
    } else {
        current
    };
    ensure!(
        before.len().saturating_add(after.len()) <= 500_000,
        "Review repository exceeds the path limit"
    );
    let paths: BTreeSet<_> = before.keys().chain(after.keys()).collect();
    let mut output = format!(
        "Compressed repository {}; workspace {}; revision {}.\nComparison base: {}; selected commit: {}.\n",
        ws.repo,
        ws.id,
        ws.head,
        base.map_or_else(|| "empty tree".into(), |id| id.to_string()),
        selected
    );
    let mut total = 0;
    let mut changed = 0;
    for path in paths {
        let old = before.get(path);
        let new = after.get(path);
        if old == new {
            continue;
        }
        changed += 1;
        ensure!(
            changed <= MAX_FILES,
            "Review has more than 100 changed files; use custom instructions to narrow it"
        );
        let mut read = |entry: Option<&model::Entry>| -> Result<Vec<u8>> {
            let Some(entry) = entry else {
                return Ok(Vec::new());
            };
            if entry.source == "git" {
                return blob(
                    &repo,
                    git2::Oid::from_str(&entry.oid)?,
                    &entry.mode,
                    &mut total,
                );
            }
            account(&mut total, entry.size)?;
            let value = engine.call(
                "files_read",
                &serde_json::json!({"workspace":ws.id,"path":path,"binary":true}),
            )?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(value["base64"].as_str().context("Missing file bytes")?)?;
            ensure!(
                bytes.len() <= MAX_FILE && bytes.len() as u64 == entry.size,
                "File size changed while preparing review"
            );
            Ok(bytes)
        };
        let old_bytes = read(old)?;
        let new_bytes = read(new)?;
        patch(
            &mut output,
            path,
            &old_bytes,
            &new_bytes,
            old.map_or("absent", |e| &e.mode),
            new.map_or("absent", |e| &e.mode),
        )?;
    }
    if changed == 0 {
        append(&mut output, "No changes in this comparison.\n")?;
    }
    Ok(output)
}

fn local_context(cwd: &Path, target: &Target) -> Result<String> {
    let repo = git2::Repository::discover(cwd)?;
    let head = repo.head()?.peel_to_commit()?.id();
    let (base, selected) = commit_pair(&repo, target, head, |reference| {
        Ok(repo.revparse_single(reference)?.peel_to_commit()?.id())
    })?;
    let before = base.map(|id| repo.find_commit(id)?.tree()).transpose()?;
    let after = repo.find_commit(selected)?.tree()?;
    let mut options = git2::DiffOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true)
        .max_size(MAX_FILE as i64);
    let diff = if matches!(target, Target::Commit { .. }) {
        repo.diff_tree_to_tree(before.as_ref(), Some(&after), Some(&mut options))?
    } else {
        repo.diff_tree_to_workdir_with_index(before.as_ref(), Some(&mut options))?
    };
    ensure!(
        diff.deltas().len() <= MAX_FILES,
        "Review has more than 100 changed files; use custom instructions to narrow it"
    );
    let mut output = format!(
        "Local repository {}.\nComparison base: {}; selected commit: {}.\n",
        repo.workdir().unwrap_or(cwd).display(),
        base.map_or_else(|| "empty tree".into(), |id| id.to_string()),
        selected
    );
    let mut total = 0;
    for delta in diff.deltas() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .context("Missing changed path")?;
        let path_text = path.to_str().context("Changed path is not Unicode")?;
        let old_mode = format!("{:o}", i32::from(delta.old_file().mode()));
        let new_mode = format!("{:o}", i32::from(delta.new_file().mode()));
        let old = if delta.old_file().id().is_zero() {
            Vec::new()
        } else {
            blob(&repo, delta.old_file().id(), &old_mode, &mut total)?
        };
        let new = if delta.status() == git2::Delta::Deleted {
            Vec::new()
        } else if matches!(target, Target::Commit { .. }) || new_mode == "160000" {
            blob(&repo, delta.new_file().id(), &new_mode, &mut total)?
        } else {
            let root = repo
                .workdir()
                .context("Repository has no working directory")?;
            let file = root.join(path);
            let metadata = file.symlink_metadata()?;
            account(&mut total, metadata.len())?;
            if metadata.file_type().is_symlink() {
                std::fs::read_link(file)?
                    .to_string_lossy()
                    .as_bytes()
                    .to_vec()
            } else {
                ensure!(metadata.is_file(), "Changed path is not a regular file");
                // Do not follow a parent symlink out of this repository.
                ensure!(
                    file.canonicalize()?.starts_with(root.canonicalize()?),
                    "Changed path escapes the repository"
                );
                let mut bytes = Vec::new();
                std::fs::File::open(file)?
                    .take(MAX_FILE as u64 + 1)
                    .read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() <= MAX_FILE && bytes.len() as u64 == metadata.len(),
                    "File changed while preparing review"
                );
                bytes
            }
        };
        patch(&mut output, path_text, &old, &new, &old_mode, &new_mode)?;
    }
    if diff.deltas().len() == 0 {
        append(&mut output, "No changes in this comparison.\n")?;
    }
    Ok(output)
}

pub async fn context(selection: Option<Selection>, cwd: PathBuf, target: Target) -> Result<String> {
    if let Target::Custom { instructions } = target {
        ensure!(
            !instructions.trim().is_empty() && instructions.len() <= 64 * 1024,
            "Custom review instructions must be nonempty and at most 64 KiB"
        );
        return Ok(instructions);
    }
    if let Some(selection) = selection {
        return Connection::new(selection.home)
            .run(move |store| {
                store.with_exclusive_engine(|engine| {
                    virtual_context(engine, &selection.workspace, &target)
                })
            })
            .await;
    }
    tokio::task::spawn_blocking(move || local_context(&cwd, &target)).await?
}

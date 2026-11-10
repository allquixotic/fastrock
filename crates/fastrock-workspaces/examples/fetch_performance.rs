//! Synthetic fetch comparison. Compile locally; run only on the Windows test host.
use anyhow::{Context, Result, ensure};
use git2::{Oid, Repository, Signature, Time};
use serde_json::{Value, json};
use std::{path::Path, time::Instant};
use vwork_engine::{engine::Engine, model::Config};

fn timed(engine: &Engine, label: &str, op: &str, args: Value) -> Result<Value> {
    let start = Instant::now();
    let result = engine.call(op, &args)?;
    println!(
        "{}",
        json!({"operation":label,"elapsed_ms":start.elapsed().as_secs_f64()*1000.,"result":if op == "repo_register" {json!({"registered":true})} else {result.clone()}})
    );
    Ok(result)
}

fn seed(path: &Path, scale: usize) -> Result<()> {
    ensure!(!path.exists() && (1..=10).contains(&scale));
    let repo = Repository::init_bare(path)?;
    let mut tree = repo.treebuilder(None)?;
    for n in 0..400 / scale {
        let bytes = (0..1024)
            .map(|line| {
                format!("fixture file {n}, source line {line}: shared synthetic source text\n")
            })
            .collect::<String>();
        tree.insert(
            format!("file-{n:05}.txt"),
            repo.blob(bytes.as_bytes())?,
            0o100644,
        )?;
    }
    let author = Signature::new(
        "Fixture",
        "fixture@example.invalid",
        &Time::new(1_700_000_000, 0),
    )?;
    let snapshot = repo.find_tree(tree.write()?)?;
    let base = repo.commit(
        Some("refs/heads/main"),
        &author,
        &author,
        "base",
        &snapshot,
        &[],
    )?;
    repo.set_head("refs/heads/main")?;
    // Packed source refs keep fixture setup separate from receiver ref costs.
    for n in 0..2000 / scale {
        repo.reference(&format!("refs/heads/branch-{n:05}"), base, false, "fixture")?;
    }
    repo.tag(
        "historical",
        &repo.find_object(base, None)?,
        &author,
        "annotated tag",
        false,
    )?;
    repo.refdb()?.compress()?;
    std::fs::write(path.join("fixture-base"), base.to_string())?;
    println!(
        "{}",
        json!({"seeded":true,"files":400/scale,"branches":2000/scale+1})
    );
    Ok(())
}

fn run(source: &Path, home: &Path) -> Result<()> {
    ensure!(!home.exists());
    // Each comparison must start from its own identical, newly seeded source:
    // resetting a ref leaves the previous run's new objects in the ODB, and
    // registration intentionally imports unreachable objects too.
    let used = source.join("fixture-used");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&used)?;
    let repo = Repository::open_bare(source)?;
    let base = Oid::from_str(std::fs::read_to_string(source.join("fixture-base"))?.trim())?;
    repo.reference("refs/heads/main", base, true, "reset synthetic fixture")?;
    let engine = Engine::open(home, Config::default())?;
    timed(
        &engine,
        "register",
        "repo_register",
        json!({"repo":"fixture","path":source}),
    )?;
    engine.call(
        "workspace_create",
        &json!({"repo":"fixture","workspace":"old"}),
    )?;
    let old = engine.workspace("old")?.head;
    timed(
        &engine,
        "unchanged_fetch",
        "repo_fetch",
        json!({"repo":"fixture","source":source}),
    )?;
    let parent = repo.find_commit(base)?;
    let mut tree = repo.treebuilder(Some(&parent.tree()?))?;
    tree.insert("new.txt", repo.blob(b"one new upstream file\n")?, 0o100644)?;
    let author = Signature::new(
        "Fixture",
        "fixture@example.invalid",
        &Time::new(1_700_000_001, 0),
    )?;
    let updated = repo.commit(
        Some("refs/heads/main"),
        &author,
        &author,
        "update",
        &repo.find_tree(tree.write()?)?,
        &[&parent],
    )?;
    let result = timed(
        &engine,
        "updated_fetch",
        "repo_fetch",
        json!({"repo":"fixture"}),
    )?;
    ensure!(result["head"] == updated.to_string());
    ensure!(engine.workspace("old")?.head == old);
    timed(
        &engine,
        "repeat_fetch",
        "repo_fetch",
        json!({"repo":"fixture"}),
    )?;
    engine.call(
        "workspace_create",
        &json!({"repo":"fixture","workspace":"new"}),
    )?;
    let read = engine.call("files_read", &json!({"workspace":"new","path":"new.txt"}))?;
    ensure!(read.to_string().contains("one new upstream file"), "{read}");
    engine.db.persist()?;
    println!(
        "{}",
        json!({"verified":true,"old_workspace_unchanged":true})
    );
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let mode = args.get(1).context("seed SOURCE SCALE | run SOURCE HOME")?;
    let source = Path::new(args.get(2).context("source")?);
    match mode.as_str() {
        "seed" => seed(source, args.get(3).context("scale")?.parse()?),
        "run" => run(source, Path::new(args.get(3).context("home")?)),
        _ => anyhow::bail!("unknown mode"),
    }
}

//! Opt-in Windows fixture; compile on the Mac, execute only on the test host.
//! The same source can be compiled against the previous release for comparison.
use anyhow::{Context, Result, ensure};
use fastrock_workspaces::embedded::Connection;
use git2::{Oid, Repository, Signature, Time};
use serde_json::{Value, json};
use std::path::Path;
use std::time::Instant;

fn seed(path: &Path, files: usize, commits: usize) -> Result<()> {
    ensure!(!path.exists(), "Source fixture must be new");
    ensure!((100..=10000).contains(&files) && (100..=20000).contains(&commits));
    let repo = Repository::init(path)?;
    let mut tree = repo.treebuilder(None)?;
    for n in 0..files {
        let content = format!("synthetic source file {n}\n{}", "source line\n".repeat(32));
        tree.insert(
            format!("file-{n:05}.txt"),
            repo.blob(content.as_bytes())?,
            0o100644,
        )?;
    }
    let mut previous: Option<Oid> = None;
    for n in 0..commits {
        tree.insert(
            "changing.txt",
            repo.blob(format!("revision {n}\n").as_bytes())?,
            0o100644,
        )?;
        let snapshot = repo.find_tree(tree.write()?)?;
        let author = Signature::new(
            "Fixture",
            "fixture@example.invalid",
            &Time::new(1700000000 + n as i64, 0),
        )?;
        let parent = previous.map(|id| repo.find_commit(id)).transpose()?;
        let side = if n > 0 && n % 100 == 0 {
            Some(repo.find_commit(repo.commit(
                None,
                &author,
                &author,
                "side",
                &snapshot,
                &parent.iter().collect::<Vec<_>>(),
            )?)?)
        } else {
            None
        };
        let parents: Vec<_> = parent.iter().chain(side.iter()).collect();
        let id = repo.commit(
            Some("HEAD"),
            &author,
            &author,
            &format!("revision {n}"),
            &snapshot,
            &parents,
        )?;
        if n == 0 {
            repo.tag(
                "historical",
                &repo.find_object(id, None)?,
                &author,
                "retained history",
                false,
            )?;
        }
        previous = Some(id);
    }
    println!(
        "{}",
        json!({"seeded":true,"files":files+1,"commits":commits,"merge_every":100})
    );
    Ok(())
}

async fn timed(connection: Connection, name: &str, op: &str, args: Value) -> Result<Value> {
    let start = Instant::now();
    let result = connection.call(op.into(), args).await;
    println!(
        "{}",
        json!({"operation":name,"elapsed_ms":start.elapsed().as_secs_f64()*1000.,"success":result.is_ok()})
    );
    result
}

fn sizes(path: &Path) -> Result<(u64, u64)> {
    let (mut files, mut bytes) = (0, 0);
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            let (f, b) = sizes(&entry.path())?;
            files += f;
            bytes += b;
        } else {
            files += 1;
            bytes += metadata.len();
        }
    }
    Ok((files, bytes))
}

async fn run(source: &Path, home: &Path) -> Result<()> {
    ensure!(!home.exists(), "Store fixture must be new");
    let connection = Connection::new(home.into());
    timed(
        connection.clone(),
        "register",
        "repo_register",
        json!({"repo":"fixture","path":source}),
    )
    .await?;
    let workspace = timed(
        connection.clone(),
        "create_cold",
        "workspace_create",
        json!({"repo":"fixture","workspace":"temporary"}),
    )
    .await?;
    let revision = workspace["head"].as_str().context("workspace revision")?;
    let start = Instant::now();
    let (one, two, log, temporary) = tokio::join!(
        timed(
            connection.clone(),
            "concurrent_create_one",
            "workspace_create",
            json!({"repo":"fixture","workspace":"one"})
        ),
        timed(
            connection.clone(),
            "concurrent_create_two",
            "workspace_create",
            json!({"repo":"fixture","workspace":"two"})
        ),
        timed(
            connection.clone(),
            "concurrent_log_three",
            "repo_log",
            json!({"repo":"fixture","limit":3})
        ),
        timed(
            connection.clone(),
            "concurrent_temporary",
            "workspace_temporary",
            json!({"workspace":"temporary","expected_revision":revision,"purpose":"synthetic performance acceptance"})
        ),
    );
    one?;
    two?;
    ensure!(log?["log"].as_str().context("log")?.lines().count() == 3);
    let lease = temporary?;
    println!(
        "{}",
        json!({"operation":"concurrent_group","elapsed_ms":start.elapsed().as_secs_f64()*1000.})
    );
    let checkout = Path::new(lease["path"].as_str().context("lease path")?);
    let repo = Repository::open(checkout)?;
    let (object_files, object_bytes) = sizes(&repo.path().join("objects"))?;
    let (store_files, store_bytes) = sizes(&home.join("workspaces.fjall"))?;
    ensure!(repo.revparse_single("historical^{}")?.kind() == Some(git2::ObjectType::Commit));
    ensure!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(checkout)
            .args(["fsck", "--full"])
            .output()?
            .status
            .success(),
        "git fsck failed"
    );
    println!(
        "{}",
        json!({"object_files":object_files,"object_bytes":object_bytes,"store_files":store_files,"store_bytes":store_bytes,"fsck":"PASS"})
    );
    drop(repo);
    timed(
        connection.clone(),
        "warm_log_three",
        "repo_log",
        json!({"repo":"fixture","limit":3}),
    )
    .await?;
    timed(
        connection.clone(),
        "create_warm",
        "workspace_create",
        json!({"repo":"fixture","workspace":"warm"}),
    )
    .await?;
    timed(
        connection,
        "release",
        "temporary_release",
        json!({"lease":lease["id"],"discard_changes":true}),
    )
    .await?;
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("seed") if args.len() == 5 => {
            seed(Path::new(&args[2]), args[3].parse()?, args[4].parse()?)
        }
        Some("run") if args.len() == 4 => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(run(Path::new(&args[2]), Path::new(&args[3]))),
        _ => anyhow::bail!("Usage: seed NEW_SOURCE FILES COMMITS | run SOURCE NEW_STORE"),
    }
}

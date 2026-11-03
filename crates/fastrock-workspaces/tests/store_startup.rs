//! Isolated large-redb seed for native startup/I/O acceptance. Never user data.
use anyhow::{Context, Result, ensure};
use redb::{ReadableDatabase, TableDefinition};
use serde_json::json;
use std::path::Path;
use vwork_engine::{engine::Engine, model::Config};

const FILLER: TableDefinition<u64, &[u8]> = TableDefinition::new("startup_fixture_payload");

fn seed(root: &Path, mib: usize, repositories: usize) -> Result<()> {
    ensure!(
        (1..=4096).contains(&mib) && (1..=64).contains(&repositories),
        "Fixture scale out of bounds"
    );
    std::fs::create_dir(root).context("Fixture root must not exist")?;
    let home = root.join("store");
    let engine = Engine::open(&home, Config::default())?;
    drop(engine);
    // Bulk fixture generation must not fill redb's default 1 GiB cache and
    // consume the hypervisor's remaining reserve before the app is measured.
    let db = redb::Database::builder()
        .set_cache_size(32 * 1024 * 1024)
        .open(home.join("workspaces.redb"))?;
    // Valid, incompressible payload isolates database-size overhead from repo
    // enumeration. Actual registered Git repos/workspaces exercise that too.
    // Each transaction holds at most 16 MiB; never allocate the full DB in RAM.
    let mut bytes = vec![0u8; 1024 * 1024];
    let mut rng = 0x1234_5678_9abc_def0u64;
    for chunk in bytes.chunks_exact_mut(8) {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        chunk.copy_from_slice(&rng.to_le_bytes());
    }
    for batch in (0..mib).step_by(16) {
        let tx = db.begin_write()?;
        {
            let mut table = tx.open_table(FILLER)?;
            for index in batch..(batch + 16).min(mib) {
                table.insert(index as u64, bytes.as_slice())?;
            }
        }
        tx.commit()?;
    }
    drop(db);
    let engine = Engine::open(&home, Config::default())?;
    for index in 0..repositories {
        let name = format!("repo-{index:03}");
        let source = root.join(&name);
        let repo = git2::Repository::init(&source)?;
        let blob = repo.blob(&bytes[..4096])?;
        let mut tree = repo.treebuilder(None)?;
        for file in 0..100 {
            tree.insert(format!("file-{file:03}.dat"), blob, 0o100644)?;
        }
        let tree = repo.find_tree(tree.write()?)?;
        let signature = git2::Signature::now("Fixture", "fixture@example.invalid")?;
        repo.commit(Some("HEAD"), &signature, &signature, "fixture", &tree, &[])?;
        engine.call("repo_register", &json!({"repo":name, "path":source}))?;
        for workspace in 0..4 {
            engine.call(
                "workspace_create",
                &json!({"repo":name,"workspace":format!("{name}-work-{workspace}")}),
            )?;
        }
    }
    drop(engine);
    std::fs::write(
        root.join("fixture.json"),
        serde_json::to_vec_pretty(
            &json!({"payload_mib":mib,"repositories":repositories,"workspaces":repositories*4}),
        )?,
    )?;
    println!(
        "{}",
        json!({"database_bytes":std::fs::metadata(home.join("workspaces.redb"))?.len(),"payload_mib":mib,"repositories":repositories,"workspaces":repositories*4})
    );
    Ok(())
}

fn verify(root: &Path) -> Result<()> {
    let home = root.join("store");
    // A clean open must work even when a new recovery snapshot cannot be made.
    std::fs::create_dir(home.join("workspaces.redb.recovery.partial"))?;
    let engine = Engine::open(&home, Config::default())?;
    ensure!(
        engine.recovery["state"] == "clean" && engine.recovery["integrity_checked"] == false,
        "Unexpected recovery on clean startup"
    );
    ensure!(
        engine
            .db
            .begin_read()?
            .open_table(FILLER)?
            .get(0)?
            .context("Missing payload")?
            .value()
            .len()
            == 1024 * 1024,
        "Lost fixture data"
    );
    let health = engine.call("health", &json!({}))?;
    ensure!(
        health["recovery"]["state"] == "clean",
        "Incorrect health report"
    );
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("window_runtime::tests") {
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("--seed") {
        ensure!(args.len() == 5, "Usage: --seed NEW_ROOT MIB REPOSITORIES");
        return seed(Path::new(&args[2]), args[3].parse()?, args[4].parse()?);
    }
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("fixture");
    seed(&root, 1, 1)?;
    verify(&root)?;
    println!("v87_clean_store_with_repositories: PASS");
    Ok(())
}

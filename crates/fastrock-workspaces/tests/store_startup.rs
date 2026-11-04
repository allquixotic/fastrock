//! Isolated large-Fjall seed for native startup/I/O acceptance. Never user data.
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::path::Path;
use vwork_engine::kv::{Database, TableDefinition};
use vwork_engine::{engine::Engine, model::Config};

const FILLER: TableDefinition<&str, &[u8]> = TableDefinition::new("records_v1");

fn seed(root: &Path, mib: usize, repositories: usize) -> Result<()> {
    ensure!(
        (1..=4096).contains(&mib) && (1..=64).contains(&repositories),
        "Fixture scale out of bounds"
    );
    std::fs::create_dir(root).context("Fixture root must not exist")?;
    let home = root.join("store");
    let engine = Engine::open(&home, Config::default())?;
    drop(engine);
    let db = Database::open(&home.join("workspaces.fjall"))?;
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
                table.insert(
                    format!("fixture_payload/{index:06}").as_str(),
                    bytes.as_slice(),
                )?;
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
    let database_bytes = engine.db.disk_space()?;
    drop(engine);
    std::fs::write(
        root.join("fixture.json"),
        serde_json::to_vec_pretty(
            &json!({"payload_mib":mib,"repositories":repositories,"workspaces":repositories*4}),
        )?,
    )?;
    println!(
        "{}",
        json!({"database_bytes":database_bytes,"payload_mib":mib,"repositories":repositories,"workspaces":repositories*4})
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
            .get("fixture_payload/000000")?
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

/// Test-only legacy export. This is the only writer of Zstd/redb here and is
/// never linked into the product. Both formats carry identical logical data.
fn legacy_copy(root: &Path) -> Result<()> {
    let db = Database::open(&root.join("store/workspaces.fjall"))?;
    let read = db.begin_read()?;
    let destination = root.join("legacy-store");
    std::fs::create_dir(&destination)?;
    let legacy = redb::Database::builder()
        .set_cache_size(32 * 1024 * 1024)
        .create(destination.join("workspaces.redb"))?;
    for (source, name) in [
        (FILLER, "records_v1"),
        (vwork_engine::git_store::OBJECTS, "git_objects_v1"),
        (vwork_engine::git_store::MEMBERS, "git_members_v1"),
        (vwork_engine::git_store::OVERLAYS, "git_members_v1"),
    ] {
        let records = read.open_table(source)?;
        let mut tx = legacy.begin_write()?;
        let mut count = 0;
        let mut bytes = 0;
        for row in records.iter()? {
            let (key, value) = row?;
            let value = if name == "git_objects_v1" {
                let (kind, raw) =
                    vwork_engine::git_store::read(&db, git2::Oid::from_str(key.value())?)?;
                let mut record = vec![kind.raw() as u8];
                record.extend_from_slice(&(raw.len() as u64).to_le_bytes());
                record.extend(zstd::encode_all(raw.as_slice(), 3)?);
                record
            } else {
                value.value().to_vec()
            };
            tx.open_table(redb::TableDefinition::<&str, &[u8]>::new(name))?
                .insert(key.value(), value.as_slice())?;
            count += 1;
            bytes += value.len();
            if count >= 128 || bytes >= 16 * 1024 * 1024 {
                tx.commit()?;
                tx = legacy.begin_write()?;
                count = 0;
                bytes = 0;
            }
        }
        tx.commit()?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("window_runtime::tests") {
        return Ok(());
    }
    if matches!(
        args.get(1).map(String::as_str),
        Some("--seed" | "--seed-both")
    ) {
        ensure!(args.len() == 5, "Usage: --seed NEW_ROOT MIB REPOSITORIES");
        seed(Path::new(&args[2]), args[3].parse()?, args[4].parse()?)?;
        if args[1] == "--seed-both" {
            legacy_copy(Path::new(&args[2]))?;
        }
        return Ok(());
    }
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("fixture");
    seed(&root, 1, 1)?;
    verify(&root)?;
    println!("v87_clean_store_with_repositories: PASS");
    Ok(())
}

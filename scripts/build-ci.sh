#!/usr/bin/env bash
# Runs inside Actions on the Mac; cached compilation only, never GUI tests.
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${FASTROCK_BUILD_CACHE:-${RUNNER_WORKSPACE}/fastrock-target}"
export CARGO_INCREMENTAL=1
python3 dev/check-build-policy.py
python3 -c 'import shutil; free=shutil.disk_usage(".").free; print(f"Mac free space: {free / 2**30:.1f} GiB"); assert free > 10*2**30, "Clean inactive build outputs before continuing"'
mkdir -p build/release-inputs
release_profile=$(python3 -c 'from scripts.release_config import build_plan; print(build_plan()["profile"])')
cargo xwin build --locked --profile "$release_profile" --target x86_64-pc-windows-msvc --bin fastrock
cargo build --locked --profile "$release_profile" --bin fastrock
cargo test --locked --lib -- --skip window_runtime::tests 2>&1 | tee build/headless-tests.log
python3 - <<'PY'
import hashlib,json,os,pathlib,subprocess,tomllib,shutil,zipfile
from scripts.release_config import build_plan
from scripts.debug_symbols import verify_windows,verify_macos
plan=build_plan()
output="debug" if plan["profile"]=="dev" else plan["profile"]
target=pathlib.Path(os.environ["CARGO_TARGET_DIR"])
root=pathlib.Path('build/release-inputs')
for name in ('fastrock.exe','fastrock','fastrock.pdb','fastrock.dSYM.zip','BUILD-INPUT.json'):
    (root/name).unlink(missing_ok=True)
for name,source in [('fastrock.exe',target/'x86_64-pc-windows-msvc'/output/'fastrock.exe'),('fastrock',target/output/'fastrock')]:
    shutil.copy2(source,root/name)
symbols={}
if not plan['prerelease']:
    pdb=target/'x86_64-pc-windows-msvc'/output/'fastrock.pdb'
    symbols['windows']=verify_windows(root/'fastrock.exe',pdb)
    shutil.copy2(pdb,root/'fastrock.pdb')
    dsym=target/output/'fastrock.dSYM'
    symbols['macos']=verify_macos(root/'fastrock',dsym)
    with zipfile.ZipFile(root/'fastrock.dSYM.zip','w',compression=zipfile.ZIP_STORED) as archive:
        for path in sorted(dsym.rglob('*')):
            if path.is_file(): archive.write(path,path.relative_to(dsym.parent))
assert subprocess.check_output(['git','status','--porcelain','--untracked-files=no'],text=True)==''
metadata={'source_commit':os.environ['GITHUB_SHA'], 'source_dirty':False,
          'workflow_run':os.environ['GITHUB_RUN_ID'], 'release_tag':os.environ['GITHUB_REF_NAME'], 'version':tomllib.load(open('Cargo.toml','rb'))['package']['version'],
          **plan, 'symbols':symbols,
          'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),
          'binary_sha256':{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in root.iterdir() if p.is_file() and p.name!='BUILD-INPUT.json'}}
(root/'BUILD-INPUT.json').write_text(json.dumps(metadata,indent=2)+'\n')
PY

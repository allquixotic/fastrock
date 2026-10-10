#!/usr/bin/env bash
# Runs inside Actions on the Mac; cached compilation only, never GUI tests.
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${FASTROCK_BUILD_CACHE:-${RUNNER_WORKSPACE}/fastrock-target}"
export CARGO_INCREMENTAL=1
python3 dev/check-build-policy.py
python3 -c 'import shutil; free=shutil.disk_usage(".").free; print(f"Mac free space: {free / 2**30:.1f} GiB"); assert free > 10*2**30, "Clean inactive build outputs before continuing"'
mkdir -p build/release-inputs
cargo xwin build --locked --target x86_64-pc-windows-msvc --bin fastrock
cargo build --locked --bin fastrock
cargo test --locked --lib -- --skip window_runtime::tests 2>&1 | tee build/headless-tests.log
cp "$CARGO_TARGET_DIR/x86_64-pc-windows-msvc/debug/fastrock.exe" build/release-inputs/
cp "$CARGO_TARGET_DIR/debug/fastrock" build/release-inputs/
python3 - <<'PY'
import hashlib,json,os,pathlib,subprocess,tomllib
root=pathlib.Path('build/release-inputs')
assert subprocess.check_output(['git','status','--porcelain','--untracked-files=no'],text=True)==''
metadata={'source_commit':os.environ['GITHUB_SHA'], 'source_dirty':False,
          'workflow_run':os.environ['GITHUB_RUN_ID'], 'version':tomllib.load(open('Cargo.toml','rb'))['package']['version'],
          'profile':'dev', 'opt_level':0, 'debug_info':0, 'lto':False,
          'codegen_units':256, 'incremental':True,
          'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),
          'binary_sha256':{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in root.iterdir() if p.is_file()}}
(root/'BUILD-INPUT.json').write_text(json.dumps(metadata,indent=2)+'\n')
PY

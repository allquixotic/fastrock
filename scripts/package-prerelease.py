#!/usr/bin/env python3
"""Package a signed Windows binary and matching optional debug symbols."""
import argparse, hashlib, json, pathlib, subprocess, tomllib, zipfile
from release_config import build_plan, suffix
from debug_symbols import verify_windows, zip_symbols
root = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=pathlib.Path)
parser.add_argument('output', type=pathlib.Path)
parser.add_argument('--signing-evidence', type=pathlib.Path)
parser.add_argument('--build-input', type=pathlib.Path)
args = parser.parse_args()
assert args.binary.name == 'fastrock.exe' and args.binary.is_file(), 'Expected compiled Windows fastrock.exe'
version = tomllib.loads((root/'Cargo.toml').read_text())['package']['version']
plan=build_plan(version)
args.output.mkdir(parents=True, exist_ok=True)
archive = args.output / f'fastrock-{version}-windows-x64{suffix(plan)}.zip'
revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip()
dirty=bool(subprocess.check_output(['git','status','--porcelain','--untracked-files=no'],cwd=root,text=True).strip())
metadata = {'source_commit':revision,'source_dirty':dirty,'binary_sha256':hashlib.sha256(args.binary.read_bytes()).hexdigest(),**plan, 'codex_backend':'installed external codex app-server',
            'minimum_codex_version':'0.162.0', 'source':json.loads((root/'codex-gui-base.json').read_text())}
if args.signing_evidence:
    signature=json.loads(args.signing_evidence.read_text(encoding='utf-8-sig'))
    assert signature['status']=='Valid' and signature['timestamp_publisher']
    assert signature['binary_sha256']==metadata['binary_sha256']
    assert signature['source_commit']==revision
    metadata['signing']=signature
if args.build_input:
    build_input=json.loads(args.build_input.read_text())
    assert build_input['source_commit']==revision and build_input['version']==version
    assert not dirty
    metadata['ci_build']=build_input
if __import__('os').environ.get('GITHUB_ACTIONS')=='true':
    assert args.signing_evidence and args.build_input, 'CI releases require signature and build evidence'
if not plan['prerelease']:
    assert args.build_input, 'Official release needs CI input and symbol provenance'
    pdb=args.binary.with_name('fastrock.pdb')
    identity=verify_windows(args.binary,pdb)
    assert identity==build_input['symbols']['windows']
    assert hashlib.sha256(pdb.read_bytes()).hexdigest()==build_input['binary_sha256']['fastrock.pdb']
    metadata['debug_symbols']=identity
    zip_symbols(args.output/f'fastrock-{version}-windows-x64-symbols.zip', [(pdb,'fastrock.pdb')], metadata)
with zipfile.ZipFile(archive,'w',compression=zipfile.ZIP_STORED) as z:
    z.write(args.binary, 'fastrock.exe')
    for filename in ['LICENSE','NOTICE','README.md','docs/RALLY-INVENTORY.md','docs/PORT-VERIFICATION.md','docs/gui.md','docs/releases.md']:
        if (root/filename).is_file(): z.write(root/filename, filename)
    for path in sorted((root/'docs/images').glob('*.png')):
        z.write(path, path.relative_to(root).as_posix())
    for path in sorted((root/'third_party/slint/i-slint-core/LICENSES').glob('*')):
        if path.is_file(): z.write(path, 'licenses/slint/'+path.name)
    z.writestr('BUILD.json',json.dumps(metadata,indent=2)+'\n')
(args.output / (archive.name+'.sha256')).write_text(hashlib.sha256(archive.read_bytes()).hexdigest()+'  '+archive.name+'\n')
print(f'Packaged {archive.name} ({archive.stat().st_size:,} bytes; no compression wait).')

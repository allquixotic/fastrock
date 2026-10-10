#!/usr/bin/env python3
"""Package a fast Windows development binary; never compile or optimize it."""
import argparse, hashlib, json, pathlib, subprocess, tomllib, zipfile
root = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=pathlib.Path)
parser.add_argument('output', type=pathlib.Path)
args = parser.parse_args()
assert args.binary.name == 'fastrock.exe' and args.binary.is_file(), 'Expected compiled Windows fastrock.exe'
version = tomllib.loads((root/'Cargo.toml').read_text())['package']['version']
args.output.mkdir(parents=True, exist_ok=True)
archive = args.output / f'fastrock-{version}-windows-x64-fast.zip'
revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip()
dirty=bool(subprocess.check_output(['git','status','--porcelain','--untracked-files=no'],cwd=root,text=True).strip())
metadata = {'source_commit':revision,'source_dirty':dirty,'binary_sha256':hashlib.sha256(args.binary.read_bytes()).hexdigest(),'version':version, 'profile':'dev', 'opt_level':0, 'debug_info':0, 'lto':False,
            'codegen_units':256, 'codex_backend':'installed external codex app-server',
            'minimum_codex_version':'0.162.0', 'source':json.loads((root/'codex-gui-base.json').read_text())}
with zipfile.ZipFile(archive,'w',compression=zipfile.ZIP_STORED) as z:
    z.write(args.binary, 'fastrock.exe')
    for filename in ['LICENSE','NOTICE','README.md','docs/RALLY-INVENTORY.md','docs/PORT-VERIFICATION.md']:
        if (root/filename).is_file(): z.write(root/filename, filename)
    for path in sorted((root/'third_party/slint/i-slint-core/LICENSES').glob('*')):
        if path.is_file(): z.write(path, 'licenses/slint/'+path.name)
    z.writestr('BUILD.json',json.dumps(metadata,indent=2)+'\n')
(args.output / (archive.name+'.sha256')).write_text(hashlib.sha256(archive.read_bytes()).hexdigest()+'  '+archive.name+'\n')
print(f'Packaged {archive.name} ({archive.stat().st_size:,} bytes; no compression wait).')

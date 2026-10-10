#!/usr/bin/env python3
"""CI-only publication; preserve previously published asset bytes."""
import json,os,pathlib,subprocess,sys,tomllib
assert os.environ.get('GITHUB_ACTIONS')=='true','Publish through GitHub Actions only'
version=tomllib.load(open('Cargo.toml','rb'))['package']['version']
tag='v'+version
assert os.environ['GITHUB_REF_NAME']==tag
assert any(marker in version for marker in ('-alpha.','-beta.','-rc.'))
dist=pathlib.Path(sys.argv[1])
assets=sorted(p for p in dist.iterdir() if p.is_file() and p.name.endswith(('.zip','.dmg','.sha256','.sigstore.json')))
assert assets
for asset in assets:
    if asset.suffix in ('.zip','.dmg'):
        subprocess.run(['gh','attestation','verify',str(asset),'--repo',os.environ['GH_REPO'],
                        '--signer-workflow',os.environ['GH_REPO']+'/.github/workflows/release.yml',
                        '--source-ref',os.environ['GITHUB_REF'],'--source-digest',os.environ['GITHUB_SHA']],check=True)
view=subprocess.run(['gh','release','view',tag,'--json','isPrerelease,assets'],text=True,capture_output=True)
if view.returncode:
    notes=pathlib.Path('docs/prerelease-notes.md')
    subprocess.run(['gh','release','create',tag,*map(str,assets),'--verify-tag','--prerelease','--title',f'Fastrock {version} — native Slint Rally','--notes-file',str(notes)],check=True)
else:
    release=json.loads(view.stdout)
    assert release['isPrerelease'],'Never overwrite a stable release'
    names={p['name'] for p in release['assets']}
    for asset in assets:
        if asset.name not in names:
            subprocess.run(['gh','release','upload',tag,str(asset)],check=True)
print('Published signed CI assets:',tag)

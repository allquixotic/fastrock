#!/usr/bin/env python3
"""Run one trusted release job on this Mac, then remove its runner registration.

Launch once with build and once with signing. No service or exported Apple key.
The persistent Cargo target cache stays on disk between releases.
"""
import argparse,hashlib,json,os,pathlib,subprocess,sys,tarfile,urllib.request
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('role',choices=('build','signing'))
args=parser.parse_args()
assert sys.platform=='darwin' and os.uname().machine=='arm64'
os.umask(0o077)
root=pathlib.Path(__file__).resolve().parents[1]
role_dir=root.parent/f'fastrock-ci-{args.role}'
role_dir.mkdir(exist_ok=True)
repo='allquixotic/fastrock'

def api(path,method='GET'):
    result=subprocess.run(['gh','api','--method',method,path],capture_output=True,text=True)
    if result.returncode: raise RuntimeError('GitHub runner API failed; credential output withheld')
    return json.loads(result.stdout) if result.stdout.strip() else None

if not (role_dir/'config.sh').exists():
    downloads=api(f'repos/{repo}/actions/runners/downloads')
    release=next(item for item in downloads if item['os']=='osx' and item['architecture']=='arm64')
    archive=role_dir/'runner.tar.gz'
    if not archive.exists() or hashlib.sha256(archive.read_bytes()).hexdigest()!=release['sha256_checksum']:
        urllib.request.urlretrieve(release['download_url'],archive)
    assert hashlib.sha256(archive.read_bytes()).hexdigest()==release['sha256_checksum']
    with tarfile.open(archive) as source:
        source.extractall(role_dir,filter='data')
if not (role_dir/'.runner').exists():
    token=api(f'repos/{repo}/actions/runners/registration-token',method='POST')['token']
    registration=subprocess.run([str(role_dir/'config.sh'),'--unattended','--ephemeral','--disableupdate',
        '--url',f'https://github.com/{repo}','--token',token,'--name',f'fastrock-{args.role}-{os.getpid()}',
        '--labels',f'fastrock-{args.role}','--work','work'],cwd=role_dir,capture_output=True,text=True)
    del token
    assert registration.returncode==0,'Runner registration failed; credential output withheld'
# Runner configuration is UTF-8 with a BOM; resume interrupted registration.
runner_id=json.loads((role_dir/'.runner').read_text(encoding='utf-8-sig'))['agentId']
registered=next(runner for runner in api(f'repos/{repo}/actions/runners')['runners'] if runner['id']==runner_id)
assert registered['status']=='offline' and not registered['busy'],'Runner is already active'
print(f'Temporary {args.role} runner {runner_id} registered; one release job only.',flush=True)
env=os.environ.copy()
env['FASTROCK_BUILD_CACHE']=str(root/'target')
try:
    result=subprocess.run([str(role_dir/'run.sh')],cwd=role_dir,env=env)
    assert result.returncode==0,'Runner job failed; inspect Actions evidence'
finally:
    runners=api(f'repos/{repo}/actions/runners')['runners']
    if any(runner['id']==runner_id for runner in runners):
        api(f'repos/{repo}/actions/runners/{runner_id}',method='DELETE')
    for name in ('.runner','.credentials','.credentials_rsaparams'):
        (role_dir/name).unlink(missing_ok=True)
    print(f'Temporary {args.role} runner removed; incremental build cache retained.',flush=True)

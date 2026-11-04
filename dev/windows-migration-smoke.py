#!/usr/bin/env python3
"""V96: real legacy migration, native progress/pause/resume and clean restart."""
import argparse, hashlib, json, os, shutil, subprocess, time
from pathlib import Path
from embedded_provider_fixture import ProviderFixture, ProcessGuard
from benchmark_guard import BoundedJob, headroom, guest_headroom
assert os.name=='nt', 'Native acceptance runs on games only'
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('binary',type=Path)
p.add_argument('--seed-executable',type=Path,required=True)
guard_options=p.add_mutually_exclusive_group(required=True)
guard_options.add_argument('--host-guard',type=Path)
guard_options.add_argument('--ci',action='store_true',help='Hosted Windows Actions runner only')
p.add_argument('--output',type=Path,required=True)
p.add_argument('--mib',type=int,default=512)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
if a.ci:
    assert os.environ.get('GITHUB_ACTIONS')=='true' and os.environ.get('RUNNER_ENVIRONMENT')=='github-hosted', 'CI mode requires a hosted Actions runner'
def available():
    return min(2_000_000_000,headroom(a.host_guard) if a.host_guard else 2_000_000_000,guest_headroom())
def digest(path):
    value=hashlib.sha256()
    with path.open('rb') as f:
        for chunk in iter(lambda:f.read(1024*1024),b''):value.update(chunk)
    return value.hexdigest()
def run(command,env,folder,name,timeout=180):
    with (folder/(name+'.log')).open('w',encoding='utf-8') as log:
        child=subprocess.Popen([str(x) for x in command],env=env,stdout=log,stderr=log)
        job=BoundedJob(child,available())
        guard=ProcessGuard(child,timeout=timeout)
        try:
            def tick():available();job.sample()
            guard.wait(tick)
            return guard.peak
        finally:job.close()
def drive(folder,env,name,steps):
    script=folder/(name+'-steps.json');script.write_text(json.dumps(steps),encoding='utf-8')
    return run([binary],dict(env,CODEX_GUI_AUTOMATION=str(script)),folder,name)
def wait(active=False,failed=False):return {'wait_migration':dict(active=active,failed=failed,timeout_ms=120000)}
def dump(path):return [{'virtual_workspaces':['migration-dump',str(path)]},{'wait':300}]
seed=out/'seed'
peak=run([a.seed_executable.resolve(),'--seed-both',seed,str(a.mib),'2'],os.environ.copy(),out,'seed')
source=seed/'legacy-store/workspaces.redb';before=digest(source)
providers=[]
try:
    manual=out/'manual';manual.mkdir();provider=ProviderFixture();providers.append(provider);env=provider.environment(manual)
    target=manual/'imported';target.mkdir()
    peak=max(peak,drive(manual,env,'manual',[
        {'wait_ready':30000},{'resize':[1440,1000]},
        {'virtual_workspaces':['migrate',str(source),str(target)]},wait(True),
        {'snapshot':str(manual/'progress.png')},*dump(manual/'active.json'),
        {'click_text':'Pause'},wait(False,True),*dump(manual/'paused.json'),
        {'snapshot':str(manual/'paused.png')},{'wait':1200},
        {'click_text':'Resume'},wait(True),wait(),*dump(manual/'complete.json'),
        {'snapshot':str(manual/'complete.png')},{'quit':True}]))
    assert digest(source)==before,'Manual migration changed source'
    active=json.loads((manual/'active.json').read_text());paused=json.loads((manual/'paused.json').read_text());done=json.loads((manual/'complete.json').read_text())
    assert any(p['active'] for p in active),active
    assert any(p['error'] and 'paused' in p['error'].lower() for p in paused),paused
    assert any(not p['active'] and not p['error'] and p['completed']==p['total'] for p in done),done
    assert (target/'workspaces.fjall/version').is_file()
    registry=json.loads((manual/'local/vwork/fastrock-imported-stores.json').read_text())
    assert any(Path(path).samefile(target) for path in registry),registry
    auto=out/'automatic';auto.mkdir();provider=ProviderFixture();providers.append(provider);env=provider.environment(auto)
    home=auto/'local/vwork';shutil.copytree(seed/'legacy-store',home)
    peak=max(peak,drive(auto,env,'automatic',[
        {'wait_ready':30000},{'new_thread':str(auto/'project')},wait(True),
        {'resize':[1440,1000]},{'snapshot':str(auto/'progress.png')},wait(),
        {'wait_idle':120000},{'send':'after automatic migration'},{'wait_idle':30000},
        *dump(auto/'complete.json'),{'runtime_facts':str(auto/'runtime.json')},{'quit':True}]))
    assert digest(home/'workspaces.redb')==before,'Automatic migration changed source'
    # Startup must prefer the activated Fjall store without reopening the source.
    (home/'workspaces.redb').rename(home/'retained-source.redb')
    peak=max(peak,drive(auto,env,'restart',[
        {'wait_ready':30000},{'new_thread':str(auto/'project')},{'wait_idle':30000},
        {'send':'after clean restart'},{'wait_idle':30000},*dump(auto/'restart-progress.json'),
        {'runtime_facts':str(auto/'restart-runtime.json')},{'quit':True}]))
    assert json.loads((auto/'restart-progress.json').read_text())==[]
    assert all(not provider.errors for provider in providers)
    receipt=dict(passed=True,peakBytes=peak,sha256=digest(binary),sourceSha256=before,
                 checks=['manual native progress','native Pause and Resume','registry discovery','automatic detection','first thread handoff','byte-exact source preservation','clean reopen without redb'])
    (out/'summary.json').write_text(json.dumps(receipt,indent=2),encoding='utf-8');print(json.dumps(receipt),flush=True)
finally:
    for provider in providers:provider.close()

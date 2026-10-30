#!/usr/bin/env python3
"""V74/V75: native isolated side chats, hidden history, recovery and explicit discard."""
import argparse, base64, hashlib, json, os, sqlite3, subprocess, time
from pathlib import Path
from embedded_provider_fixture import ProviderFixture, ProcessGuard
assert os.name == 'nt', 'Run GUI acceptance on games only'
p=argparse.ArgumentParser(description=__doc__);p.add_argument('binary',type=Path);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
repo=out/'project';provider=ProviderFixture(workspace_repo=repo);env=provider.environment(out,streaming=True);env['FASTROCK_WEBHOOK_BIND']='127.0.0.1:0'
config=out/'goose/config/config.yaml';value=json.loads(config.read_text(encoding='utf-8'))
value['extensions']={'vwork':dict(enabled=True,name='vwork',type='stdio',cmd=str(out/'absent/vwork.exe'),args=['--home',str(out/'local/vwork'),'mcp'],timeout=30)};config.write_text(json.dumps(value),encoding='utf-8')
(repo/'src').mkdir();original=b'original side source\r\n';(repo/'src/main.rs').write_bytes(original)
git_env={k:v for k,v in os.environ.items() if not k.upper().startswith('GIT_')};git_env.update(GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL=str(out/'aws-empty'))
for args in (['init','-b','main','.'],['add','.'],['-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','Side fixture']):
 subprocess.run(['git','-c','core.hooksPath=NUL','-c','core.autocrlf=false',*args],cwd=repo,env=git_env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,check=True)
def complete():return [{'wait':400},{'wait_idle':30000}]
def export(name):return [{'transcript':'export-complete:'+str(out/(name+'.md'))},{'wait':500}]
def run(name,steps,crash=False):
 script=out/(name+'-steps.json');script.write_text(json.dumps(steps),encoding='utf-8')
 with (out/(name+'.log')).open('w',encoding='utf-8') as log:
  process=subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
  guard=ProcessGuard(process,timeout=120)
  if not crash:return guard.wait()
  try:
   while process.poll() is None and not any(r['prompt']=='hold-side-crash' for r in provider.requests):guard.sample();time.sleep(.1)
   assert process.poll() is None,'Side chat failed before crash'
   time.sleep(.8);guard.sample();process.kill();process.wait(timeout=10)
  finally:
   if process.poll() is None:process.kill();process.wait(timeout=10)
  return guard.peak
def bindings():return [(path,json.loads(path.read_text(encoding='utf-8'))) for path in out.rglob('*.binding.json')]
def chat_snapshot():
 paths=list(out.rglob('fastrock-open-chats.json'));assert len(paths)==1,paths
 return json.loads(paths[0].read_text(encoding='utf-8'))
def dbrows():
 database,=(out/'goose').rglob('sessions.db')
 with sqlite3.connect(database) as db:return db.execute('SELECT id, session_type, working_dir FROM sessions').fetchall()
try:
 peak=run('create',[{'wait_ready':30000},{'resize':[1440,1000]},{'open_folder':str(repo)},{'wait':1000},{'wait_idle':30000},
  {'send':'completed-parent-reference'},*complete(),{'send':'hold-side-parent'},{'wait':600},
  {'tab_action':'side'},{'wait':600},{'wait_idle':30000},{'send':'vwork-child-edit'},*complete(),
  {'composer':['type','Preserve side draft é']},{'palette':['dump',str(out/'draft.json')]},{'wait':300},
  *export('side'),{'snapshot':str(out/'side.png')},{'quit':True}])
 side_records=[(p,v) for p,v in bindings() if v.get('side')];assert len(side_records)==1,bindings()
 path,side=side_records[0];parent=side['forked_from'];encoded=path.name.removesuffix('.binding.json');side_id=base64.urlsafe_b64decode(encoded+'='*(-len(encoded)%4)).decode()
 assert side['workspace']['selection']['workspace'] != next(v['workspace']['selection']['workspace'] for _,v in bindings() if not v.get('side'))
 assert (repo/'src/main.rs').read_bytes()==original
 requests=[r for r in provider.requests if r['prompt']=='vwork-child-edit'];assert requests,[(r['summary'],r['prompt'][:100]) for r in provider.requests]
 first=json.dumps(requests[0]['body']);assert 'completed-parent-reference' in first and 'Side conversation boundary.' in first
 assert 'hold-side-parent' not in first,first
 names=[t.get('toolSpec',{}).get('name','') for t in requests[0]['body'].get('toolConfig',{}).get('tools',[])]
 assert not any('fastrock_agents' in n or 'codex_gui' in n for n in names),names
 assert any(row[0]==side_id and row[1]=='hidden' for row in dbrows()),dbrows()
 chats=[c for w in chat_snapshot()['windows'].values() for c in w['chats']]
 assert any(c['side_parent'] and c['draft']['text']=='Preserve side draft é' for c in chats),chats
 before=len(provider.requests)
 peak=max(peak,run('restart',[{'wait_ready':30000},{'wait':1600},{'palette':['dump',str(out/'restored-draft.json')]},{'wait':300},*export('restored'),{'quit':True}]))
 assert not [r for r in provider.requests[before:] if not r['summary']],'Restart replayed side work'
 assert json.loads((out/'restored-draft.json').read_text(encoding='utf-8'))['draft']=='Preserve side draft é'
 assert 'vwork-child-edit' in (out/'restored.md').read_text(encoding='utf-8')
 peak=max(peak,run('crash',[{'wait_ready':30000},{'wait':1500},{'click_text':'Resume'},{'wait':300},{'send':'hold-side-crash'},{'wait':120000}],True))
 before=len(provider.requests)
 peak=max(peak,run('discard',[{'wait_ready':30000},{'wait':1500},*export('crash-restored'),{'shortcut':'close-tab'},{'wait':700},
  {'new_thread':str(repo)},{'wait_idle':30000},{'tab_action':'side'},{'wait':600},{'wait_idle':30000},{'send':'ordinary-side-question'},*complete(),{'quit':True}]))
 assert not [r for r in provider.requests[before:] if not r['summary'] and r['prompt']!='ordinary-side-question'],'Crash replayed side work'
 assert not any(row[0]==side_id for row in dbrows()),dbrows()
 for suffix in ['json','binding.json','tools.json','vwork.json','delegates.json','outcome.json','delegate-result.json','last-dispatch.json']:
  assert not path.with_name(encoded+'.'+suffix).exists(),f'Temporary sidecar retained: {suffix}'
 assert sum(r['prompt']=='hold-side-crash' for r in provider.requests)==1
 ordinary=[r for r in provider.requests if r['prompt']=='ordinary-side-question'];assert len(ordinary)==1
 current=[c for w in chat_snapshot()['windows'].values() for c in w['chats'] if c['side_parent']]
 assert len(current)==1 and Path(current[0]['cwd'])!=repo and current[0]['thread_id']!='goose:'+side_id,current
 live_id=current[0]['thread_id'].removeprefix('goose:')
 live_encoded=base64.urlsafe_b64encode(live_id.encode()).decode().rstrip('=')
 peak=max(peak,run('close-running',[{'wait_ready':30000},{'wait':1500},{'click_text':'Resume'},{'wait':300},
  {'send':'hold-side-explicit-close'},{'wait':700},{'shortcut':'close-tab'},{'wait':300},{'dialog':True},{'wait':2500},{'quit':True}]))
 assert not any(row[0]==live_id for row in dbrows()),dbrows()
 assert not list(path.parent.glob(live_encoded+'.*')),'Late turn completion recreated temporary metadata'
 assert not provider.errors,provider.errors
 receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,native_side_while_parent_running=True,hidden_reference_boundary=True,isolated_virtual_workspace=True,app_delegation_excluded=True,original_source_unchanged=True,quit_and_crash_recovery=True,draft_preserved=True,explicit_close_discards=True,no_id_reuse=True,ordinary_side_directory_isolated=True,no_restart_replay=True)
 (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n');print('PASS: native side isolation, reference boundary, recovery, draft and discard',flush=True)
finally:
 provider.close();provider.write_evidence(out)

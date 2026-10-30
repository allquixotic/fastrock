#!/usr/bin/env python3
"""Native checkpoints: turn capture, guarded restore, backup and restart recovery."""
import argparse, hashlib, json, os, subprocess, sqlite3, time
from pathlib import Path
from embedded_provider_fixture import ProviderFixture, ProcessGuard
assert os.name=='nt','Run GUI acceptance on games only'
p=argparse.ArgumentParser(description=__doc__);p.add_argument('binary',type=Path);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
repo=out/'project';provider=ProviderFixture(workspace_repo=repo);env=provider.environment(out)
env['FASTROCK_WEBHOOK_BIND']='127.0.0.1:0'
config=out/'goose/config/config.yaml';value=json.loads(config.read_text(encoding='utf-8'))
value['extensions']={'vwork':dict(enabled=True,name='vwork',type='stdio',cmd=str(out/'absent/vwork.exe'),args=['--home',str(out/'local/vwork'),'mcp'],timeout=30)};config.write_text(json.dumps(value),encoding='utf-8')
(repo/'src').mkdir();original=b'original checkpoint source\r\n';(repo/'src/main.rs').write_bytes(original)
git_env={k:v for k,v in os.environ.items() if not k.upper().startswith('GIT_')};git_env.update(GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL=str(out/'aws-empty'))
for args in (['init','.'],['add','src/main.rs'],['-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','Fixture']):
 subprocess.run(['git','-c','core.hooksPath=NUL','-c','core.autocrlf=false',*args],cwd=repo,env=git_env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,check=True)
def cp(*args):return {'checkpoints':list(args)}
def dump(name):return [cp('dump',str(out/(name+'.json'))),{'wait':180}]
def read(name):return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def review(index):return [cp('select',str(index)),{'wait':600}]
def export(name):return [{'transcript':'export-complete:'+str(out/(name+'.md'))},{'wait':400}]
def run(name,steps,extra=None):
 script=out/(name+'-steps.json');script.write_text(json.dumps(steps),encoding='utf-8')
 with (out/(name+'.log')).open('w',encoding='utf-8') as log:
  process=subprocess.Popen([str(binary)],env=dict(env,**(extra or {}),CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
  return ProcessGuard(process,timeout=120).wait()
try:
 peak=run('normal',[{'wait_ready':30000},{'resize':[1440,1000]},{'open_folder':str(repo)},{'wait':800},{'wait_idle':30000},
  {'click_text':'Checkpoints'},{'wait':600},cp('label','Baseline'),{'click_text':'Save checkpoint'},{'wait':600},*dump('manual'),{'key':'escape'},
  {'send':'checkpoint first question'},{'wait':300},{'wait_idle':30000},{'send':'vwork-child-edit'},{'wait':300},{'wait_idle':30000},
  {'composer':['type','Retain checkpoint draft']},{'click_text':'Checkpoints'},{'wait':600},*dump('turns'),*review(1),*dump('review'),{'snapshot':str(out/'checkpoints.png')},
  {'click_text':'Restore selected'},{'wait':1700},*dump('restored'),{'key':'escape'},{'wait_idle':30000},*export('rewound'),{'palette':['dump',str(out/'draft.json')]},{'wait':200},
  {'click_text':'Checkpoints'},{'wait':500},*review(0),*dump('backup-review'),{'click_text':'Restore selected'},{'wait':1500},*dump('backup-restored'),{'key':'escape'},{'wait_idle':30000},*export('recovered'),
  {'click_text':'Resume'},{'wait':400},{'click_text':'Checkpoints'},{'wait':500},*review(2),{'send':'checkpoint newer question'},{'wait':300},{'wait_idle':30000},
  {'click_text':'Restore selected'},{'wait':700},*dump('stale'),{'key':'escape'},{'quit':True}])
 assert len(read('manual')['rows'])==1,read('manual')
 assert len(read('turns')['rows'])==5,read('turns')
 assert read('review')['canFiles'] and read('review')['canConversation'],read('review')
 assert 'fork isolated λ' in read('review')['review']['review']['diff'],read('review')
 assert 'vwork-child-edit' not in (out/'rewound.md').read_text(encoding='utf-8')
 assert 'vwork-child-edit' in (out/'recovered.md').read_text(encoding='utf-8')
 assert read('draft')['draft']=='Retain checkpoint draft',read('draft')
 assert 'changed after review' in read('stale')['status'],read('stale')
 assert (repo/'src/main.rs').read_bytes()==original
 thread=read('review')['review']['review']['checkpoint']['session']
 # The automation-only fault runs after the durable intent and file restore, before
 # the Goose write. Relaunch must leave it pending and never run provider work.
 interrupted=run('interrupted',[{'wait_ready':30000},{'wait':700},{'resume':'goose:'+thread},{'wait':700},{'wait_idle':30000},{'click_text':'Checkpoints'},{'wait':500},
  *review(8),{'click_text':'Restore selected'},{'wait':900},*dump('failed'),{'click_text':'Refresh'},{'wait':700},*dump('pending'),{'quit':True}],{'FASTROCK_CHECKPOINT_TEST_FAIL':'1'})
 assert 'fixture interruption' in read('failed')['status'].lower(),read('failed')
 assert read('pending')['blocked'] and read('pending')['recovery'],read('pending')
 ordinary_before=sum(not r['summary'] for r in provider.requests)
 recovered=run('restart',[{'wait_ready':30000},{'wait':700},{'resume':'goose:'+thread},{'wait':700},{'wait_idle':30000},{'click_text':'Checkpoints'},{'wait':700},*dump('restart-pending'),
  {'click_text':'Restore retained backup'},{'wait':1700},*dump('resolved'),{'key':'escape'},{'wait_idle':30000},*export('restart-recovered'),{'quit':True}])
 assert sum(not r['summary'] for r in provider.requests)==ordinary_before,'Restart/restore unexpectedly prompted a provider'
 assert read('restart-pending')['blocked'] and read('restart-pending')['recovery'],read('restart-pending')
 assert not read('resolved')['blocked'],read('resolved')
 assert 'checkpoint newer question' in (out/'restart-recovered.md').read_text(encoding='utf-8')
 assert (repo/'src/main.rs').read_bytes()==original
 baseline=len(read('resolved')['rows'])-1
 modes=run('modes',[{'wait_ready':30000},{'wait':600},{'resume':'goose:'+thread},{'wait':600},{'wait_idle':30000},{'click_text':'Checkpoints'},{'wait':500},
  *review(baseline),cp('conversation','false'),{'click_text':'Restore selected'},{'wait':1300},*review(baseline+1),*dump('files-only'),{'key':'escape'},*export('files-only'),
  {'click_text':'Checkpoints'},{'wait':500},*review(0),{'click_text':'Restore selected'},{'wait':1300},*review(baseline+2),cp('files','false'),{'click_text':'Restore selected'},{'wait':1300},
  *review(baseline+3),*dump('conversation-only'),{'key':'escape'},*export('conversation-only'),{'quit':True}])
 assert read('files-only')['review']['review']['diff']=='',read('files-only')
 assert 'checkpoint newer question' in (out/'files-only.md').read_text(encoding='utf-8')
 assert read('conversation-only')['review']['current']['count']==0,read('conversation-only')
 assert 'fork isolated λ' in read('conversation-only')['review']['review']['diff'],read('conversation-only')
 assert 'checkpoint newer question' not in (out/'conversation-only.md').read_text(encoding='utf-8')
 # Seed an isolated saved Goose session with an approved tool request whose
 # result was never persisted. Even prior approval must not activate on load.
 database_path,=(out/'goose').rglob('sessions.db')
 tool_name='vwork__files_read'
 contents=[
  ('uncertain-user','user',[dict(type='text',text='Uncertain fixture work')],True),
  ('uncertain-request','assistant',[dict(type='toolRequest',id='uncertain-tool',toolCall=dict(status='success',value=dict(name=tool_name,arguments=dict(path='src/main.rs'))))],True),
  ('uncertain-confirm','assistant',[dict(type='actionRequired',data=dict(actionType='toolConfirmation',id='uncertain-tool',toolName=tool_name,arguments=dict(path='src/main.rs'),prompt=None))],True),
  ('uncertain-approval','user',[dict(type='actionRequired',data=dict(actionType='toolConfirmationResponse',id='uncertain-tool',permission='allow_once'))],False)]
 with sqlite3.connect(database_path) as database:
  for mid,role,content,visible in contents:
   database.execute('INSERT INTO messages (message_id,session_id,role,content_json,created_timestamp,metadata_json) VALUES (?,?,?,?,?,?)',(mid,thread,role,json.dumps(content),int(time.time()),json.dumps(dict(user_visible=visible,agent_visible=visible))))
  stored_before=database.execute('SELECT message_id,role,content_json,metadata_json FROM messages WHERE session_id=? ORDER BY id',(thread,)).fetchall()
 ordinary_before=sum(not r['summary'] for r in provider.requests)
 uncertain=run('uncertain',[{'wait_ready':30000},{'wait':600},{'resume':'goose:'+thread},{'wait':600},{'wait_idle':30000},*export('uncertain'),{'click_text':'Resume'},{'wait':500},
  {'click_text':'Checkpoints'},{'wait':500},*dump('uncertain-points'),{'key':'escape'},{'quit':True}])
 assert 'Recovery review required' in (out/'uncertain.md').read_text(encoding='utf-8')
 assert 'Uncertain fixture work' in (out/'uncertain.md').read_text(encoding='utf-8')
 assert sum(not r['summary'] for r in provider.requests)==ordinary_before,'Approved tool continuation activated on history load'
 with sqlite3.connect(database_path) as database:
  assert database.execute('SELECT message_id,role,content_json,metadata_json FROM messages WHERE session_id=? ORDER BY id',(thread,)).fetchall()==stored_before,'Recovery view changed stored messages'
 provider.write_evidence(out)
 receipt=dict(binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_bytes=max(peak,interrupted,recovered,modes,uncertain),turn_checkpoints=4,checks=['manual/turn capture','native diff','file + exact conversation restore','retained backup','draft retention','stale review guard','original checkout unchanged','durable interrupted restore','restart review only','explicit rollback','files-only restore','conversation-only restore','approved uncertain tools never activated','read-only recovery preserves stored messages'])
 (out/'result.json').write_text(json.dumps(receipt,indent=2),encoding='utf-8');print(json.dumps(receipt,indent=2))
finally:provider.close()

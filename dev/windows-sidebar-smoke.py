#!/usr/bin/env python3
"""V67: new chats appear immediately under human-readable repository identity."""
import argparse,hashlib,json,os,subprocess
from pathlib import Path
from embedded_provider_fixture import ProviderFixture,ProcessGuard
assert os.name=='nt','Run GUI acceptance on games only'
p=argparse.ArgumentParser(description=__doc__);p.add_argument('binary',type=Path);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
provider=ProviderFixture();env=provider.environment(out,streaming=True);env['FASTROCK_WEBHOOK_BIND']='127.0.0.1:0';repo=out/'project'
(repo/'README.md').write_text('Sidebar fixture\n',encoding='utf-8')
git_env={k:v for k,v in os.environ.items() if not k.upper().startswith('GIT_')};git_env.update(GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL=str(out/'aws-empty'))
for args in (['init','.'],['add','.'],['-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','Fixture']):
 subprocess.run(['git','-c','core.hooksPath=NUL','-c','core.autocrlf=false',*args],cwd=repo,env=git_env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,check=True)
def dump(name):return [{'sidebar':['dump',str(out/(name+'.json'))]},{'wait':200}]
def read(name):return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def run(name,steps):
 script=out/(name+'-steps.json');script.write_text(json.dumps(steps),encoding='utf-8')
 with (out/(name+'.log')).open('w',encoding='utf-8') as log:
  process=subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
  return ProcessGuard(process,timeout=120).wait()
try:
 peak=run('live',[{'wait_ready':30000},{'resize':[1440,1000]},{'open_folder':str(repo)},{'wait':1000},{'wait_idle':30000},*dump('new'),
  {'send':'hold sidebar first stream'},{'wait':900},*dump('streaming'),{'snapshot':str(out/'sidebar.png')},
  {'click_text':'Archived'},{'wait':900},*dump('archived'),{'click_text':'Show active'},{'wait':900},*dump('active'),
  {'sidebar':['search','NoSuchThread987']},{'wait':700},*dump('filtered'),{'sidebar':['search','']},{'wait':700},*dump('restored'),
  {'shortcut':'escape'},{'wait_idle':10000},{'composer':['type','Keep sidebar draft']},
  {'key':'ctrl+shift+P'},{'wait':200},{'palette':['query','Goose recipes']},{'wait':200},{'key':'return'},{'wait':900},
  {'settings':['recipes','dump',str(out/'recipes.json')]},{'wait':200},
  {'key':'ctrl+shift+P'},{'wait':200},{'palette':['query','Tools and hooks']},{'wait':200},{'key':'return'},{'wait':900},
  {'settings':['capabilities','dump',str(out/'capabilities.json')]},{'wait':200},{'select_tab':0},{'wait':200},
  {'palette':['dump',str(out/'draft.json')]},{'wait':200},{'quit':True}])
 for name in ['new','streaming','active','restored']:
  state=read(name);assert len(state['rows'])==1,state
  assert len(state['projectFolders'])==1 and state['projectFolders'][0]['count']==1,state
  assert state['navigation']['activeScope']=='project',state
 assert read('new')['navigation']['activeTitle']=='New conversation',read('new')
 assert read('streaming')['rows'][0]['status']=='Running',read('streaming')
 assert read('new')['rows'][0]['age']=='now',read('new')
 assert read('streaming')['rows'][0]['title']=='hold sidebar first stream',read('streaming')
 assert not read('archived')['rows'],read('archived')
 assert not read('filtered')['rows'],read('filtered')
 assert read('recipes')['catalog']['projects'][0]['name']=='project',read('recipes')
 assert 'catalog' in read('capabilities') and not read('capabilities')['busy'],read('capabilities')
 assert read('draft')['draft']=='Keep sidebar draft',read('draft')
 before=sum(not r['summary'] for r in provider.requests)
 peak=max(peak,run('restart',[{'wait_ready':30000},{'wait':1500},*dump('restart'),{'quit':True}]))
 assert read('restart')['navigation']['activeScope']=='project',read('restart')
 assert sum(not r['summary'] for r in provider.requests)==before,'Restart prompted provider'
 assert not provider.errors,provider.errors
 receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,first_stream_visible=True,project_name_and_scope=True,search_archive_filters=True,native_palette_pages=True,draft_and_restart_identity=True)
 (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf-8');print('PASS: new/streaming sidebar, repository identity, filters, palette and restart',flush=True)
finally:provider.close();provider.write_evidence(out)

#!/usr/bin/env python3
"""V68: native sidebar preferences, working state, per-chat opt-out and restart."""
import argparse,hashlib,json,os,subprocess
from pathlib import Path
from embedded_provider_fixture import ProviderFixture,ProcessGuard
assert os.name=='nt','Run GUI acceptance on games only'
p=argparse.ArgumentParser(description=__doc__);p.add_argument('binary',type=Path);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
provider=ProviderFixture(stream_chunks=9000);env=provider.environment(out,streaming=True);env['FASTROCK_WEBHOOK_BIND']='127.0.0.1:0'
repo=out/'project';(repo/'README.md').write_text('Sidebar preference fixture\n')
git_env={k:v for k,v in os.environ.items() if not k.upper().startswith('GIT_')};git_env.update(GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL=str(out/'aws-empty'))
for args in (['init','.'],['add','.'],['-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','Fixture']):
 subprocess.run(['git','-c','core.hooksPath=NUL','-c','core.autocrlf=false',*args],cwd=repo,env=git_env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,check=True)
def dump(name):return [{'sidebar':['dump',str(out/(name+'.json'))]},{'wait':200}]
def read(name):return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def policy(key,value):return [{'sidebar':['policy',key,str(value)]},{'wait':700}]
def click(x,y):return [{'pointer':['press',x,y]},{'pointer':['release',x,y]},{'wait':700}]
def action(id,name):return [{'sidebar':['thread',id,name]},{'wait':700}]
def run(name,steps):
 script=out/(name+'-steps.json');script.write_text(json.dumps(steps),encoding='utf-8')
 with (out/(name+'.log')).open('w',encoding='utf-8') as log:
  process=subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
  return ProcessGuard(process,timeout=150).wait()
def row(state,id):return next(r for r in state['rows'] if r['id']==id)
try:
 peak=run('create',[{'wait_ready':30000},{'resize':[1440,1100]},{'open_folder':str(repo)},{'wait':900},{'wait_idle':30000},{'send':'Sidebar preference chat'},{'wait':300},{'wait_idle':30000},{'wait_summaries':30000},*dump('created'),{'quit':True}])
 created=read('created');id=created['navigation']['openChats'][0]['id']
 assert created['groupProjects'] and created['workingSection'] and created['autoSettleDays']==3,created
 peak=max(peak,run('configure',[{'wait_ready':30000},{'wait':1200},{'resize':[1440,1100]},
  {'open_settings':'appearance'},{'wait':600},{'snapshot':str(out/'settings.png')},
  *click(1340,192),*dump('flat'),*click(1340,254),*click(1268,330),{'key':'ctrl+a'},{'key':'7'},{'key':'return'},{'wait':700},*action(id,'auto-settle-off'),*dump('configured'),
  {'resume':id},{'wait':400},{'click_text':'Resume'},{'wait':400},{'send':'hold sidebar policy fixture'},{'wait':1000},*dump('running-flat'),
  *policy('working-section',1),*dump('running-grouped'),{'shortcut':'escape'},{'wait_idle':15000},{'wait':700},
  *policy('auto-settle-days',366),*dump('invalid'),*policy('auto-settle-days',7),*policy('working-section',0),{'quit':True}]))
 assert not read('flat')['projectFolders'],read('flat')
 configured=read('configured');assert not configured['groupProjects'] and not configured['workingSection'] and configured['autoSettleDays']==7,configured
 assert not row(configured,id)['autoSettleEnabled'],configured
 assert row(read('running-flat'),id)['status']=='Running' and row(read('running-flat'),id)['section']=='active',read('running-flat')
 assert row(read('running-grouped'),id)['section']=='working',read('running-grouped')
 assert read('invalid')['organizationError'] and read('invalid')['autoSettleDays']==7,read('invalid')
 before=sum(not r['summary'] for r in provider.requests)
 peak=max(peak,run('restart',[{'wait_ready':30000},{'wait':1200},*dump('restored'),*policy('group-projects',1),*action(id,'auto-settle-on'),*policy('auto-settle-days',0),*dump('disabled'),{'quit':True}]))
 restored=read('restored');assert not restored['groupProjects'] and not restored['workingSection'] and restored['autoSettleDays']==7,restored
 assert not row(restored,id)['autoSettleEnabled'],restored
 assert read('disabled')['autoSettleDays']==0 and read('disabled')['projectFolders'],read('disabled')
 assert row(read('disabled'),id)['autoSettleEnabled'],read('disabled')
 assert sum(not r['summary'] for r in provider.requests)==before,'Preference or restart dispatched provider work'
 assert not provider.errors,provider.errors
 receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,native_settings_pointer_and_keyboard=True,flat_and_grouped=True,working_preference=True,auto_settle_preference=True,per_chat_opt_out=True,invalid_preserved=True,restart=True)
 (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n');print('PASS: native sidebar preferences, working groups, auto-settle opt-out and restart',flush=True)
finally:provider.close();provider.write_evidence(out)

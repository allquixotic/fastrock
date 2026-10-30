#!/usr/bin/env python3
"""V68: native sidebar groups, batch actions, ordering, deadlines and restart."""
import argparse,hashlib,json,os,subprocess,time
from pathlib import Path
from embedded_provider_fixture import ProviderFixture,ProcessGuard
assert os.name=='nt','Run GUI acceptance on games only'
p=argparse.ArgumentParser(description=__doc__);p.add_argument('binary',type=Path);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
provider=ProviderFixture(stream_chunks=9000);env=provider.environment(out,streaming=True);env['FASTROCK_WEBHOOK_BIND']='127.0.0.1:0'
normal_response=provider.respond
failures=[]
def delayed_failure(handler,body):
 messages=body.get('messages',[])
 prompt=' '.join(c.get('text','') for c in messages[-1].get('content',[])) if messages else ''
 if prompt.strip().startswith('FailureWake'):
  provider.requests.append(dict(path=handler.path,body=body,summary=False));failures.append('requested')
  deadline=time.monotonic()+30
  while not (out/'failure-snoozed.json').exists():
   assert time.monotonic()<deadline,'Failure marker timeout'
   time.sleep(.05)
  handler.send_error(500,'Intentional local provider failure');return
 return normal_response(handler,body)
provider.respond=delayed_failure
repos=[out/'project',out/'second-project'];repos[1].mkdir()
git_env={k:v for k,v in os.environ.items() if not k.upper().startswith('GIT_')};git_env.update(GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL=str(out/'aws-empty'))
for repo in repos:
 (repo/'README.md').write_text('Organization fixture\n',encoding='utf-8')
 for args in (['init','.'],['add','.'],['-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','Fixture']):
  subprocess.run(['git','-c','core.hooksPath=NUL','-c','core.autocrlf=false',*args],cwd=repo,env=git_env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,check=True)
def dump(name):return [{'sidebar':['dump',str(out/(name+'.json'))]},{'wait':180}]
def read(name):return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def thread(id,action):return [{'sidebar':['thread',id,action]},{'wait':500}]
def org(action):return [{'sidebar':['organization',action]},{'wait':500}]
def run(name,steps):
 script=out/(name+'-steps.json');script.write_text(json.dumps(steps),encoding='utf-8')
 with (out/(name+'.log')).open('w',encoding='utf-8') as log:
  process=subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
  return ProcessGuard(process,timeout=150).wait()
def row(state,id):return next(r for r in state['rows'] if r['id']==id)
try:
 steps=[{'wait_ready':30000},{'resize':[1440,1100]}]
 for repo,text in [(repos[0],'First organized MossKeyword chat'),(repos[0],'Second organized FjordKeyword chat'),(repos[1],'Third organized CedarKeyword chat')]:
  steps += [{'open_folder':str(repo)},{'wait':900},{'wait_idle':30000},{'send':text},{'wait':300},{'wait_idle':30000},{'wait':700}]
 peak=run('create',[*steps,{'wait_summaries':30000},*dump('created'),{'quit':True}])
 state=read('created');assert state['organizationReady'] and not state['organizationError'],state
 ids=[c['id'] for c in state['navigation']['openChats']];assert len(ids)==3,state
 first,second,third=ids
 project_index=state['projectChoices'].index('project')
 peak=max(peak,run('organize',[{'wait_ready':30000},{'wait':1500},{'resize':[1440,1100]},
  *thread(first,'pin'),*thread(second,'settle'),*thread(third,'snooze-hour'),*dump('groups'),{'snapshot':str(out/'groups.png')},
  {'sidebar':['project',str(project_index)]},{'wait':300},*dump('project-filter'),{'sidebar':['project','0']},{'wait':300},
  {'resume':first},{'wait':300},{'click_text':'Resume'},{'wait':400},{'send':'hold organization fixture'},{'wait':900},*dump('pinned-working'),
  *thread(first,'unpin'),*dump('working'),{'shortcut':'escape'},{'wait_idle':15000},{'wait':600},*dump('active'),
  {'click_text':'Select'},{'wait':250},*thread(first,'select'),*thread(second,'select'),{'click_text':'Pin'},{'wait':600},*dump('batch'),
  *thread(second,'move-up'),*dump('ordered'),{'sidebar':['section','pinned']},{'wait':500},*dump('collapsed'),
  {'sidebar':['search','ObsidianNeedle']},{'wait':900},*dump('search-collapsed'),{'sidebar':['search','']},{'wait':700},
  {'sidebar':['section','pinned']},{'wait':500},*org('select'),
  {'sidebar':['folder',str(repos[0]),'rename-project']},{'wait':200},{'dialog_text':'Readable project'},{'dialog':True},{'wait':600},*dump('renamed'),
  *thread(third,'wake'),*thread(third,'snooze-hour'),{'quit':True}]))
 state=read('groups');assert [row(state,i)['section'] for i in ids]==['pinned','settled','snoozed'],state
 assert {r['id'] for r in read('project-filter')['rows']}=={first,second},read('project-filter')
 assert row(read('pinned-working'),first)['section']=='pinned'
 assert row(read('working'),first)['section']=='working',read('working')
 assert row(read('active'),first)['section']=='active',read('active')
 assert read('batch')['selectionCount']==2 and all(row(read('batch'),i)['pinned'] for i in [first,second]),read('batch')
 assert [r['id'] for r in read('ordered')['rows'] if r['section']=='pinned']==[second,first],read('ordered')
 assert read('collapsed')['selectionCount']==0 and not any(r['section']=='pinned' for r in read('collapsed')['rows']),read('collapsed')
 assert any(r['id']==first for r in read('search-collapsed')['rows']),read('search-collapsed')
 assert 'Readable project' in read('renamed')['projectChoices'],read('renamed')
 assert read('renamed')['navigation']['activeScope']=='Readable project',read('renamed')
 peak=max(peak,run('failure-wake',[{'wait_ready':30000},{'wait':1000},{'resume':first},{'wait':400},{'click_text':'Resume'},{'wait':500},
  *thread(first,'unpin'),{'send':'FailureWake local fixture'},{'wait':800},*thread(first,'snooze-hour'),*dump('failure-snoozed'),
  {'wait_idle':30000},{'wait':900},*dump('failure-woken'),*thread(first,'pin'),{'quit':True}]))
 assert 1 <= len(failures) <= 4,failures  # Unmodified AWS SDK may retry HTTP 500.
 assert row(read('failure-snoozed'),first)['section']=='snoozed',read('failure-snoozed')
 assert row(read('failure-woken'),first)['section']=='active',read('failure-woken')
 assert row(read('failure-woken'),first)['status']=='Error',read('failure-woken')
 # A near-future persisted deadline exercises the real single-shot native wake timer.
 metadata=out/'codex/fastrock-sidebar.json'
 if not metadata.exists():metadata=next(out.rglob('fastrock-sidebar.json'))
 saved=json.loads(metadata.read_text());saved['entries'][third]['snoozed_until']=int(time.time())+8
 metadata.write_text(json.dumps(saved),encoding='utf-8')
 before=sum(not r['summary'] for r in provider.requests)
 peak=max(peak,run('restart-wake',[{'wait_ready':30000},{'wait':700},*dump('restarted'),{'wait':9000},*dump('woken'),{'quit':True}]))
 assert row(read('restarted'),third)['section']=='snoozed',read('restarted')
 assert row(read('woken'),third)['section']=='active',read('woken')
 assert [r['id'] for r in read('restarted')['rows'] if r['section']=='pinned']==[second,first],read('restarted')
 assert 'Readable project' in read('restarted')['projectChoices']
 assert sum(not r['summary'] for r in provider.requests)==before,'Restart or wake dispatched agent'
 assert not provider.errors,provider.errors
 receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,native_groups=True,failure_wakes_snoozed_chat=True,batch_pin=True,project_filter=True,manual_order=True,collapse_and_search=True,project_name=True,restart_persistence=True,timed_wake_without_dispatch=True)
 (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf-8');print('PASS: native sidebar groups, filtering, batches, ordering, project names and timed wake',flush=True)
finally:provider.close();provider.write_evidence(out)

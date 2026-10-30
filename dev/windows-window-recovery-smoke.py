#!/usr/bin/env python3
"""V76: original native windows, isolated side drafts, Rally stores and restart selection."""
import argparse, hashlib, importlib.util, json, os, subprocess, threading, time
from pathlib import Path
from embedded_provider_fixture import ProviderFixture, ProcessGuard
assert os.name=='nt','Run native acceptance on games only'
p=argparse.ArgumentParser(description=__doc__);p.add_argument('binary',type=Path);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
spec=importlib.util.spec_from_file_location('rally_fixture',Path(__file__).with_name('windows-rally-smoke.py'));module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
rally=module.RallyFixture();server=threading.Thread(target=rally.server.serve_forever,daemon=True);server.start()
provider=ProviderFixture();env=provider.environment(out);env.update(FASTROCK_WEBHOOK_BIND='127.0.0.1:0',FASTROCK_RALLY_TOKEN=module.TOKEN)
(out/'home/rally.json').write_text(json.dumps(dict(rallyEndpoint=rally.base,rallyWorkspace=rally.workspace['_ref'],rallyProject=rally.project['_ref'],projectChildren=False)),encoding='utf-8')
def r(*args):return {'rally':list(args)}
def wait(rows=0,detail=None,tabs=None):return {'wait_rally':dict(rows=rows,detail=detail,tabs=tabs,timeout_ms=30000)}
def dump(name):return [r('dump',str(out/(name+'.json'))),{'wait':300}]
def draft(name):return [{'palette':['dump',str(out/(name+'.json'))]},{'wait':300}]
def complete():return [{'wait':400},{'wait_idle':30000}]
def read(name):return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def snapshot():return json.loads((out/'codex/fastrock-open-chats.json').read_text(encoding='utf-8'))
def run(name,steps,child_steps,crash=None):
 script=out/(name+'-steps.json');script.write_text(json.dumps(steps),encoding='utf-8')
 child=out/(name+'-child-steps.json');child.write_text(json.dumps(child_steps),encoding='utf-8')
 with (out/(name+'.log')).open('w',encoding='utf-8') as log:
  process=subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(script),FASTROCK_RALLY_CHILD_AUTOMATION=str(child)),stdout=log,stderr=log)
  guard=ProcessGuard(process,timeout=120)
  if crash is None:return guard.wait()
  try:
   while process.poll() is None and not crash.exists():guard.sample();time.sleep(.1)
   assert crash.exists(),'Child did not finish recovery before crash'
   time.sleep(.8);guard.sample();process.kill();process.wait(timeout=10)
  finally:
   if process.poll() is None:process.kill();process.wait(timeout=10)
  return guard.peak
try:
 peak=run('prepare',[
  {'wait_ready':30000},{'resize':[1300,900]},{'new_thread':str(out/'project')},{'wait_idle':30000},{'send':'main-window-question'},*complete(),
  {'composer':['type','Main window unsent draft']},r('open','userstories'),wait(128),r('item',rally.story['_ref']),wait(detail=True),r('edit','Name','Child Rally unsaved title'),
  {'wait':1800},{'click_text':'New window'},wait(tabs=0),{'select_tab':0},{'wait':8000},*draft('main-before'),{'quit':True}],
  [{'wait_ready':30000},wait(detail=True,tabs=1),{'resize':[1100,820]},*dump('child-before'),
   {'new_thread':str(out/'project')},{'wait_idle':30000},{'send':'child-window-question'},*complete(),
   {'tab_action':'side'},{'wait':600},{'wait_idle':30000},{'send':'side-window-question'},*complete(),
   {'composer':['type','Child side unsent draft é']},*draft('child-draft-before'),{'wait':120000}])
 saved=snapshot();assert len(saved['windows'])==2,saved
 main=saved['windows']['1'];child=next(w for owner,w in saved['windows'].items() if owner!='1')
 assert len(main['chats'])==1 and len(child['chats'])==2,(main,child)
 assert main['size']==[1300,900] and child['size']==[1100,820],(main['size'],child['size'])
 assert child['chats'][1]['side_parent'] and child['chats'][1]['draft']['text']=='Child side unsent draft é'
 registry=child['rally'];store=Path(registry['root'])/'windows'/registry['id']/'rally.json'
 documents=json.loads(store.read_text(encoding='utf-8'))['documents'];assert len(documents)==1
 identity=documents[0]['identity'];assert read('child-before')['dirty']
 before=len(provider.requests)
 peak=max(peak,run('restore',[
  {'wait_ready':30000},{'wait':1800},*draft('main-restored'),{'snapshot':str(out/'main-restored.png')},{'wait':6000},{'quit':True}],
  [{'wait_ready':30000},{'wait':1800},*draft('child-restored'),{'snapshot':str(out/'child-restored.png')},
   {'transcript':'export-complete:'+str(out/'child-restored.md')},{'wait':400},
   {'click_text':'Rally'},wait(detail=True,tabs=1),*dump('child-rally-restored'),{'wait':120000}]))
 assert read('main-restored')['draft']=='Main window unsent draft'
 assert read('child-restored')['draft']=='Child side unsent draft é'
 assert 'side-window-question' in (out/'child-restored.md').read_text(encoding='utf-8')
 assert read('child-rally-restored')['dirty'] and any(f['name']=='Name' and f['value']=='Child Rally unsaved title' for f in read('child-rally-restored')['fields'])
 assert not [q for q in provider.requests[before:] if not q['summary']],'Restart ran an agent'
 saved=snapshot();child=next(w for owner,w in saved['windows'].items() if owner!='1')
 assert child['rally']==registry and child['active_document']==identity,child
 assert not json.loads((out/'home/rally.json').read_text(encoding='utf-8'))['documents'],'Child document flattened into main'
 # Simulate a monitor that no longer exists before the next launch.
 child['position']=[-200000,-200000]
 (out/'codex/fastrock-open-chats.json').write_text(json.dumps(saved),encoding='utf-8')
 peak=max(peak,run('crash',[
  {'wait_ready':30000},{'wait':120000}],
  [{'wait_ready':30000},wait(detail=True,tabs=1),{'wait':1200},*dump('child-crash-ready'),{'wait':120000}],out/'child-crash-ready.json'))
 peak=max(peak,run('crash-restore',[
  {'wait_ready':30000},{'wait':1800},*draft('main-crash-restored'),{'wait':5000},{'quit':True}],
  [{'wait_ready':30000},wait(detail=True,tabs=1),{'wait':1200},*dump('child-after-crash'),{'snapshot':str(out/'child-after-crash.png')},{'wait':120000}]))
 assert read('main-crash-restored')['draft']=='Main window unsent draft'
 assert read('child-after-crash')['dirty'] and read('child-after-crash')['rallyTabs']==1
 assert all(w['position']!=[-200000,-200000] for w in snapshot()['windows'].values()),'Recovered window stranded offscreen'
 assert len(snapshot()['windows'])==2 and not json.loads((out/'home/rally.json').read_text(encoding='utf-8'))['documents']
 assert not [q for q in provider.requests[before:] if not q['summary']],'Crash restart ran an agent'
 assert all(q['method']=='GET' for q in rally.requests),'Window recovery wrote to Rally'
 assert not provider.errors,provider.errors
 receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,original_window_groups=True,logical_geometry=True,removed_monitor_fallback=True,side_and_main_drafts=True,active_chat_and_rally_document=True,child_registry_preserved=True,dirty_rally_draft=True,no_rally_writes=True,quit_and_crash_recovery=True,no_flattening=True,no_restart_dispatch=True)
 (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n');print('PASS: original windows, geometry, chat/Rally drafts, active selection and no replay',flush=True)
finally:
 provider.close();provider.write_evidence(out);rally.server.shutdown();rally.server.server_close();server.join(timeout=5)

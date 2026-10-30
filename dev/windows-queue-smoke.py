#!/usr/bin/env python3
"""V70: native queue ordering, explicit selected Resume, persistence and no replay."""
import argparse,hashlib,json,os,subprocess
from pathlib import Path
from embedded_provider_fixture import ProviderFixture,ProcessGuard
assert os.name=='nt','Run GUI acceptance on games only'
p=argparse.ArgumentParser(description=__doc__);p.add_argument('binary',type=Path);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
provider=ProviderFixture(stream_chunks=10000);env=provider.environment(out,streaming=True);env['FASTROCK_WEBHOOK_BIND']='127.0.0.1:0'
def dump(name):return [{'queue':['dump',str(out/(name+'.json'))]},{'wait':250}]
def read(name):return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def run(name,steps):
 script=out/(name+'-steps.json');script.write_text(json.dumps(steps),encoding='utf-8')
 with (out/(name+'.log')).open('w',encoding='utf-8') as log:
  process=subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
  return ProcessGuard(process,timeout=120).wait()
def action(index,verb):return [{'queue':['action',str(index),verb]},{'wait':700}]
def prompts():return [r['prompt'] for r in provider.requests if not r['summary']]
try:
 steps=[{'wait_ready':30000},{'resize':[1500,1150]},{'new_thread':str(out/'project')},{'wait_idle':30000},
  {'wait':600},{'send':'hold queue fixture'},{'wait':800}]
 for name in ['Queue first é','Queue second','Queue third']:
  steps += [{'composer':['type',name]},{'composer':['send']},{'wait':500}]
 steps += [*dump('queued'),*action(2,'up'),*action(1,'up'),*dump('reordered'),
  *action(1,'send'),*dump('busy-send'),{'shortcut':'escape'},{'wait_idle':30000},{'wait':500},*dump('paused'),{'quit':True}]
 peak=run('prepare',steps)
 assert [r['text'] for r in read('queued')['rows']]==['Queue first é','Queue second','Queue third'],read('queued')
 expected=['Queue third','Queue first é','Queue second']
 assert [r['text'] for r in read('reordered')['rows']]==expected,read('reordered')
 assert read('busy-send')['rows']==read('reordered')['rows'],read('busy-send')
 assert read('paused')['paused'] and read('paused')['canSend'],read('paused')
 assert prompts()==['hold queue fixture'],prompts()
 peak=max(peak,run('restart',[{'wait_ready':30000},{'wait':1200},*dump('restored'),
  {'snapshot':str(out/'restored.png')},{'wait':1000},*dump('selected'),
  # The native exact-label helper selects the last matching row: Queue second.
  {'click_text':'Resume & send'},{'wait':500},{'wait_idle':30000},{'wait':1500},*dump('drained'),
  {'click_text':'Review interrupted sends (1)'},{'wait':300},{'click_text':'Copy recovery records'},{'wait':200},
  {'selection':['capture',str(out/'uncertain.json')]},{'quit':True}]))
 assert read('restored')['paused'] and [r['text'] for r in read('restored')['rows']]==expected,read('restored')
 assert prompts()==['hold queue fixture','Queue second','Queue third','Queue first é'],prompts()
 assert not read('drained')['rows'] and not read('drained')['paused'],read('drained')
 uncertain=read('uncertain')
 assert len(uncertain)==1 and uncertain[0]['input'][0]['text']=='hold queue fixture',uncertain
 peak=max(peak,run('final-restart',[{'wait_ready':30000},{'wait':1500},*dump('final'),{'quit':True}]))
 assert prompts()==['hold queue fixture','Queue second','Queue third','Queue first é'],'Restart dispatched work'
 assert not read('final')['rows'],read('final')
 assert not provider.errors,provider.errors
 receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,native_reorder=True,busy_preserves_queue=True,restart_paused=True,explicit_selected_resume=True,exact_provider_order=prompts(),uncertain_never_replayed=True)
 (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n');print('PASS: native queue ordering, selected Resume, persistence and uncertain-send protection',flush=True)
finally:
 provider.close();provider.write_evidence(out)

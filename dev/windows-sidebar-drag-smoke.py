#!/usr/bin/env python3
"""V68: real native pointer drag, cancellation, grouping, order and restart."""
import argparse, hashlib, json, os, subprocess
from pathlib import Path
from embedded_provider_fixture import ProviderFixture, ProcessGuard
assert os.name == 'nt', 'Run GUI acceptance on games only'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args(); binary = a.binary.resolve(); out = a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
provider = ProviderFixture(stream_chunks=9000)
env = provider.environment(out, streaming=True); env['FASTROCK_WEBHOOK_BIND'] = '127.0.0.1:0'
repo = out / 'project'; (repo / 'README.md').write_text('Native drag fixture\n')
git_env = {k:v for k,v in os.environ.items() if not k.upper().startswith('GIT_')}
git_env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=str(out / 'aws-empty'))
for args in (['init','.'], ['add','.'], ['-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','Fixture']):
 subprocess.run(['git','-c','core.hooksPath=NUL','-c','core.autocrlf=false',*args], cwd=repo, env=git_env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)
def run(name, steps):
 script = out / (name + '-steps.json'); script.write_text(json.dumps(steps), encoding='utf-8')
 with (out / (name + '.log')).open('w', encoding='utf-8') as log:
  process = subprocess.Popen([str(binary)], env=dict(env, CODEX_GUI_AUTOMATION=str(script)), stdout=log, stderr=log)
  return ProcessGuard(process, timeout=150).wait()
def dump(name): return [{'sidebar':['dump',str(out / (name + '.json'))]}, {'wait':200}]
def read(name): return json.loads((out / (name + '.json')).read_text(encoding='utf-8'))
def action(id, verb): return [{'sidebar':['thread',id,verb]}, {'wait':600}]
def pointer(kind, target): return [{'sidebar':['pointer',kind,target]}, {'wait':200}]
def drag(source, over, name):
 # First move starts capture; the second uses updated geometry after drop targets appear.
 return [*pointer('press',source), *pointer('move',over), *pointer('move',over), *dump(name+'-during'), *pointer('release',over), {'wait':600}, *dump(name)]
def row(state,id): return next(r for r in state['rows'] if r['id']==id)
try:
 steps = [{'wait_ready':30000},{'resize':[1440,1100]}]
 for title in ['First drag chat','Second drag chat','Third drag chat']:
  steps += [{'open_folder':str(repo)},{'wait':900},{'wait_idle':30000},{'send':title},{'wait':300},{'wait_idle':30000},{'wait':700}]
 peak = run('create', [*steps,{'wait_summaries':30000},*dump('created'),{'quit':True}])
 first,second,third = [c['id'] for c in read('created')['navigation']['openChats']]
 before = sum(not r['summary'] for r in provider.requests)
 peak = max(peak,run('drag', [
  {'wait_ready':30000},{'wait':1200},{'resize':[1440,1100]},
  *drag(first,'drop:pinned','pin'), *drag(second,'drop:pinned','pin-second'),
  *drag(second,first,'order'), *drag(first,'drop:settled','settle'),
  *action(first,'snooze-hour'), *drag(first,'drop:active','wake'),
  *pointer('press',first),*pointer('move','drop:pinned'),*pointer('move','drop:pinned'),*dump('cancel-during'),
  {'key':'Escape'},{'wait':250},*pointer('release','drop:pinned'),*dump('cancelled'),
  *pointer('press',first),*pointer('move','drop:settled'),{'pointer':['move',700,500]}, {'wait':150},
  {'pointer':['release',700,500]},{'wait':500},*dump('outside'),{'quit':True}
 ]))
 for name in ['pin','pin-second','order','settle','wake']:
  assert read(name+'-during')['dragging'],(name,read(name+'-during'))
  assert not read(name)['dragging'],name
 assert row(read('pin'),first)['section']=='pinned',read('pin')
 assert [r['id'] for r in read('order')['rows'] if r['section']=='pinned']==[second,first],read('order')
 assert row(read('settle'),first)['section']=='settled' and not row(read('settle'),first)['pinned'],read('settle')
 assert row(read('wake'),first)['section']=='active',read('wake')
 assert read('cancel-during')['dragging'] and not read('cancelled')['dragging'],read('cancelled')
 assert row(read('cancelled'),first)['section']=='active',read('cancelled')
 assert row(read('outside'),first)['section']=='active',read('outside')
 assert sum(not r['summary'] for r in provider.requests)==before,'Drag dispatched a provider request'
 # Escape cancels a pointer drag while an independent foreground turn keeps running.
 peak=max(peak,run('running-cancel',[
  {'wait_ready':30000},{'wait':1000},{'resume':third},{'wait':400},{'click_text':'Resume'},{'wait':400},
  {'send':'hold drag cancellation fixture'},{'wait':800},*dump('running'),
  *pointer('press',first),*pointer('move','drop:pinned'),*pointer('move','drop:pinned'),
  {'key':'Escape'},{'wait':200},*pointer('release','drop:pinned'),*dump('running-cancelled'),
  {'shortcut':'escape'},{'wait_idle':15000},{'wait':600},*dump('stopped'),{'quit':True}]))
 assert row(read('running'),third)['status']=='Running',read('running')
 assert row(read('running-cancelled'),third)['status']=='Running',read('running-cancelled')
 assert row(read('running-cancelled'),first)['section']=='active',read('running-cancelled')
 before=sum(not r['summary'] for r in provider.requests)
 peak=max(peak,run('restart',[{'wait_ready':30000},{'wait':1300},*dump('restored'),{'quit':True}]))
 assert row(read('restored'),second)['section']=='pinned' and row(read('restored'),first)['section']=='active',read('restored')
 assert sum(not r['summary'] for r in provider.requests)==before,'Restart dispatched a provider request'
 assert not provider.errors,provider.errors
 receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,native_pointer_drag=True,group_drop=True,reorder=True,wake=True,escape_without_agent_interruption=True,outside_cancel=True,restart=True)
 (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n')
 print('PASS: native sidebar pointer drag, grouping, order, wake, Escape and restart',flush=True)
finally:
 provider.close();provider.write_evidence(out)

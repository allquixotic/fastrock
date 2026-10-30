#!/usr/bin/env python3
"""V68: pinned/snoozed metadata beyond page one never activates a Goose agent."""
import argparse,hashlib,json,os,subprocess
from pathlib import Path
from embedded_provider_fixture import ProviderFixture,ProcessGuard
assert os.name=='nt','Run GUI acceptance on games only'
p=argparse.ArgumentParser(description=__doc__);p.add_argument('binary',type=Path);p.add_argument('--seed-executable',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
provider=ProviderFixture();env=provider.environment(out);env['FASTROCK_WEBHOOK_BIND']='127.0.0.1:0'
def run(name,command,extra={}):
 with (out/(name+'.log')).open('w',encoding='utf-8') as log:
  proc=subprocess.Popen(command,env=dict(env,**extra),stdout=log,stderr=log)
  return ProcessGuard(proc,timeout=90).wait()
def app(name):
 script=out/(name+'-steps.json');script.write_text(json.dumps([{'wait_ready':30000},{'resize':[1280,820]},{'wait':2000},{'sidebar':['dump',str(out/(name+'.json'))]},{'wait':200},{'quit':True}]))
 return run(name,[str(binary)],dict(CODEX_GUI_AUTOMATION=str(script)))
try:
 parameters=out/'seed-parameters.json';parameters.write_text(json.dumps(dict(sessions=100,messages=2,project=str(out/'project'),vwork_home=str(out/'local/vwork'),output=str(out/'seed.json'))))
 peak=run('seed',[str(a.seed_executable.resolve()),'--seed-benchmark',str(parameters)])
 peak=max(peak,app('baseline'))
 before=json.loads((out/'baseline.json').read_text());visible={r['id'] for r in before['rows']};assert len(visible)<100, len(visible)
 seeded=json.loads((out/'seed.json').read_text())['sessions'];old=next('goose:'+s for s in seeded if 'goose:'+s not in visible)
 home=Path(env['FASTROCK_HOME']);metadata=home/'fastrock-sidebar.json'
 if not metadata.exists():metadata=Path(env['CODEX_HOME'])/'fastrock-sidebar.json'
 state=dict(version=1,revision=1,entries={old:dict(pinned=True)},order=[],collapsed=[],project_names={})
 metadata.write_text(json.dumps(state),encoding='utf-8')
 peak=max(peak,app('pinned'))
 pinned=json.loads((out/'pinned.json').read_text());assert any(r['id']==old and r['section']=='pinned' for r in pinned['rows']),pinned
 assert not [r for r in provider.requests if not r['summary']],'Metadata activated provider'
 assert not provider.errors,provider.errors
 receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,stored_conversations=100,first_page=len(visible),pinned_outside_first_page=old,provider_activation=False)
 (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n');print('PASS: old pinned chat visible without provider activation',flush=True)
finally:provider.close();provider.write_evidence(out)

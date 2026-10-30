#!/usr/bin/env python3
"""V69: cheap-profile title generation, stale-name guards and cancelable batches."""
import argparse,hashlib,json,os,subprocess,time
from pathlib import Path
from embedded_provider_fixture import ProviderFixture,ProcessGuard
assert os.name=='nt','Run GUI acceptance on games only'
p=argparse.ArgumentParser(description=__doc__);p.add_argument('binary',type=Path);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
provider=ProviderFixture();env=provider.environment(out);env['FASTROCK_WEBHOOK_BIND']='127.0.0.1:0'
normal=provider.respond;mode='normal';titles=[]
def respond(handler,body):
 system=' '.join(c.get('text','') for c in body.get('system',[]))
 if mode=='summary-hold' and 'search index sidecar' in json.dumps(body):
  deadline=time.monotonic()+25
  while not (out/'summary-release.json').exists():
   assert time.monotonic()<deadline,'Summary fixture marker timeout'
   time.sleep(.04)
 if 'Fastrock title generation.' not in system:return normal(handler,body)
 assert 'gpt-6-luna' in handler.path,handler.path
 assert not body.get('toolConfig'),body
 titles.append(dict(path=handler.path,body=body,mode=mode));number=len(titles)
 if mode in ['stale','cancel']:
  marker=out/('manual-saved.json' if mode=='stale' else 'cancel-requested.json');deadline=time.monotonic()+25
  while not marker.exists():
   assert time.monotonic()<deadline,'Title fixture marker timeout'
   time.sleep(.04)
 text='invalid\nmultiline' if mode=='invalid' else f'Generated fixture title {number}'
 return provider.json_response(handler,dict(output={'message':{'role':'assistant','content':[{'text':text}]}},stopReason='end_turn',usage={'inputTokens':3,'outputTokens':4,'totalTokens':7},metrics={'latencyMs':1}))
provider.respond=respond
repo=out/'project';(repo/'README.md').write_text('Title fixture\n')
git_env={k:v for k,v in os.environ.items() if not k.upper().startswith('GIT_')};git_env.update(GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL=str(out/'aws-empty'))
for args in (['init','.'],['add','.'],['-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','Fixture']):
 subprocess.run(['git','-c','core.hooksPath=NUL','-c','core.autocrlf=false',*args],cwd=repo,env=git_env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,check=True)
def dump(name):return [{'sidebar':['dump',str(out/(name+'.json'))]},{'wait':200}]
def read(name):return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def action(id,verb):return [{'sidebar':['thread',id,verb]},{'wait':600}]
def org(verb):return [{'sidebar':['organization',verb]},{'wait':600}]
def run(name,steps):
 script=out/(name+'-steps.json');script.write_text(json.dumps(steps),encoding='utf-8')
 with (out/(name+'.log')).open('w',encoding='utf-8') as log:
  process=subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
  return ProcessGuard(process,timeout=150).wait()
def row(state,id):return next(r for r in state['rows'] if r['id']==id)
try:
 steps=[{'wait_ready':30000},{'resize':[1440,1100]}]
 for text in ['First title chat','Second title chat','Third title chat']:
  steps += [{'open_folder':str(repo)},{'wait':900},{'wait_idle':30000},{'send':text},{'wait':300},{'wait_idle':30000},{'wait':500}]
 peak=run('create',[*steps,{'wait_summaries':30000},*dump('created'),{'quit':True}])
 ids=[c['id'] for c in read('created')['navigation']['openChats']];first=ids[0]
 foreground=sum(not r['summary'] for r in provider.requests)
 peak=max(peak,run('generate',[{'wait_ready':30000},{'wait':1200},*action(first,'regenerate-title'),{'wait':1000},*dump('single'),
  {'click_text':'Select'},{'wait':300},*org('select-all'),{'click_text':'Retitle'},{'wait':2000},*dump('batch'),{'quit':True}]))
 assert row(read('single'),first)['title']=='Generated fixture title 1',read('single')
 assert len(titles)==4,titles
 assert all(row(read('batch'),id)['title'].startswith('Generated fixture title ') for id in ids),read('batch')
 assert not read('batch')['regeneratingTitles'] and '0 failed' in read('batch')['titleGenerationStatus'],read('batch')
 assert sum(not r['summary'] for r in provider.requests)==foreground,'Title batch activated an agent'
 mode='summary-hold';before=len(titles)
 peak=max(peak,run('pending-summary',[{'wait_ready':30000},{'wait':1200},{'resume':first},{'wait':400},{'click_text':'Resume'},{'wait':400},
  {'send':'New context for the title'},{'wait':300},{'wait_idle':30000},{'wait':500},*dump('summary-turn-complete'),
  *action(first,'regenerate-title'),*dump('summary-release'),{'wait_summaries':30000},*dump('pending-summary-result'),{'quit':True}]))
 assert len(titles)==before,'Retitling called the model before its summary was current'
 assert row(read('pending-summary-result'),first)['title']==row(read('summary-turn-complete'),first)['title'],read('pending-summary-result')
 assert '1 failed' in read('pending-summary-result')['titleGenerationStatus'],read('pending-summary-result')
 foreground=sum(not r['summary'] for r in provider.requests)
 mode='stale'
 peak=max(peak,run('stale',[{'wait_ready':30000},{'wait':1200},*action(first,'regenerate-title'),*dump('generating'),
  *action(first,'rename'),{'dialog_text':'Human title wins'},{'dialog':True},{'wait':700},*dump('manual-saved'),{'wait':1700},*dump('stale-result'),{'quit':True}]))
 assert read('generating')['regeneratingTitles']==1,read('generating')
 assert row(read('manual-saved'),first)['title']=='Human title wins',read('manual-saved')
 assert row(read('stale-result'),first)['title']=='Human title wins',read('stale-result')
 assert '1 failed' in read('stale-result')['titleGenerationStatus'],read('stale-result')
 mode='invalid'
 peak=max(peak,run('invalid',[{'wait_ready':30000},{'wait':1200},*action(first,'regenerate-title'),{'wait':1000},*dump('invalid-result'),{'quit':True}]))
 assert row(read('invalid-result'),first)['title']=='Human title wins',read('invalid-result')
 assert '1 failed' in read('invalid-result')['titleGenerationStatus'],read('invalid-result')
 mode='cancel';before=len(titles)
 peak=max(peak,run('cancel',[{'wait_ready':30000},{'wait':1200},*org('select'),*org('select-all'),{'click_text':'Retitle'},{'wait':800},
  {'click_text':'Cancel remaining'},{'wait':250},*dump('cancel-requested'),{'wait':1500},*dump('cancel-result'),{'quit':True}]))
 assert len(titles)==before+1,titles
 assert '2 canceled' in read('cancel-result')['titleGenerationStatus'],read('cancel-result')
 mode='normal';before=len(titles)
 peak=max(peak,run('restart',[{'wait_ready':30000},{'wait':1200},*dump('restored'),{'quit':True}]))
 assert len(titles)==before,'Restart replayed a title request'
 assert {r['id']:r['title'] for r in read('restored')['rows']}=={r['id']:r['title'] for r in read('cancel-result')['rows']}
 assert sum(not r['summary'] for r in provider.requests)==foreground,'Title generation activated an agent'
 assert not provider.errors,provider.errors
 receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,cheap_model_only=True,no_agent_or_tools=True,single_and_native_batch=True,stale_manual_name_preserved=True,invalid_output_preserved=True,pending_summary_rejected=True,cancel_remaining=True,restart=True)
 (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n');print('PASS: cheap-model titles, native batch, stale/invalid preservation, cancellation and restart',flush=True)
finally:
 provider.close();provider.write_evidence(out);(out/'title-requests.json').write_text(json.dumps(titles,indent=2))

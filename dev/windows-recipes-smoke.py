#!/usr/bin/env python3
"""Native Goose recipe library, guarded editing, parameters and workspace execution."""
import argparse,hashlib,json,os,subprocess,sys,sqlite3,threading,time
from pathlib import Path
from embedded_provider_fixture import ProviderFixture,ProcessGuard
assert os.name=='nt','Run GUI acceptance on games only'
p=argparse.ArgumentParser(description=__doc__);p.add_argument('binary',type=Path);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
repo=out/'project';provider=ProviderFixture(workspace_repo=repo,tools=True);env=provider.environment(out)
env['FASTROCK_WEBHOOK_BIND']='127.0.0.1:0'
config=out/'goose/config/config.yaml';cfg=json.loads(config.read_text(encoding='utf-8'))
cfg['extensions']['source']=dict(enabled=True,name='source',type='stdio',cmd=str(out/'absent/vwork.exe'),args=['--home',str(out/'local/vwork'),'mcp'],timeout=30)
config.write_text(json.dumps(cfg),encoding='utf-8');(repo/'src').mkdir();(repo/'src/main.rs').write_text('recipe source remains intact\n',encoding='utf-8')
git_env={k:v for k,v in os.environ.items() if not k.upper().startswith('GIT_')};git_env.update(GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL=str(out/'aws-empty'))
for args in (['init','.'],['add','.'],['-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','Fixture']):
 subprocess.run(['git','-c','core.hooksPath=NUL','-c','core.autocrlf=false',*args],cwd=repo,env=git_env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,check=True)
recipe=dict(title='Native recipe fixture',description='Typed parameters and isolated workspace',instructions='RecipeUniqueFjord {{topic}} {{mode}} {{count}} {{source}}',prompt='{{task}}',extensions=[],settings=dict(goose_provider='aws_bedrock',goose_model='openai.gpt-6-luna'),parameters=[
 dict(key='topic',input_type='string',requirement='required',description='Subject'),
 dict(key='mode',input_type='select',requirement='required',description='Mode',options=['review','explain'],default='review'),
 dict(key='count',input_type='number',requirement='required',description='Count',default='2'),
 dict(key='source',input_type='file',requirement='required',description='Source file'),
 dict(key='task',input_type='string',requirement='required',description='Task',default='vwork-bound-parent')])
valid=json.dumps(recipe);bad=json.loads(valid);bad['settings']['goose_provider']='ollama'
def r(*args):return {'settings':['recipes',*args]}
def wait():return {'wait':800}
def dump(name):return [r('dump',str(out/(name+'.json'))),{'wait':180}]
def read(name):return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def values():return [r('parameter','topic','Aurora'),r('parameter','source',str(repo/'src/main.rs'))]
def run(name,steps):
 script=out/(name+'-steps.json');script.write_text(json.dumps(steps),encoding='utf-8')
 with (out/(name+'.log')).open('w',encoding='utf-8') as log:
  process=subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
  return ProcessGuard(process,timeout=150).wait()
try:
 peak=run('prepare',[{'wait_ready':30000},{'resize':[1440,1250]},{'open_folder':str(repo)},wait(),{'wait_idle':30000},
  {'open_settings':'recipes'},{'wait':2500},*dump('catalog'),{'click_text':'New recipe'},{'wait':250},r('content','invalid: true'),{'wait':300},*dump('new-editor'),{'snapshot':str(out/'new-editor.png')},{'click_text':'Save recipe'},wait(),*dump('invalid'),
  r('content',json.dumps(bad)),{'wait':180},{'click_text':'Save recipe'},wait(),wait(),*dump('saved'),*values(),r('prepare'),wait(),*dump('unsupported'),
  r('content',valid),r('save'),wait(),wait(),r('prepare'),wait(),*dump('missing'),*values(),r('parameter','count','NaN'),r('prepare'),wait(),*dump('number'),
  r('parameter','count','2'),{'snapshot':str(out/'recipes.png')},r('prepare'),wait(),wait(),*dump('prepared'),{'palette':['dump',str(out/'draft.json')]},{'wait':180},
  {'composer':['send']},{'wait':400},{'wait_idle':30000},wait(),{'transcript':'export-complete:'+str(out/'recipe.md')},wait(),
  {'open_settings':'capabilities'},wait(),{'settings':['capabilities','session','1']},wait(),{'settings':['capabilities','dump',str(out/'recipe-tools.json')]},wait(),{'quit':True}])
 assert read('invalid')['dirty'] and read('invalid')['status'],read('invalid')
 assert 'supports Bedrock' in read('unsupported')['status'],read('unsupported')
 assert 'Enter a value' in read('missing')['status'],read('missing')
 assert 'must be a number' in read('number')['status'],read('number')
 assert not read('prepared')['status'],read('prepared')
 assert read('draft')['draft']=='vwork-bound-parent',read('draft')
 ordinary=[q for q in provider.requests if not q['summary']]
 assert ordinary and all(q['path'].startswith('/model/openai.gpt-6-luna/') for q in ordinary),ordinary
 assert 'RecipeUniqueFjord Aurora review 2 recipe source remains intact' in json.dumps(ordinary[0]['body']),ordinary[0]
 assert provider.workspace_results and provider.workspace_results[0]['prompt']=='vwork-bound-parent',provider.workspace_results
 tools=read('recipe-tools');assert any(e['name']=='source' and e['protected'] for e in tools['extensions']),tools
 assert not any(e['name']=='fixture' for e in tools['extensions']),tools
 recipe_id=read('saved')['document']['id']
 before=sum(not q['summary'] for q in provider.requests)
 peak=max(peak,run('restart',[{'wait_ready':30000},wait(),{'open_settings':'recipes'},wait(),r('select',recipe_id),wait(),*dump('restart'),{'quit':True}]))
 assert sum(not q['summary'] for q in provider.requests)==before,'Restart ran recipe'
 assert read('restart')['document']['id']==recipe_id
 assert any(row['id']==recipe_id for row in read('restart')['catalog']['recipes'])
 recipe_path=next((out/'goose/config/recipes').glob('*.yaml'))
 edits=[]
 def external_edit():
  deadline=time.monotonic()+40
  while not (out/'stale-open.json').exists():
   if time.monotonic()>deadline:edits.append('marker timeout');return
   time.sleep(.05)
  recipe_path.write_text(recipe_path.read_text(encoding='utf-8')+'\n# external editor changed this file\n',encoding='utf-8');edits.append('edited')
 writer=threading.Thread(target=external_edit,daemon=True);writer.start()
 peak=max(peak,run('stale-delete',[{'wait_ready':30000},wait(),{'open_settings':'recipes'},wait(),r('select',recipe_id),wait(),wait(),*dump('stale-open'),{'wait':1100},
  r('content',valid),r('save'),wait(),*dump('stale-refused'),r('discard'),wait(),wait(),{'click_text':'Delete recipe'},{'wait':250},*dump('delete-confirm'),
  {'click_text':'Confirm deletion'},wait(),wait(),*dump('deleted'),{'quit':True}]))
 writer.join(timeout=1);assert edits==['edited'],edits
 assert read('stale-refused')['dirty'] and 'Recipe changed' in read('stale-refused')['status'],read('stale-refused')
 assert read('delete-confirm')['deleting'],read('delete-confirm')
 assert not recipe_path.exists() and not read('deleted')['catalog']['recipes'],read('deleted')
 assert any('external editor changed' in p.read_text(encoding='utf-8') for p in recipe_path.parent.glob('*.fastrock-backup-*'))
 assert sum(not q['summary'] for q in provider.requests)==before
 peak=max(peak,run('resume-deleted-recipe',[{'wait_ready':30000},wait(),{'resume':tools['threadId']},wait(),{'wait_idle':30000},
  {'click_text':'Resume'},{'wait':500},{'send':'vwork-child-restored'},{'wait':400},{'wait_idle':30000},wait(),
  {'transcript':'export-complete:'+str(out/'resumed.md')},wait(),{'quit':True}]))
 assert any(r['prompt']=='vwork-child-restored' for r in provider.workspace_results),provider.workspace_results
 assert all(q['path'].startswith('/model/openai.gpt-6-luna/') for q in provider.requests if not q['summary'])
 assert (repo/'src/main.rs').read_text(encoding='utf-8')=='recipe source remains intact\n'
 assert list((out/'goose/config/recipes').glob('*.fastrock-backup-*')),'No original recipe backup'
 assert not provider.errors,provider.errors
 receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,
  native_recipe_edit=True,stale_edit_preserved=True,guarded_delete_with_backup=True,typed_parameters=True,provider_scope_enforced=True,recipe_model_applied=True,
  prepared_prompt_retained=True,embedded_workspace_tool_executed=True,extension_replacement_retains_workspace=True,restart_no_recipe_run=True,resume_retains_recipe_and_workspace=True)
 (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf-8');print('PASS: native recipe editing, parameter validation, provider policy, isolated workspace and restart',flush=True)
finally:provider.close();provider.write_evidence(out)

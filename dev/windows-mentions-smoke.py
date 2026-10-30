#!/usr/bin/env python3
"""V71: native @ search reads compressed source, isolates projects and cancels stale queries."""
import argparse,hashlib,json,os,subprocess
from pathlib import Path
from embedded_provider_fixture import ProviderFixture,ProcessGuard
assert os.name=='nt','Run GUI acceptance on games only'
p=argparse.ArgumentParser(description=__doc__);p.add_argument('binary',type=Path);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
provider=ProviderFixture();env=provider.environment(out);env['FASTROCK_WEBHOOK_BIND']='127.0.0.1:0'
repo=out/'project';other=out/'other';other.mkdir()
git_env={k:v for k,v in os.environ.items() if not k.upper().startswith('GIT_')};git_env.update(GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL=str(out/'aws-empty'))
for root,files in [(repo,['src/main.rs','src/config.rs','docs/design notes.md','assets/stone.png','İmage/石.rs']),(other,['docs/guide.md'])]:
 for file in files:
  path=root/file;path.parent.mkdir(parents=True,exist_ok=True);path.write_text('Fixture source\n',encoding='utf-8')
 for args in (['init','.'],['add','.'],['-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','Fixture']):
  subprocess.run(['git','-c','core.hooksPath=NUL','-c','core.autocrlf=false',*args],cwd=root,env=git_env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,check=True)
def dump(name):return [{'composer':['dump',str(out/(name+'.json'))]},{'wait':200}]
def read(name):return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def query(text,name):return [{'composer':['type',text]},{'wait':800},*dump(name)]
def run(name,steps):
 script=out/(name+'-steps.json');script.write_text(json.dumps(steps),encoding='utf-8')
 with (out/(name+'.log')).open('w',encoding='utf-8') as log:
  process=subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
  return ProcessGuard(process,timeout=120).wait()
try:
 peak=run('mentions',[{'wait_ready':30000},{'resize':[1440,1000]},{'open_folder':str(repo)},{'wait':1000},{'wait_idle':30000},
  *query('@MAIN','exact'),{'snapshot':str(out/'mentions.png')},{'key':'return'},{'wait':300},*dump('accepted'),
  *query('@cfg','fuzzy'),*query('@src','directory'),{'key':'return'},{'wait':300},*dump('directory-accepted'),
  *query('@i\u0307mage/石','unicode'),*query('@stone','image-reference'),{'key':'return'},{'wait':300},*dump('image-accepted'),
  {'composer':['type','@main']},{'composer':['type','@no-such-file-123']},{'wait':800},*dump('latest'),
  {'composer':['type','@main']},{'composer':['type','Kept draft']},{'wait':800},*dump('closed'),
  {'open_folder':str(other)},{'wait':1000},{'wait_idle':30000},*query('@main','other-absent'),*query('@guide','other-present'),
  {'new_thread':str(other)},{'wait_idle':30000},*query('@guide','local'),
  {'select_tab':0},{'wait':400},*query('@MAIN','back'),{'composer':['type','Retain this draft']},{'quit':True}])
 assert read('exact')['files'][0]['path']=='src/main.rs',read('exact')
 assert 'src/main.rs' in read('accepted')['text'] and not read('accepted')['open'],read('accepted')
 assert any(r['path']=='src/config.rs' for r in read('fuzzy')['files']),read('fuzzy')
 assert read('directory')['files'][0]['path']=='src' and read('directory')['files'][0]['directory'],read('directory')
 assert 'src/' in read('directory-accepted')['text'],read('directory-accepted')
 assert read('unicode')['files'][0]['path']=='İmage/石.rs',read('unicode')
 assert read('unicode')['files'][0]['indices']==list(range(7)),read('unicode')
 assert 'assets/stone.png' in read('image-accepted')['text'],read('image-accepted')
 assert not read('latest')['files'] and not read('latest')['searching'],read('latest')
 assert not read('closed')['open'] and read('closed')['text']=='Kept draft',read('closed')
 assert not read('other-absent')['files'],read('other-absent')
 assert read('other-present')['files'][0]['path']=='docs/guide.md',read('other-present')
 assert read('local')['files'][0]['path']=='docs/guide.md',read('local')
 assert read('back')['files'][0]['path']=='src/main.rs',read('back')
 assert not provider.requests,'File search contacted a provider'
 peak=max(peak,run('restart',[{'wait_ready':30000},{'wait':1200},*dump('restored'),*query('@cfg','restart-search'),{'quit':True}]))
 assert read('restored')['text']=='Retain this draft',read('restored')
 assert any(r['path']=='src/config.rs' for r in read('restart-search')['files']),read('restart-search')
 assert not provider.requests and not provider.errors,(provider.requests,provider.errors)
 receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,compressed_source_mentions=True,case_insensitive_and_fuzzy=True,unicode_highlights=True,native_keyboard_completion=True,directories=True,compressed_image_reference=True,project_isolation=True,ordinary_local_search=True,stale_query_and_close=True,restart=True,no_provider_calls=True)
 (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n');print('PASS: compressed file mentions, native completion, scope, cancellation and restart',flush=True)
finally:
 provider.close();provider.write_evidence(out)

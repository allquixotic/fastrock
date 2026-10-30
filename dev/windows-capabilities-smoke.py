#!/usr/bin/env python3
"""Native Goose extension defaults, tools, in-process code mode and hooks."""
import argparse, hashlib, json, os, subprocess, sys
from pathlib import Path
from embedded_provider_fixture import ProviderFixture, ProcessGuard
assert os.name == 'nt', 'Run native acceptance on games only'
p=argparse.ArgumentParser(description=__doc__);p.add_argument('binary',type=Path);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
provider=ProviderFixture(tools=True);env=provider.environment(out,streaming=True);env['FASTROCK_WEBHOOK_BIND']='127.0.0.1:0'
config=out/'goose/config/config.yaml';value=json.loads(config.read_text(encoding='utf-8'));value['UNRELATED']='preserve'
value['extensions']['source']={'enabled':True,'name':'source','type':'stdio','cmd':str(out/'absent/vwork.exe'),'args':['--home',str(out/'local/vwork'),'mcp'],'timeout':30}
value['extensions']['fixture']['unknown_future']={'retain':True};config.write_text(json.dumps(value),encoding='utf-8')
secrets=config.with_name('secrets.yaml');secrets.write_text('UNUSED_KEY: preserve-fixture-only\n',encoding='utf-8');original_secrets=secrets.read_bytes()
plugin=out/'goose/.agents/plugins/fixture-hooks';(plugin/'hooks').mkdir(parents=True)
(plugin/'plugin.json').write_text(json.dumps(dict(name='fixture-hooks',version='1.0.0')),encoding='utf-8')
hook=plugin/'hooks/hooks.json';hook.write_text('{"hooks":{}}',encoding='utf-8');original_hooks=hook.read_bytes()
marker=out/'hook-invocations.jsonl';helper=out/'hook-fixture.py'
helper.write_text('import json,sys\nfrom pathlib import Path\nv=json.load(sys.stdin)\nwith Path(sys.argv[1]).open("a",encoding="utf-8") as f:f.write(json.dumps({"event":v.get("event"),"session":v.get("session_id")})+"\\n")\nprint("{}")\n',encoding='utf-8')
command=' '.join("'"+str(path).replace('\\','/').replace("'","'\"'\"'")+"'" for path in (Path(sys.executable),helper,marker))
new_hooks=json.dumps({'hooks':{'Stop':[{'hooks':[{'type':'command','command':command,'timeout':10}]}]}})
def c(*args):return {'settings':['capabilities',*args]}
def wait():return {'wait':800}
def dump(name):return [c('dump',str(out/(name+'.json'))),{'wait':180}]
def read(name):return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def run(name,steps):
 script=out/(name+'-steps.json');script.write_text(json.dumps(steps),encoding='utf-8')
 with (out/(name+'.log')).open('w',encoding='utf-8') as log:
  process=subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
  return ProcessGuard(process,timeout=180).wait()
def section(n):return [c('section',str(n)),{'wait':150}]
def extension(state,name):return next(e for e in state['catalog']['extensions'] if e['key']==name)
try:
 peak=run('controls',[{'wait_ready':30000},{'resize':[1440,1200]},{'new_thread':str(out/'project')},{'wait_idle':30000},wait(),
  {'open_settings':'capabilities'},wait(),wait(),*dump('initial'),
  *section(1),{'click_text':'Enable code mode'},wait(),*dump('code-on'),{'snapshot':str(out/'capabilities.png')},
  {'select_tab':0},{'send':'capability-code-mode'},{'wait_idle':30000},wait(),
  {'open_settings':'capabilities'},wait(),*section(1),{'click_text':'Disable code mode'},wait(),*dump('code-off'),
  {'select_tab':0},{'send':'hold capability mutation guard'},{'wait':500},{'open_settings':'capabilities'},wait(),
  {'click_text':'Enable code mode'},wait(),*dump('busy-refused'),{'select_tab':0},{'shortcut':'escape'},{'wait_idle':10000},
  {'open_settings':'capabilities'},wait(),
  c('select','fixture__selection_tool'),{'wait':180},{'click_text':'Never allow'},wait(),*dump('never'),
  {'click_text':'Ask before'},wait(),*dump('ask'),{'click_text':'Always allow'},wait(),*dump('allow'),
  *section(0),c('select','fixture'),{'wait':180},{'click_text':'Disable default'},wait(),*dump('disabled'),
  {'click_text':'Add MCP extension'},c('field','name','native_added'),c('field','transport','1'),c('field','address',provider.endpoint+'/mcp'),
  c('field','keys','UNUSED_KEY'),{'wait':180},{'click_text':'Save extension'},wait(),*dump('added'),
  *section(2),c('field','name','native-hooks'),{'click_text':'Create hook plugin'},wait(),*dump('created-hook-plugin'),c('select',str(plugin)),{'wait':180},{'click_text':'Edit hooks'},wait(),c('field','hooks','{}'),{'wait':180},{'click_text':'Save hooks'},wait(),*dump('invalid-hooks'),
  c('field','hooks',new_hooks),{'wait':180},{'click_text':'Save hooks'},wait(),*dump('saved-hooks'),
  {'new_thread':str(out/'project')},{'wait_idle':30000},wait(),{'send':'Check native hook invocation'},{'wait_idle':30000},wait(),
  {'open_settings':'capabilities'},wait(),c('session','1'),wait(),*section(1),*dump('new-chat'),{'select_tab':2},{'shortcut':'close-tab'},wait(),{'open_settings':'capabilities'},wait(),wait(),*dump('closed-selection'),{'quit':True}])
 assert not read('initial')['busy'],read('initial')
 assert extension(read('initial'),'source')['protected']
 assert next(e for e in read('initial')['extensions'] if e['name']=='source')['protected']
 workspace_tools=[t for t in read('initial')['tools'] if t['name'].startswith('source__')]
 assert workspace_tools and all(t['protected'] for t in workspace_tools),read('initial')
 assert read('code-on')['codeMode'] and not read('code-off')['codeMode']
 assert not read('busy-refused')['codeMode'] and 'Wait for this conversation' in read('busy-refused')['status'],read('busy-refused')
 assert any(c['name']=='selection_tool' and c['arguments'].get('command')=='code-mode-fixture' for c in provider.tool_calls),provider.tool_calls
 for name,expected in [('never','never_allow'),('ask','ask_before'),('allow','always_allow')]:
  assert next(t['permission'] for t in read(name)['tools'] if t['name']=='fixture__selection_tool')==expected,read(name)
 assert not extension(read('disabled'),'fixture')['enabled']
 assert not extension(read('added'),'native_added')['enabled']
 assert read('invalid-hooks')['dirty'] and 'hooks object' in read('invalid-hooks')['status'],read('invalid-hooks')
 assert any(h['key'].endswith('native-hooks') and not h['enabled'] for h in read('created-hook-plugin')['catalog']['hooks']),read('created-hook-plugin')
 assert not read('saved-hooks')['dirty'],read('saved-hooks')
 assert hook.read_text(encoding='utf-8')==new_hooks
 assert any(p.read_bytes()==original_hooks for p in hook.parent.glob('hooks.fastrock-backup-*.json'))
 assert marker.exists() and marker.stat().st_size>0,'Configured Goose Stop hook did not execute'
 invocations=[json.loads(line) for line in marker.read_text(encoding='utf-8').splitlines()]
 assert any(i['event']=='Stop' and i['session']==read('new-chat')['threadId'].removeprefix('goose:') for i in invocations),invocations
 assert not any(t['name'].startswith('fixture__') for t in read('new-chat')['tools']),read('new-chat')
 assert len(read('closed-selection')['sessions'])==1 and not read('closed-selection')['busy'],read('closed-selection')
 assert secrets.read_bytes()==original_secrets
 # A new process reads the saved defaults and installed hooks; it must not run
 # foreground conversations until the fixture explicitly creates one. Summary
 # backfill is independently authorized background work.
 before=sum(not r['summary'] for r in provider.requests)
 peak=max(peak,run('restart',[{'wait_ready':30000},wait(),{'open_settings':'capabilities'},wait(),*dump('restart'),{'quit':True}]))
 assert sum(not r['summary'] for r in provider.requests)==before,'Restart replayed foreground provider work'
 assert not extension(read('restart'),'fixture')['enabled']
 text=config.read_text(encoding='utf-8');assert 'unknown_future' in text and 'UNRELATED' in text and 'preserve' in text
 assert not provider.errors,provider.errors
 receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,
  native_extensions=True,custom_workspace_alias_protected=True,code_mode_tool_executed=True,tool_permissions_persisted=True,idle_mutation_guard=True,
  defaults_only_affect_new_chats=True,hook_validation_backup_execution=True,credentials_preserved=True,restart_no_replay=True)
 (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf-8')
 print('PASS: native Goose extension/tool/code-mode/hook controls',flush=True)
finally:
 provider.close();provider.write_evidence(out)

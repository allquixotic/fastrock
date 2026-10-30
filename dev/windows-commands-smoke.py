#!/usr/bin/env python3
"""V80: visible slash controls use embedded Goose, isolated source and local diff."""
import argparse, base64, hashlib, json, os, subprocess
from pathlib import Path
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert os.name == 'nt', 'Run GUI acceptance on games only'
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('binary',type=Path)
p.add_argument('--output',type=Path,required=True)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
repo=out/'project';provider=ProviderFixture(workspace_repo=repo);env=provider.environment(out);env['FASTROCK_WEBHOOK_BIND']='127.0.0.1:0'
config=out/'goose/config/config.yaml';value=json.loads(config.read_text(encoding='utf-8'))
value['extensions']={'vwork':dict(enabled=True,name='vwork',type='stdio',cmd=str(out/'absent/vwork.exe'),args=['--home',str(out/'local/vwork'),'mcp'],timeout=30)}
config.write_text(json.dumps(value),encoding='utf-8')
(repo/'src').mkdir();source=repo/'src/main.rs';source.write_text('original source\n',encoding='utf-8')
git_env={k:v for k,v in os.environ.items() if not k.upper().startswith('GIT_')};git_env.update(GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL=str(out/'aws-empty'))
for args in (['init','-b','main','.'],['add','.'],['-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','Root']):
    subprocess.run(['git','-c','core.hooksPath=NUL','-c','core.autocrlf=false',*args],cwd=repo,env=git_env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,check=True)
def complete():return [{'wait':400},{'wait_idle':30000}]
def command(text):return [{'composer':['type',text]},{'composer':['send']},{'wait':650}]
def capture(name):return [{'selection':['capture',str(out/(name+'.txt'))]},{'wait':250}]
def dump(name):return [{'palette':['dump',str(out/(name+'.json'))]},{'wait':250}]
def run(steps):
    script=out/'steps.json';script.write_text(json.dumps(steps),encoding='utf-8')
    with (out/'app.log').open('w',encoding='utf-8') as log:
        process=subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
        return ProcessGuard(process,timeout=120).wait()
try:
    peak=run([
        {'wait_ready':30000},{'resize':[1440,1000]},{'open_folder':str(repo)},{'wait':1000},{'wait_idle':30000},
        *command('/model'),{'snapshot':str(out/'model-options.png')},
        *command('/plan preserve this unsupported request'),*dump('unsupported'),{'composer':['type','']},
        *command('/mcp'),{'settings':['capabilities','dump',str(out/'capabilities.json')]},{'wait':250},
        {'open_settings':'diagnostics'},{'wait':400},{'snapshot':str(out/'diagnostics.png')},
        {'click_text':'Open provider settings'},{'wait':300},{'select_tab':0},
        {'send':'vwork-child-edit'},*complete(),{'click_text':'Changes'},{'wait':800},{'snapshot':str(out/'changes.png')},
        {'click_text':'Copy diff'},{'wait':250},*capture('diff'),{'shortcut':'close-tab'},{'wait':300},{'select_tab':0},{'wait':250},
        *command('/new Command-created chat'),{'wait_idle':30000},{'wait':700},{'sidebar':['dump',str(out/'new.json')]},{'wait':250},
        *command('/ps'),{'terminal':['dump',str(out/'terminals.json')]},{'wait':250},{'quit':True},
    ])
    read=lambda name:json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
    assert read('unsupported')['draft']=='/plan preserve this unsupported request'
    assert 'catalog' in read('capabilities') and not read('capabilities')['busy'],read('capabilities')
    diff=(out/'diff.txt').read_text(encoding='utf-8');assert '-original source' in diff and '+fork isolated λ' in diff,diff
    state=read('new');assert state['navigation']['activeScope']=='project' and state['navigation']['activeTitle']=='Command-created chat',state
    bindings=[json.loads(p.read_text(encoding='utf-8')) for p in out.rglob('*.binding.json')]
    workspaces=[v['workspace']['selection']['workspace'] for v in bindings if v.get('workspace')]
    assert len(workspaces)==2 and len(set(workspaces))==2,bindings
    assert read('terminals')['terminals']==0,'/ps started a shell'
    assert source.read_text(encoding='utf-8')=='original source\n'
    assert all(r['summary'] or r['prompt']=='vwork-child-edit' for r in provider.requests),'A local command dispatched a model request'
    assert not provider.errors,provider.errors
    receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,goose_options_and_capabilities=True,unsupported_draft_preserved=True,native_diff=True,new_chat_isolated=True,terminal_list_no_spawn=True,no_unintended_model_calls=True)
    (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf-8');print('PASS: Goose slash controls, local diff and isolated new chat',flush=True)
finally:
    provider.close();provider.write_evidence(out)

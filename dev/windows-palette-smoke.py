#!/usr/bin/env python3
"""Native palette keyboard/pointer navigation, modal Escape, and draft retention."""
import argparse, hashlib, json, os, subprocess
from pathlib import Path
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert os.name == 'nt', 'Execute GUI acceptance on games only'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
binary, out = a.binary.resolve(), a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
provider = ProviderFixture(stream_chunks=9000)
env = provider.environment(out, streaming=True)
env['FASTROCK_WEBHOOK_BIND'] = '127.0.0.1:0'
repo = out/'project'
(repo/'README.md').write_text('Palette fixture repository\n',encoding='utf-8')
git_env = {k:v for k,v in os.environ.items() if not k.upper().startswith('GIT_')}
git_env.update(GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL=str(out/'aws-empty'))
for args in (['init','.'],['add','README.md'],['-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','Fixture']):
    subprocess.run(['git','-c','core.hooksPath=NUL','-c','core.autocrlf=false',*args],cwd=repo,env=git_env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,check=True)

def dump(name): return [{'palette':['dump',str(out/(name+'.json'))]}, {'wait':150}]
def search(query): return [{'palette':['query',query]}, {'wait':150}]
def open_palette(): return [{'key':'ctrl+shift+P'}, {'wait':200}]
def read(name): return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))

steps = [{'wait_ready':30000}, {'resize':[1440,1000]}, {'open_folder':str(repo)}, {'wait':1000},
         {'wait_idle':30000}, {'key':'x'}, {'wait':100}, *dump('initial-focus'),
         {'composer':['type','']}, {'send':'hold palette fixture'}, {'wait':900},
         {'composer':['type','Unsent palette draft']}, *open_palette(),
         {'key':'p'}, {'key':'r'}, {'key':'v'}, {'wait':200}, *dump('fuzzy'),
         {'snapshot':str(out/'palette.png')}, {'key':'escape'}, {'wait':200}, *dump('closed'),
         *open_palette(), *open_palette(), *dump('toggle-closed'),
         *open_palette(), *search('>'), {'key':'down'}, {'wait':100}, *dump('navigation'),
         *search('provider profiles'), {'key':'return'}, {'wait':500}, *dump('provider'),
         {'select_tab':0}, {'wait':300}, *dump('restored'),
         *open_palette(), *search('open rally'), {'click_text':'Open Rally'}, {'wait':400},
         *open_palette(), *search('hold palette fixture'), *dump('chats'),
         {'key':'return'}, {'wait':400}, *dump('chat-opened'),
         *open_palette(), *search('no such palette item 12345'), {'key':'return'}, *dump('empty'),
         {'key':'escape'}, {'wait':200}, {'shortcut':'escape'}, {'wait_idle':10000},
         {'transcript':'export-complete:'+str(out/'conversation.md')}, {'wait':200}, *open_palette(), *search(str(repo)), *dump('projects'),
         {'key':'return'}, {'wait':1000}, {'wait_idle':30000}, *dump('project-opened'), {'quit':True}]
script = out/'steps.json'
script.write_text(json.dumps(steps), encoding='utf-8')
try:
    with (out/'app.log').open('w', encoding='utf-8') as log:
        proc = subprocess.Popen([str(binary)], env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
        peak = ProcessGuard(proc, timeout=90).wait()
    assert read('initial-focus')['draft']=='x',read('initial-focus')
    assert read('fuzzy')['open'] and read('fuzzy')['rows'][0]['title']=='Provider profiles',read('fuzzy')
    assert not read('toggle-closed')['open'],read('toggle-closed')
    assert not read('closed')['open'] and read('closed')['busy'],read('closed')
    assert read('closed')['draft']=='Unsent palette draft',read('closed')
    assert read('navigation')['selected']==1 and all(r['kind']=='Command' for r in read('navigation')['rows']),read('navigation')
    assert not read('provider')['open'],read('provider')
    assert read('restored')['draft']=='Unsent palette draft' and read('restored')['busy'],read('restored')
    assert read('chats')['rows'][0]['kind']=='Conversation',read('chats')
    assert not read('chat-opened')['open'] and read('chat-opened')['busy'],read('chat-opened')
    assert read('empty')['open'] and not read('empty')['rows'],read('empty')
    transcript=(out/'conversation.md').read_text(encoding='utf-8')
    assert 'Holding hold palette fixture' in transcript and 'Unsent palette draft' not in transcript
    assert read('projects')['rows'][0]['kind']=='Project',read('projects')
    assert not read('project-opened')['open'],read('project-opened')
    assert (repo/'README.md').read_text(encoding='utf-8')=='Palette fixture repository\n'
    assert sum(not r['summary'] for r in provider.requests)==1,provider.requests
    receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,
                 native_keyboard_and_pointer=True,escape_preserves_active_stream=True,draft_retained=True,
                 command_filter_and_fuzzy=True,conversation_navigation=True,project_navigation=True)
    (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf-8')
    print('PASS: native command palette keyboard/pointer, modal Escape and retained draft',flush=True)
finally:
    provider.close()
    provider.write_evidence(out)

#!/usr/bin/env python3
"""V79: native project defaults, branch chats, invalid-ref retention and restart."""
import argparse, hashlib, json, os, subprocess
from pathlib import Path
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert os.name == 'nt', 'Run GUI acceptance on games only'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
binary, out = a.binary.resolve(), a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
provider = ProviderFixture()
env = provider.environment(out)
env['FASTROCK_WEBHOOK_BIND'] = '127.0.0.1:0'
repo = out / 'project'
source = repo / 'README.md'
source.write_text('main source\n', encoding='utf-8')
git_env = {k:v for k,v in os.environ.items() if not k.upper().startswith('GIT_')}
git_env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=str(out/'aws-empty'))
def git(*args):
    return subprocess.check_output(['git', '-c', 'core.hooksPath=NUL', '-c', 'core.autocrlf=false', *args], cwd=repo, env=git_env, stderr=subprocess.PIPE, text=True).strip()
git('init', '-b', 'main', '.')
git('add', '.')
git('-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','Main')
git('checkout','-b','feature')
source.write_text('feature branch λ\n', encoding='utf-8')
git('add','.')
git('-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','Feature')
git('checkout','main')
before = source.read_bytes()
def dump(name):
    return [{'project':['dump',str(out/(name+'.json'))]}, {'wait':250}]
def read(name):
    return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def run(name, steps):
    script = out/(name+'-steps.json')
    script.write_text(json.dumps(steps), encoding='utf-8')
    with (out/(name+'.log')).open('w',encoding='utf-8') as log:
        process = subprocess.Popen([str(binary)], env=dict(env,CODEX_GUI_AUTOMATION=str(script)), stdout=log, stderr=log)
        return ProcessGuard(process, timeout=120).wait()
try:
    peak = run('settings', [
        {'wait_ready':30000}, {'resize':[1440,1000]}, {'project':['open',str(repo)]}, {'wait':1000}, *dump('loaded'),
        {'project':['reference','missing-branch']}, {'click_text':'Save default'}, {'wait':500}, *dump('invalid'),
        {'project':['reference','feature']}, {'click_text':'Save default'}, {'wait':700}, *dump('saved'),
        {'snapshot':str(out/'project-settings.png')}, {'click_text':'New chat'}, {'wait':1000}, {'wait_idle':30000},
        {'composer':['type','Keep project draft é']},
        {'key':'ctrl+shift+P'}, {'wait':200}, {'palette':['query','Project settings']}, {'wait':200}, {'key':'return'}, {'wait':800}, *dump('palette'),
        {'key':'escape'}, {'wait':200}, *dump('closed'), {'palette':['dump',str(out/'draft.json')]}, {'wait':200}, {'quit':True},
    ])
    assert read('loaded')['ready'] and read('loaded')['reference']=='HEAD', read('loaded')
    assert read('loaded')['branches']==['HEAD','feature','main'], read('loaded')
    assert read('invalid')['error'] and read('invalid')['reference']=='missing-branch', read('invalid')
    assert read('saved')['revision']==1 and read('saved')['reference']=='feature' and not read('saved')['error'], read('saved')
    assert Path(read('palette')['path']).samefile(repo) and read('palette')['reference']=='feature', read('palette')
    assert read('draft')['draft']=='Keep project draft é', read('draft')
    assert not read('closed')['open'], 'Escape did not close the project editor'
    peak = max(peak, run('restart', [
        {'wait_ready':30000}, {'wait':1200}, {'project':['open',str(repo)]}, {'wait':600}, *dump('restored'),
        {'project':['reference','main']}, {'click_text':'New chat'}, {'wait':900}, {'wait_idle':30000},
        {'project':['open',str(repo)]}, {'wait':600}, *dump('after-override'), {'click_text':'Close'}, {'quit':True},
    ]))
    assert read('restored')['reference']=='feature' and read('restored')['revision']==1
    assert read('after-override')['reference']=='feature', 'One-off branch changed the saved default'
    assert source.read_bytes()==before
    assert not provider.requests, 'Project UI dispatched a provider turn'
    assert not provider.errors, provider.errors
    receipt = dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,native_project_editor=True,invalid_reference_preserves_input=True,saved_default_and_override=True,restart_persistence=True,draft_preserved=True,source_unchanged=True,no_provider_dispatch=True)
    (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf-8')
    print('PASS: native project defaults, branches, draft and restart',flush=True)
finally:
    provider.close()
    provider.write_evidence(out)

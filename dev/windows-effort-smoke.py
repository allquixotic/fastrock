#!/usr/bin/env python3
"""Saved Goose effort defaults, actual native controls and local Bedrock requests."""
import argparse, hashlib, json, os, subprocess
from pathlib import Path
from urllib.parse import unquote
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert os.name == 'nt', 'Execute native acceptance on Windows only'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
binary, out = a.binary.resolve(), a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
provider = ProviderFixture()
env = provider.environment(out)
env['FASTROCK_WEBHOOK_BIND'] = '127.0.0.1:0'
OPUS = 'us.anthropic.claude-opus-5-5-v1:0'
SONNET = 'us.anthropic.claude-sonnet-4-5-20250929-v1:0'
config = out/'goose/config/config.yaml'
value = json.loads(config.read_text(encoding='utf-8'))
value.update(GOOSE_MODEL=OPUS, CUSTOM_SETTING='preserved')
config.write_text(json.dumps(value), encoding='utf-8')
secrets = config.with_name('secrets.yaml')
secrets.write_text('ANTHROPIC_API_KEY: unused-preserve\n', encoding='utf-8')
original_secrets = secrets.read_bytes()

def control(*args): return {'settings':['goose-setup',*args]}
def settled(): return [{'wait_goose':10000}]
def choose(name, value): return [control('choose', name, value), *settled()]
def dump(name): return [*settled(), control('dump',str(out/(name+'.json'))), {'wait':150}]
def read(name): return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def effort(name):
    state = read(name)
    assert not state['sessionChanging'], state
    return next(o['current'] for o in state['sessionOptions'] if o['id']=='thinking_effort')
def new(): return [{'new_thread':str(out/'project')}, {'wait_idle':30000}, *settled()]
def run(name, steps):
    script = out/(name+'-steps.json')
    script.write_text(json.dumps(steps),encoding='utf-8')
    with (out/(name+'.log')).open('w',encoding='utf-8') as log:
        process = subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
        return ProcessGuard(process,timeout=120).wait()

try:
    peak = run('save', [{'wait_ready':30000}, {'resize':[1440,1100]}, *new(),
        {'sidebar':['dump',str(out/'first-tab.json')]}, *dump('initial'),
        {'click_text':f'Goose · {OPUS}  ▾'}, {'wait':200},
        *choose('thinking_effort','high'), {'click_text':'Save effort default'}, {'wait':100}, *dump('saved'),
        {'snapshot':str(out/'effort-default.png')},
        *choose('thinking_effort','low'), {'send':'Keep explicit low effort in this conversation'}, {'wait_idle':30000}, {'wait':500},
        *dump('override'), *new(), *dump('new-default'),
        {'send':'Verify saved high default in a new conversation'}, {'wait_idle':30000}, {'wait':400},
        *choose('model',SONNET), *choose('thinking_effort','medium'), control('save-effort'), *dump('second-model'),
        control('custom-model',OPUS), {'click_text':'Use model ID'}, {'wait':100}, *dump('switched-back'),
        *choose('thinking_effort','not-offered'), *dump('unsupported'),
        {'select_tab':0}, {'wait':400}, *dump('original-unchanged'), {'quit':True}])
    assert effort('saved')=='high' and 'saved for this provider/model' in read('saved')['effortStatus'],read('saved')
    assert effort('override')=='low'
    assert effort('new-default')=='high',read('new-default')
    assert effort('second-model')=='medium'
    assert effort('switched-back')=='high',read('switched-back')
    assert 'not offered' in read('unsupported')['sessionError'] and effort('unsupported')=='high',read('unsupported')
    assert effort('original-unchanged')=='low',read('original-unchanged')
    original_id = read('first-tab')['navigation']['openChats'][0]['id']
    peak = max(peak, run('restart', [{'wait_ready':30000}, {'wait':1000}, {'resume':original_id},
        {'wait_idle':30000}, {'wait':400}, *dump('restored'), *new(), *dump('restart-default'),
        control('clear-effort'), *dump('cleared'), *new(), *dump('after-clear'),
        *choose('model',SONNET), *dump('other-default-retained'), {'quit':True}]))
    assert effort('restored')=='low',read('restored')
    assert effort('restart-default')=='high',read('restart-default')
    assert 'cleared' in read('cleared')['effortStatus'],read('cleared')
    assert effort('after-clear')==effort('initial'),read('after-clear')
    assert effort('other-default-retained')=='medium',read('other-default-retained')
    assert 'preserved' in config.read_text(encoding='utf-8')
    assert secrets.read_bytes()==original_secrets
    ordinary = [r for r in provider.requests if not r['summary']]
    assert len(ordinary)==2 and all(f'/model/{OPUS}/converse'==unquote(r['path']) for r in ordinary),ordinary
    assert [r['body']['additionalModelRequestFields']['output_config']['effort'] for r in ordinary]==['low','high'],ordinary
    assert not provider.errors,provider.errors
    receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,
                 native_save_control=True,per_model_defaults=True,new_session_and_switch=True,
                 restored_effort_preserved=True,unsupported_choice_rejected=True,clear_is_scoped=True,
                 original_credentials_preserved=True)
    (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf-8')
    print('PASS: saved Goose effort defaults, model switching, restart and scoped clear',flush=True)
finally:
    provider.close()
    provider.write_evidence(out)

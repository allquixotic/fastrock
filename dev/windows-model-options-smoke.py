#!/usr/bin/env python3
"""Configured-only providers, native compact controls and persisted speed choices.
AWS profiles are configuration fixtures only: HTTPS proxy blocks external calls.
Actual Responses wire payloads are verified separately by embedded_goose.
"""
import argparse, hashlib, json, os, subprocess, sys
from pathlib import Path
from embedded_provider_fixture import ProviderFixture, ProcessGuard
assert os.name == 'nt', 'Run native verification on Windows only'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
binary, out = a.binary.resolve(), a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
fixture = ProviderFixture()
env = fixture.environment(out)
env.update(FASTROCK_WEBHOOK_BIND='127.0.0.1:0', HTTPS_PROXY=fixture.endpoint, HTTP_PROXY=fixture.endpoint,
           https_proxy=fixture.endpoint, http_proxy=fixture.endpoint, FASTROCK_V89_KEY='fixture', FASTROCK_V89_BLANK='   ')
SOL, LUNA = 'us.openai.gpt-6.1-sol', 'us.openai.gpt-6-luna'
custom = out/'goose/config/custom_providers'
custom.mkdir()
marker = out/'helper-must-not-run.txt'
helper = out/'credential.py'
helper.write_text(f'from pathlib import Path\nPath({str(marker)!r}).write_text("unexpected")\nprint("fixture")\n')

def profile(name, **updates):
    value = dict(name=name,display_name=name.removeprefix('custom_').replace('_',' ').title(),engine='openai',
        base_url='https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1',base_path='openai/v1/responses',
        requires_auth=True,api_key_env='FASTROCK_V89_KEY',dynamic_models=False,supports_streaming=False,
        models=[dict(name=SOL,reasoning=True),dict(name=LUNA,reasoning=True)])
    value.update(updates)
    (custom/(name+'.json')).write_text(json.dumps(value),encoding='utf-8')
profile('custom_bedrock_sol')
profile('custom_helper',api_key_env='',auth=dict(command=sys.executable,args=[str(helper)]))
profile('custom_blank',api_key_env='FASTROCK_V89_BLANK')
profile('custom_no_model',models=[])
profile('custom_no_endpoint',base_url='')
profile('custom_missing_helper',api_key_env='',auth=dict(command=str(out/'does-not-exist.exe')))
profile('custom_disabled')
config = out/'goose/config/config.yaml'
settings = json.loads(config.read_text(encoding='utf-8'))
settings['providers'] = {'custom_bedrock_sol':dict(enabled=True,configured=True,model=SOL),
                         'custom_disabled':dict(enabled=False,configured=True,model=SOL)}
config.write_text(json.dumps(settings),encoding='utf-8')
original = {p:p.read_bytes() for p in custom.glob('*.json')}

def ctl(*args): return {'settings':['goose-setup',*args]}
def settled(): return [{'wait_goose':15000}]
def choose(key,value): return [ctl('choose',key,value),*settled()]
def dump(name): return [*settled(),ctl('dump',str(out/(name+'.json'))),{'wait':250}]
def read(name): return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def option(name, key): return next(o for o in read(name)['sessionOptions'] if o['id']==key)
def new(): return [{'new_thread':str(out/'project')},{'wait_idle':30000},*settled()]
def run(name,steps):
    path=out/(name+'-steps.json');path.write_text(json.dumps(steps),encoding='utf-8')
    with (out/(name+'.log')).open('w',encoding='utf-8') as log:
        process=subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(path)),stdout=log,stderr=log)
        return ProcessGuard(process,timeout=180).wait()

try:
    peak=run('controls',[{'wait_ready':30000},{'resize':[1440,1100]},*new(),
        {'click_text':'us.openai.gpt-6.1-sol  ▾'},*settled(),*dump('initial'),
        *choose('provider','custom_bedrock_sol'),*choose('model',SOL),*dump('sol'),
        {'click_text':'Standard'},{'wait':150},{'key':'down'},*settled(),*dump('ultra'),
        *choose('thinking_effort','high'),*dump('effort'),{'snapshot':str(out/'compact-model-controls.png')},
        {'sidebar':['dump',str(out/'tabs.json')]},
        {'resize':[900,850]},{'wait':250},*dump('narrow'),
        *new(),*dump('second'),{'select_tab':0},{'wait':400},*dump('isolated'),{'quit':True}])
    expected={'aws_bedrock','custom_bedrock_sol','custom_helper'}
    assert set(option('initial','provider')['values'])==expected,read('initial')
    assert option('initial','speed')['values']==['default']
    assert option('sol','speed')['values']==['default','ultrafast'],read('sol')
    assert option('ultra','speed')['current']=='ultrafast',read('ultra')
    assert option('effort','speed')['current']=='ultrafast' and option('effort','thinking_effort')['current']=='high',read('effort')
    assert option('second','speed')['current']=='default'
    assert option('isolated','speed')['current']=='ultrafast'
    assert [o['id'] for o in read('ultra')['sessionOptions']]==['provider','mode','model','thinking_effort','speed']
    for name,width in [('effort',1440),('narrow',900)]:
        rows=read(name)['controlGeometry']
        assert not any('Goose' in r['text'] for r in rows),rows
        selector=next(r for r in rows if r['text']==SOL+'  ▴')
        speed=next(r for r in rows if r['text']=='Ultrafast')
        provider=next(r for r in rows if r['text']=='Bedrock Sol')
        assert 100 < selector['width'] < 550,selector
        for row in [selector,speed,provider]:
            assert row['width'] > 0 and row['x']+row['width'] <= width+1,row
        assert speed['width'] < 220 and provider['width'] < 300,(speed,provider)
        # Info sidebar may repeat labels: take those beside the Speed row.
        labels=[next(r for r in rows if r['text']==label and r['y']>selector['y']) for label in ['Provider','Mode','Model','Thinking effort','Speed']]
        assert max(r['x'] for r in labels)-min(r['x'] for r in labels)<1,labels
        assert 8 <= selector['x']-labels[0]['x'] <= 16,(selector,labels)
        assert 110 <= speed['x']-labels[-1]['x'] <= 145,(speed,labels)
        assert abs(speed['x']-provider['x'])<1,(speed,provider)
    first=json.loads((out/'tabs.json').read_text(encoding='utf-8'))['navigation']['openChats'][0]['id']
    peak=max(peak,run('restore',[{'wait_ready':30000},{'wait':600},{'resume':first},{'wait_idle':30000},{'click_text':'Resume'},*dump('restored'),
        *choose('model',LUNA),*dump('luna'),*choose('speed','ultrafast'),*dump('rejected'),
        *choose('model',SOL),*dump('reset'),{'quit':True}]))
    assert option('restored','speed')['current']=='ultrafast',read('restored')
    assert option('luna','speed')['values']==['default'] and option('luna','speed')['current']=='default',read('luna')
    assert 'not supported' in read('rejected')['sessionError'],read('rejected')
    assert option('reset','speed')['current']=='default'
    assert not marker.exists(),'Opening provider options ran a credential helper'
    assert all(path.read_bytes()==data for path,data in original.items()),'Existing provider configuration changed'
    assert not fixture.requests and not fixture.errors,(fixture.requests,fixture.errors)
    receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,
        configured_only=True,no_helper_probe=True,native_speed_click=True,per_session=True,restart=True,
        model_switch_reset=True,unsupported_rejected=True,compact_bounds=True,branding_removed=True)
    (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n')
    print('PASS: configured providers, compact native controls and persisted speed',flush=True)
finally:
    fixture.close();fixture.write_evidence(out)

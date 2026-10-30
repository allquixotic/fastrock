#!/usr/bin/env python3
"""Native named Goose profiles: isolated endpoints/vault keys, concurrent chats and moved-app recovery."""
import argparse
import ctypes as C
from ctypes import wintypes as W
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import subprocess
import threading
import time
import uuid
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert os.name == 'nt', 'Execute native verification on games only'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
binary, out = a.binary.resolve(), a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
base = ProviderFixture()
env = base.environment(out)
env['FASTROCK_WEBHOOK_BIND'] = '127.0.0.1:0'
secrets_path = out / 'goose/config/secrets.yaml'
original = b'ANTHROPIC_API_KEY: fixture-existing-preserve\n'
secrets_path.write_bytes(original)

current_stage = 'setup'
class Direct:
    def __init__(self, name, engine):
        self.name, self.engine = name, engine
        self.secret = 'fixture-' + uuid.uuid4().hex
        self.requests, self.errors, self.summaries, self.observations = [], [], [], []
        self.active, self.overlap = 0, False
        self.stop = threading.Event()
        owner = self
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def do_GET(self):
                raw = json.dumps({'data':[{'id':owner.name,'object':'model'}]}).encode()
                self.send_response(200); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(raw))); self.end_headers(); self.wfile.write(raw)
            def do_POST(self):
                try:
                    header = self.headers.get('x-api-key') if owner.engine == 'anthropic' else self.headers.get('Authorization')
                    expected = owner.secret if owner.engine == 'anthropic' else 'Bearer ' + owner.secret
                    assert header == expected, 'Wrong profile credential reached endpoint'
                    body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                    assert body['model'] == owner.name, 'Wrong profile model reached endpoint'
                    prompt = next((m['content'] for m in reversed(body['messages']) if m['role']=='user'),'')
                    if isinstance(prompt,list): prompt = ' '.join(c.get('text','') for c in prompt)
                    owner.requests.append(prompt)
                    owner.summaries.append('search index sidecar' in json.dumps(body))
                    owner.observations.append(dict(stage=current_stage,summary=owner.summaries[-1]))
                    owner.overlap |= any(s is not owner and s.active for s in servers)
                    if not body.get('stream', False):
                        response = ({'id':'fixture','type':'message','role':'assistant','model':owner.name,
                                     'content':[{'type':'text','text':'Reply from '+owner.name}], 'stop_reason':'end_turn',
                                     'usage':{'input_tokens':1,'output_tokens':4}} if owner.engine=='anthropic' else
                                    {'id':'fixture','object':'chat.completion','created':0,'model':owner.name,
                                     'choices':[{'index':0,'message':{'role':'assistant','content':'Reply from '+owner.name},'finish_reason':'stop'}],
                                     'usage':{'prompt_tokens':1,'completion_tokens':4,'total_tokens':5}})
                        raw=json.dumps(response).encode()
                        self.send_response(200); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(raw))); self.end_headers(); self.wfile.write(raw)
                        return
                    self.send_response(200); self.send_header('Content-Type','text/event-stream'); self.end_headers()
                    def event(value, kind=None):
                        raw = (('event: '+kind+'\n') if kind else '')+'data: '+json.dumps(value)+'\n\n'
                        self.wfile.write(raw.encode()); self.wfile.flush()
                    if owner.engine == 'anthropic':
                        event({'type':'message_start','message':{'id':'fixture','type':'message','role':'assistant','model':owner.name,'content':[],'usage':{'input_tokens':1,'output_tokens':0}}},'message_start')
                        event({'type':'content_block_start','index':0,'content_block':{'type':'text','text':''}},'content_block_start')
                        event({'type':'content_block_delta','index':0,'delta':{'type':'text_delta','text':'Reply from '+owner.name}},'content_block_delta')
                        event({'type':'content_block_stop','index':0},'content_block_stop')
                        event({'type':'message_delta','delta':{'stop_reason':'end_turn','stop_sequence':None},'usage':{'output_tokens':4}},'message_delta')
                        event({'type':'message_stop'},'message_stop')
                    else:
                        def chunk(text, reason=None):
                            event({'id':'fixture','object':'chat.completion.chunk','created':0,'model':owner.name,'choices':[{'index':0,'delta':{'content':text},'finish_reason':reason}]})
                        if prompt.startswith('hold'):
                            owner.active += 1
                            try:
                                for i in range(9000):
                                    if owner.stop.wait(.02): break
                                    chunk('Holding '+owner.name+' '+str(i)+'\n')
                            finally: owner.active -= 1
                        else: chunk('Reply from '+owner.name)
                        chunk('', 'stop')
                        self.wfile.write(b'data: [DONE]\n\n'); self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError): pass
                except Exception as e:
                    owner.errors.append(str(e)); self.send_error(500,'Fixture rejected request')
        self.server = ThreadingHTTPServer(('127.0.0.1',0),Handler)
        self.url = f'http://127.0.0.1:{self.server.server_port}' + ('/v1' if engine=='openai' else '')
        self.thread = threading.Thread(target=self.server.serve_forever,daemon=True); self.thread.start()
    def close(self):
        self.stop.set(); self.server.shutdown(); self.server.server_close(); self.thread.join(timeout=2)

servers = [Direct('fixture-a','openai'),Direct('fixture-b','openai'),Direct('fixture-c','anthropic'),Direct('fixture-a','openai')]
for i,s in enumerate(servers): env['PROFILE_FIXTURE_'+str(i)] = s.secret

def profile(*args): return {'settings':['profiles',*args]}
def setup(*args): return {'settings':['goose-setup',*args]}
def dump(name): return [profile('dump',str(out/(name+'.json'))),{'wait':300}]
def read(name): return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def create(name, server, index):
    return [profile('new'),{'wait':100},profile('name',name),profile('engine',server.engine),profile('url',server.url),
            profile('model',server.name),profile('secret-env','PROFILE_FIXTURE_'+str(index)),profile('save'),{'wait':1200}]
def run(steps,name,exe=binary):
    global current_stage
    current_stage=name
    script=out/(name+'-steps.json');script.write_text(json.dumps(steps),encoding='utf-8')
    with (out/(name+'.log')).open('w',encoding='utf-8') as log:
        proc=subprocess.Popen([str(exe)],env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
        edited=False
        def tick():
            nonlocal edited
            if name=='create' and not edited and (out/'selected.json').exists():
                path=out/'goose/config/custom_providers/custom_fixture_a.json'
                data=json.loads(path.read_text(encoding='utf-8'))
                data['display_name']='Fixture A externally edited'
                path.write_text(json.dumps(data),encoding='utf-8')
                edited=True
        return ProcessGuard(proc,timeout=150).wait(tick)

vault_ids=set()
try:
    peak=run([{'wait_ready':30000},{'resize':[1440,1100]},{'open_settings':'goose'},{'wait':800},
        *create('Fixture A',servers[0],0),*create('Fixture B',servers[1],1),*create('Fixture C',servers[2],2),*dump('created'),
        profile('select','Fixture A'),{'wait':100},profile('url','https://key@example.invalid'),profile('save'),{'wait':300},*dump('invalid'),
        profile('select','Fixture A'),{'wait':100},*dump('selected'),{'wait':1500},profile('save'),{'wait':600},*dump('stale'),
        profile('select','custom_fixture_a'),{'wait':100},profile('name','Fixture A'),profile('save'),{'wait':1000},*dump('retained-key'),{'snapshot':str(out/'profiles.png')},
        setup('reload'),{'wait':500},setup('provider','Fixture A'),setup('model','fixture-a'),setup('save'),{'wait':800},{'quit':True}], 'create')
    created=read('created')
    assert created['secretEmpty'] and len(created['profiles'])==3,created
    ids={p['name']:p['id'] for p in created['profiles']}
    vault_ids.update(p['credential_id'] for p in created['profiles'])
    assert 'cannot contain' in read('invalid')['status'],read('invalid')
    assert 'Profile changed' in read('stale')['status'],read('stale')
    assert any(p['name']=='Fixture A externally edited' for p in read('stale')['profiles']), 'Stale form overwrote external edit'
    assert next(p['credential_id'] for p in read('retained-key')['profiles'] if p['name']=='Fixture A') == next(p['credential_id'] for p in created['profiles'] if p['name']=='Fixture A'), 'Blank key edit replaced credential'
    assert secrets_path.read_bytes()==original
    peak=max(peak,run([{'wait_ready':30000},{'new_thread':str(out/'project')},{'wait_idle':30000},
        {'send':'hold profile A'},{'wait':2500}, {'new_thread':str(out/'project')},{'wait_idle':30000},
        setup('choose','provider',ids['Fixture B']),{'wait':1000},{'send':'profile B request'},{'wait_idle':30000},{'wait':500},
        {'transcript':'export-complete:'+str(out/'b.md')},{'select_tab':0},{'shortcut':'escape'},{'wait_idle':10000},
        {'new_thread':str(out/'project')},{'wait_idle':30000},setup('choose','provider',ids['Fixture C']),{'wait':1000},
        {'send':'profile C request'},{'wait_idle':30000},{'wait':500},{'transcript':'export-complete:'+str(out/'c.md')},
        {'open_settings':'goose'},{'wait':800},profile('select','Fixture A'),{'wait':100},profile('url',servers[3].url),
        profile('secret-env','PROFILE_FIXTURE_3'),profile('save'),{'wait':1200},*dump('rotated'),profile('cheap','Fixture B'),{'wait':700},{'quit':True}], 'concurrent'))
    assert servers[1].overlap,'Second named profile did not run while first streamed'
    assert 'Reply from fixture-b' in (out/'b.md').read_text(encoding='utf-8')
    assert 'Reply from fixture-c' in (out/'c.md').read_text(encoding='utf-8')
    rotated=read('rotated');vault_ids.update(p['credential_id'] for p in rotated['profiles'])
    old=next(p for p in created['profiles'] if p['name']=='Fixture A')
    new=next(p for p in rotated['profiles'] if p['name']=='Fixture A')
    assert new['credential_id']!=old['credential_id'] and new['url']==servers[3].url,rotated
    moved=out/'moved-fastrock.exe';shutil.copy2(binary,moved)
    peak=max(peak,run([{'wait_ready':30000},{'new_thread':str(out/'project')},{'wait_idle':30000},
        {'send':'rotated profile A after move'},{'wait_idle':30000},{'wait_summaries':30000},{'wait':700},
        {'transcript':'export-complete:'+str(out/'moved.md')},{'open_settings':'goose'},{'wait':600},*dump('restored'),{'quit':True}], 'moved',moved))
    assert any('rotated profile A after move' in r for r in servers[3].requests),servers[3].requests
    for path in (out/'goose/config/custom_providers').glob('*.json'):
        data=json.loads(path.read_text(encoding='utf-8'))
        assert Path(data['auth']['command'])==moved
        assert data['api_key_env']=='' and data['auth']['args'][0]=='--fastrock-provider-credential'
    assert secrets_path.read_bytes()==original
    assert 'FASTROCK_CHEAP_PROFILE' in (out/'goose/config/config.yaml').read_text(encoding='utf-8')
    assert any(servers[1].summaries), 'Named cheap profile was not used'
    for path in out.rglob('*'):
        if path.is_file() and path.suffix in ('.json','.jsonl','.yaml','.yml','.log'):
            contents=path.read_text(encoding='utf-8',errors='ignore')
            assert all(s.secret not in contents for s in servers),f'Credential leaked to {path.name}'
    assert all(not s.errors for s in servers),[s.errors for s in servers]
    result=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,
                named_profiles=3,stale_editor_rejected=True,blank_key_preserved=True,independent_concurrent_credentials=True,direct_anthropic=True,named_cheap_profile=True,
                rotated_endpoint_and_credential=True,moved_helper_repaired=True,legacy_credentials_unchanged=True,no_plaintext_keys=True)
    (out/'result.json').write_text(json.dumps(result,indent=2),encoding='utf-8')
    print('PASS: named profiles, concurrent endpoint/credential isolation, key rotation and moved-app recovery',flush=True)
finally:
    (out/'fixture-diagnostics.json').write_text(json.dumps([dict(model=s.name,requests=s.observations,errors=s.errors) for s in servers],indent=2),encoding='utf-8')
    base.close()
    for s in servers:s.close()
    # Only delete the fixture's explicitly created UUIDs, never enumerate the vault.
    for path in (out/'goose/config/custom_providers').glob('*.json'):
        data=json.loads(path.read_text(encoding='utf-8'));vault_ids.add(data['auth']['args'][1])
    api=C.WinDLL('Advapi32',use_last_error=True);api.CredDeleteW.argtypes=[W.LPCWSTR,W.DWORD,W.DWORD]
    for identity in vault_ids: api.CredDeleteW('fastrock:provider/'+identity,1,0)

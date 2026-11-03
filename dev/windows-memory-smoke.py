#!/usr/bin/env python3
"""Real embedded memory learning/recall, scope, native editing and persistence."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import select
import socket
import threading
import time
from embedded_provider_fixture import ProviderFixture, ProcessGuard
assert os.name == 'nt'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
binary, out = a.binary.resolve(), a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
class MemoryFixture(ProviderFixture):
    def __init__(self):
        super().__init__()
        self.learning = []
        self.recalled = []
        self.recall_results = []
        self.stalled = threading.Event()
        self.cancelled = threading.Event()
        self.probe = threading.Event()
    def respond(self, handler, body):
        system = ' '.join(c.get('text', '') for c in body.get('system', []))
        if 'Fastrock conservative memory extraction' in system:
            user = json.loads(body['messages'][0]['content'][0]['text'])['user_statement']
            self.learning.append(user)
            if user == 'Neon means our staging cluster' and not self.stalled.is_set():
                self.stalled.set()
                until = time.monotonic() + 20
                while time.monotonic() < until:
                    if select.select([handler.connection], [], [], .05)[0]:
                        try: closed = not handler.connection.recv(1, socket.MSG_PEEK)
                        except (ConnectionResetError, ConnectionAbortedError): closed = True
                        if closed:
                            self.cancelled.set()
                            return
                raise AssertionError('Foreground did not cancel stalled memory HTTP request')
            facts = ([dict(key='Quartz', text='Quartz means our release approval service',
                           evidence='Quartz means our release approval service', kind='terminology')]
                     if user == 'Quartz means our release approval service' else [])
            return self.json_response(handler, dict(output={'message':{'role':'assistant','content':[{'text':json.dumps(facts)}]}},
                stopReason='end_turn', usage={'inputTokens':1,'outputTokens':1,'totalTokens':2},metrics={'latencyMs':1}))
        if 'search index sidecar' not in system:
            user = next((' '.join(c.get('text','') for c in m['content']) for m in reversed(body['messages']) if m['role']=='user' and any('text' in c for c in m['content'])), '').split('<turn-context>',1)[0].strip()
            self.recalled.append(dict(prompt=user, system=system))
            if user.startswith('What does Quartz mean'):
                results = [c['toolResult'] for c in body['messages'][-1]['content'] if 'toolResult' in c]
                assert user not in self.learning, 'Learning ran before final handoff'
                if not results:
                    names = [t['toolSpec']['name'] for t in body.get('toolConfig',{}).get('tools',[]) if 'toolSpec' in t]
                    name = next((n for n in names if n == 'fastrockmemories__recall'), None)
                    assert name, names
                    # Parallel calls must not multiply the memory context budget.
                    content = [{'toolUse':dict(toolUseId=f'memory-{i}',name=name,input={'query':'Quartz'})} for i in range(2)]
                    return self.json_response(handler, dict(output={'message':{'role':'assistant','content':content}},stopReason='tool_use',usage={'inputTokens':1,'outputTokens':1,'totalTokens':2},metrics={'latencyMs':1}))
                assert all(r.get('status') != 'error' for r in results), results
                self.recall_results.append(dict(prompt=user, texts=[c.get('text','') for r in results for c in r['content']]))
            if user.startswith('foreground-after-stalled-memory'):
                assert self.stalled.is_set(), 'Fixture failed to stall memory first'
                self.probe.set()
        return super().respond(handler, body)
provider = MemoryFixture()
env = provider.environment(out)
(out/'other-project').mkdir()
def control(*args): return {'settings':['memories',*args]}
def dump(name): return [control('dump',str(out/(name+'.json'))),{'wait':300}]
def read(name): return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def run(name, steps):
    script = out/(name+'-steps.json'); script.write_text(json.dumps(steps),encoding='utf-8')
    with (out/(name+'.log')).open('w',encoding='utf-8') as log:
        process = subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
        return ProcessGuard(process, timeout=150).wait()
try:
    peak = run('learn', [ {'wait_ready':30000},{'new_thread':str(out/'project')},{'wait_idle':30000},
        {'send':'Quartz means our release approval service'},{'wait_idle':30000},{'wait_summaries':30000},{'wait':7000},
        {'open_settings':'memories'},{'wait':1000},*dump('learned'),control('select-first'),{'wait':200},
        {'snapshot':str(out/'memories.png')},
        {'new_thread':str(out/'project')},{'wait_idle':30000},{'send':'What does Quartz mean in this project?'},{'wait_idle':30000},
        {'transcript':'export-complete:'+str(out/'recall.md')},
        {'new_thread':str(out/'other-project')},{'wait_idle':30000},{'send':'What does Quartz mean elsewhere?'},{'wait_idle':30000},
        {'open_settings':'memories'},{'wait':600},control('select-first'),control('text','Quartz is our updated release service'),control('pin','true'),control('update'),{'wait':800},*dump('edited'),{'quit':True}])
    assert read('learned')['count']==1,read('learned')
    same = next(r for r in provider.recall_results if r['prompt'].startswith('What does Quartz mean in this project?'))
    other = next(r for r in provider.recall_results if r['prompt'].startswith('What does Quartz mean elsewhere?'))
    recalled = [t for t in same['texts'] if '<fastrock_memories>' in t]
    assert len(recalled)==1 and 'release approval service' in recalled[0] and len(recalled[0].encode('utf-8')) <= 2048, same
    assert all('<fastrock_memories>' not in t for t in other['texts']), other
    peak = max(peak,run('restart', [ {'wait_ready':30000},{'wait':4000},{'open_settings':'memories'},{'wait':800},*dump('restored'),
        {'new_thread':str(out/'project')},{'wait_idle':30000},{'send':'What does Quartz mean after restart?'},{'wait_idle':30000},
        {'open_settings':'memories'},{'wait':700},
        control('select-first'),{'wait':200},control('forget'),{'wait':700},*dump('forgotten'),
        {'new_thread':str(out/'project')},{'wait_idle':30000},{'send':'What does Quartz mean after forgetting?'},{'wait_idle':30000},
        {'new_thread':str(out/'project')},{'wait_idle':30000},{'send':'Quartz means our release approval service'},{'wait_idle':30000},{'wait_summaries':30000},{'wait':6000},
        {'open_settings':'memories'},{'wait':700},*dump('not-resurrected'),control('toggle-learning'),{'wait':500},control('toggle-recall'),{'wait':500},*dump('disabled'),{'quit':True}]))
    assert read('restored')['count']==1
    assert read('forgotten')['count']==0
    assert read('not-resurrected')['count']==0
    assert not read('disabled')['learning'] and not read('disabled')['recall']
    restored = next(r for r in provider.recall_results if r['prompt']=='What does Quartz mean after restart?')
    forgotten = next(r for r in provider.recall_results if r['prompt']=='What does Quartz mean after forgetting?')
    assert any('updated release service' in t for t in restored['texts']), restored
    assert all('<fastrock_memories>' not in t for t in forgotten['texts']), forgotten
    peak = max(peak,run('priority', [ {'wait_ready':30000},{'open_settings':'memories'},{'wait':1000},control('toggle-learning'),{'wait':500},
        {'new_thread':str(out/'project')},{'wait_idle':30000},{'send':'Neon means our staging cluster'},
        {'wait_idle':30000},{'wait_summaries':30000},{'wait':7000},
        {'send':'foreground-after-stalled-memory'},{'wait_idle':10000},{'wait':800},{'quit':True}]))
    assert provider.stalled.is_set() and provider.cancelled.is_set() and provider.probe.is_set(), dict(stalled=provider.stalled.is_set(),cancelled=provider.cancelled.is_set(),probe=provider.probe.is_set(),learning=provider.learning)
    assert not provider.errors,provider.errors
    receipt = dict(passed=True,sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peakBytes=peak,
                   learningRequests=len(provider.learning),foregroundRequests=len(provider.recalled),
                   checks=['completed-turn learning','actual model recall tool','parallel per-turn budget','repository isolation','native edit/pin',
                           'stalled HTTP foreground cancellation','restart persistence','forget tombstone','learning/recall toggles'])
    (out/'acceptance.json').write_text(json.dumps(receipt,indent=2),encoding='utf-8')
    print(json.dumps(receipt))
finally:
    (out/'memory-provider.json').write_text(json.dumps(dict(learning=provider.learning, recalled=provider.recalled, recallResults=provider.recall_results, errors=provider.errors),indent=2),encoding='utf-8')
    provider.close()

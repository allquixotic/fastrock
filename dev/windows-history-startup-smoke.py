#!/usr/bin/env python3
"""V100: saved history survives failed and >90-second agent activation."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import time
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert os.name == 'nt', 'Native acceptance runs on Windows only'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--seed-executable', type=Path)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
binary, out = a.binary.resolve(), a.output.resolve()
seed = a.seed_executable
if seed is None:
    target = json.loads((binary.parent/'TEST-TARGETS.json').read_text())['embedded_goose']
    assert Path(target).name == target
    seed = binary.parent/target
out.mkdir(parents=True, exist_ok=False)

class SlowTools(ProviderFixture):
    def __init__(self):
        self.initializations = []
        super().__init__(tools=True)

    def mcp(self, handler, body):
        if body.get('method') == 'initialize':
            # No application-specific delay hooks: stall a real configured MCP
            # extension while the unchanged embedded Goose server activates it.
            self.initializations.append(dict(started=time.monotonic(), history_visible=(out/'before-slow.md').exists()))
            assert self.stop.wait(105) is False, 'Fixture stopped during initialization'
        super().mcp(handler, body)

provider = SlowTools()
env = provider.environment(out)
config = out/'goose/config/config.yaml'
value = json.loads(config.read_text())
value['extensions']['fixture']['timeout'] = 180
config.write_text(json.dumps(value), encoding='utf-8')
parameters = out/'seed-parameters.json'
parameters.write_text(json.dumps(dict(sessions=2, messages=400, project=str(out/'project'),
    vwork_home=str(out/'local/vwork'), output=str(out/'seed.json'))), encoding='utf-8')

def run_command(command, name, timeout=180):
    with (out/(name+'.log')).open('w', encoding='utf-8') as log:
        process = subprocess.Popen(command, cwd=out, env=env, stdout=log, stderr=log)
        return ProcessGuard(process, timeout=timeout).wait()

def run(name, steps):
    path = out/(name+'-steps.json')
    path.write_text(json.dumps(steps), encoding='utf-8')
    env['CODEX_GUI_AUTOMATION'] = str(path)
    return run_command([str(binary)], name, 190)

def export(name):
    return [{'transcript':'export-complete:'+str(out/(name+'.md'))}, {'wait':400},
            {'settings':['goose-setup','dump',str(out/(name+'.json'))]}, {'wait':200}]

def read(name):
    return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))

def assert_history(name, questions=200):
    text = (out/(name+'.md')).read_text(encoding='utf-8')
    assert text.count('## User\n') == questions, (name, text.count('## User\n'))
    assert 'Fixture prompt 0/0:' in text and 'HISTORY-END-SENTINEL' in text, name
    assert 'timed out' not in text.lower(), (name, text[-1000:])
    return text

try:
    peak = run_command([str(seed.resolve()), '--seed-benchmark', str(parameters)], 'seed')
    sid = json.loads((out/'seed.json').read_text())['sessions'][0]
    database = next((out/'goose').rglob('sessions.db'))
    with sqlite3.connect(database) as connection:
        connection.execute('UPDATE sessions SET provider_name=? WHERE id=?', ('unavailable-fixture-provider',sid))
    opening = [{'wait_ready':30000}, {'resize':[1440,1000]}, {'resume':'goose:'+sid}, {'wait_idle':30000}]
    peak = max(peak, run('unavailable', [*opening, *export('before-failure'),
        {'click_text':'Model options  ▾'}, {'wait':2000}, *export('failed'), {'snapshot':str(out/'failed.png')},
        {'click_text':'Retry'}, {'wait_goose':30000}, *export('retried'), {'quit':True}]))
    before = assert_history('before-failure')
    assert assert_history('failed') == before and assert_history('retried') == before
    assert read('failed')['sessionError'] and read('retried')['sessionError'], 'Real activation errors need a visible retry'
    assert not provider.initializations and not provider.requests, 'Browsing activated a provider or extension'
    with sqlite3.connect(database) as connection:
        connection.execute('UPDATE sessions SET provider_name=? WHERE id=?', ('aws_bedrock',sid))
    peak = max(peak, run('slow', [*opening, *export('before-slow'), {'click_text':'Model options  ▾'},
        {'wait':95000}, *export('during-slow'), {'snapshot':str(out/'waiting.png')},
        {'wait_goose':60000}, *export('activated'), {'send':'Continue after delayed activation'},
        {'wait_idle':30000}, *export('continued'), {'quit':True}]))
    assert assert_history('during-slow') == assert_history('before-slow')
    assert read('during-slow')['sessionChanging'], read('during-slow')
    assert not read('during-slow')['sessionError'], read('during-slow')
    assert assert_history('activated') == before, 'ACP tail replay duplicated or lost saved messages'
    assert not read('activated')['sessionChanging'] and not read('activated')['sessionError']
    assert_history('continued', questions=201)
    assert len(provider.initializations) == 1 and provider.initializations[0]['history_visible'], provider.initializations
    assert len([r for r in provider.requests if not r['summary']]) == 1, 'Opening history ran inference'
    assert not provider.errors, provider.errors
    (out/'acceptance.json').write_text(json.dumps(dict(result='PASS',
        binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), peak_process_bytes=peak,
        saved_messages=400, failed_activation_retry=True, extension_delay_seconds=105,
        history_readable_while_activation_waits=True, no_duplicate_replay=True), indent=2), encoding='utf-8')
    print('PASS: offline history, retry, 105-second activation, continued conversation')
finally:
    provider.close()
    provider.write_evidence(out)

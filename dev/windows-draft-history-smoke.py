#!/usr/bin/env python3
"""V107/V108: native composer undo/redo across tabs, crash, restart and Send."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert os.name == 'nt', 'Run native acceptance on games only'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
binary, out = a.binary.resolve(), a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
provider = ProviderFixture()
env = provider.environment(out)
env['FASTROCK_WEBHOOK_BIND'] = '127.0.0.1:0'
FIRST, SECOND = 'Draft é 🔭\nFirst paragraph', 'Draft é 🔭\nRevised paragraph'

def dump(name):
    return [{'composer': ['dump', str(out / (name + '.json'))]}, {'wait': 300}]

def text(name):
    return json.loads((out / (name + '.json')).read_text(encoding='utf-8'))['text']

def run(name, steps, crash=False):
    script = out / (name + '-steps.json')
    script.write_text(json.dumps(steps), encoding='utf-8')
    with (out / (name + '.log')).open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(binary)], env=dict(env, CODEX_GUI_AUTOMATION=str(script)), stdout=log, stderr=log)
        guard = ProcessGuard(process, timeout=120)
        if not crash:
            return guard.wait()
        try:
            while process.poll() is None:
                guard.sample()
                saved = out / 'codex/fastrock-open-chats.json'
                if (out / 'crash-ready.json').exists() and saved.exists():
                    draft = json.loads(saved.read_text(encoding='utf-8'))['windows']['1']['chats'][0]['draft']
                    if draft['text'] == FIRST and draft['history']['redo']:
                        process.kill()
                        process.wait(timeout=10)
                        return guard.peak
                time.sleep(.1)
            raise AssertionError('App exited before durable draft/redo snapshot')
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=10)

try:
    peak = run('crash', [
        {'wait_ready': 30000}, {'new_thread': str(out / 'project')}, {'wait_idle': 30000},
        {'send': 'Saved conversation before draft'}, {'wait_idle': 30000}, {'wait_summaries': 30000},
        {'composer': ['type', FIRST]}, {'composer': ['type', SECOND]},
        {'key': 'ctrl+z'}, {'wait': 300}, *dump('first-undo'),
        {'new_thread': str(out / 'project')}, {'wait_idle': 30000},
        {'composer': ['type', 'Other conversation draft']}, {'select_tab': 0}, {'wait': 300},
        *dump('crash-ready'), {'wait': 120000},
    ], crash=True)
    assert text('first-undo') == FIRST and text('crash-ready') == FIRST
    before = len([r for r in provider.requests if not r['summary']])
    peak = max(peak, run('restore', [
        {'wait_ready': 30000}, {'wait_idle': 30000}, {'wait': 1000}, *dump('restored'),
        {'key': 'ctrl+y'}, {'wait': 300}, *dump('redo-after-crash'),
        {'key': 'ctrl+z'}, {'wait': 200}, {'key': 'ctrl+z'}, {'wait': 200}, *dump('undo-all'),
        {'key': 'ctrl+shift+z'}, {'wait': 200}, *dump('redo-shortcut'),
        {'composer': ['type', 'New branch after Undo']}, {'key': 'ctrl+y'}, {'wait': 200}, *dump('branch'),
        {'select_tab': 1}, {'wait': 200}, *dump('other'), {'select_tab': 0}, {'wait': 200},
        {'composer': ['send']}, {'wait_idle': 30000}, {'wait_summaries': 30000}, *dump('sent'),
        {'key': 'ctrl+z'}, {'wait': 300}, *dump('undo-send'), {'quit': True},
    ]))
    assert text('restored') == FIRST
    assert text('redo-after-crash') == SECOND
    assert text('undo-all') == ''
    assert text('redo-shortcut') == FIRST
    assert text('branch') == 'New branch after Undo'
    assert text('other') == 'Other conversation draft'
    assert text('sent') == '' and text('undo-send') == 'New branch after Undo'
    ordinary = [r for r in provider.requests if not r['summary']]
    assert len(ordinary) == before + 1 and ordinary[-1]['prompt'] == 'New branch after Undo', ordinary
    peak = max(peak, run('clean-restart', [
        {'wait_ready': 30000}, {'wait_idle': 30000}, {'wait': 700}, *dump('clean'),
        {'key': 'ctrl+y'}, {'wait': 300}, *dump('redo-send'), {'quit': True},
    ]))
    assert text('clean') == 'New branch after Undo' and text('redo-send') == ''
    assert len([r for r in provider.requests if not r['summary']]) == before + 1
    assert not provider.errors, provider.errors
    (out / 'acceptance.json').write_text(json.dumps(dict(result='PASS', binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
        peak_process_bytes=peak, forced_crash=True, durable_undo_redo=True, unicode=True, tab_isolation=True,
        ordinary_send_after_restart=True, undo_send=True, redo_branch=True, clean_restart=True, no_replay=True), indent=2), encoding='utf-8')
    print('PASS: durable composer undo/redo, crash/restart and normal Send')
finally:
    provider.close()
    provider.write_evidence(out)

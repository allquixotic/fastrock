#!/usr/bin/env python3
"""Native signed webhooks, vault keys, durable receipts and restart safety. Local fixtures only."""
import argparse
import ctypes as C
from ctypes import wintypes as W
import hashlib
import hmac
import http.client
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from urllib.parse import urlsplit
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert os.name == 'nt', 'Native acceptance runs only on Windows'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
binary, out = a.binary.resolve(), a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
provider = ProviderFixture(stream_chunks=9000)
env = provider.environment(out, streaming=True)
env['FASTROCK_WEBHOOK_BIND'] = '127.0.0.1:0'
repo = out / 'project'
(repo / 'source.txt').write_text('Original source\n', encoding='utf-8')
for args in (['init', '.'], ['add', '.'], ['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-m', 'Fixture']):
    subprocess.run(['git', '-c', 'core.hooksPath=NUL', *args], cwd=repo, check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
store = out / 'home/goose-queues/schedules.json'

class Credential(C.Structure):
    _fields_ = [('Flags', W.DWORD), ('Type', W.DWORD), ('TargetName', W.LPWSTR), ('Comment', W.LPWSTR),
                ('LastWritten', W.FILETIME), ('CredentialBlobSize', W.DWORD), ('CredentialBlob', C.POINTER(C.c_ubyte)),
                ('Persist', W.DWORD), ('AttributeCount', W.DWORD), ('Attributes', C.c_void_p),
                ('TargetAlias', W.LPWSTR), ('UserName', W.LPWSTR)]
api = C.WinDLL('Advapi32', use_last_error=True)
api.CredReadW.argtypes = [W.LPCWSTR, W.DWORD, W.DWORD, C.POINTER(C.POINTER(Credential))]
api.CredReadW.restype = W.BOOL
api.CredFree.argtypes = [C.c_void_p]
api.CredDeleteW.argtypes = [W.LPCWSTR, W.DWORD, W.DWORD]

def key(identity):
    ptr = C.POINTER(Credential)()
    assert api.CredReadW('fastrock:webhook/' + identity, 1, 0, C.byref(ptr)), 'Fixture signing key missing from vault'
    try:
        raw = C.string_at(ptr.contents.CredentialBlob, ptr.contents.CredentialBlobSize)
        # keyring 3 stores password bytes as UTF-8; tolerate native UTF-16 vaults.
        return raw.decode('utf-16-le' if b'\0' in raw else 'utf-8')
    finally:
        api.CredFree(ptr)

identity, signing, url = None, None, None
keys = []
checked = set()

def action(*args):
    return {'settings': ['schedules', *args]}

def dump(name):
    return [action('refresh'), {'wait': 500}, action('dump', str(out / (name + '.json'))), {'wait': 300}]

def snapshot(name):
    return json.loads((out / (name + '.json')).read_text(encoding='utf-8'))

def record(name):
    return next(t for t in snapshot(name)['data']['tasks'] if t['title'] == 'Webhook fixture')

def payload(identity, text='hook-complete', action='push', group='g'):
    return dict(id=identity, text=text, action=action, group=group)

def send(body, expect=202, secret=None, extra=None, signature_body=None):
    raw = body if isinstance(body, bytes) else json.dumps(body, separators=(',', ':')).encode()
    headers = {'Content-Type': 'application/json', 'Authorization': 'Bearer must-not-reach-model',
               'Cookie': 'must-not-reach-model', 'X-Api-Key': 'must-not-reach-model'}
    secret = signing if secret is None else secret
    if secret:
        headers['x-hub-signature-256'] = 'sha256=' + hmac.new(secret.encode(), signature_body or raw, hashlib.sha256).hexdigest()
    if extra:
        headers.update(extra)
    target = urlsplit(url)
    conn = http.client.HTTPConnection(target.hostname, target.port, timeout=5)
    try:
        conn.request('POST', target.path + '?token=must-not-reach-model&visible=yes', raw, headers)
        res = conn.getresponse()
        data = res.read()
        assert res.status == expect, (res.status, expect, data)
        return json.loads(data) if data else {}
    finally:
        conn.close()

def once(name, callback):
    if name not in checked and (out / (name + '.json')).exists():
        callback()
        checked.add(name)

def run(steps, name, tick=None, crash=False):
    script = out / (name + '-steps.json')
    script.write_text(json.dumps(steps), encoding='utf-8')
    with (out / (name + '.log')).open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(binary), '--remote', 'goose'], cwd=repo,
            env=dict(env, CODEX_GUI_AUTOMATION=str(script)), stdout=log, stderr=log)
        guard = ProcessGuard(process, timeout=150)
        if not crash:
            return guard.wait(tick)
        try:
            while process.poll() is None:
                guard.sample()
                if tick:
                    tick()
                if provider.active_streams == 1:
                    active = next(iter(json.loads(store.read_text())['tasks'].values()))['active']
                    if active and active['child']:
                        process.kill()
                        process.wait(timeout=10)
                        return guard.peak
                time.sleep(.1)
            raise AssertionError('Exited before webhook crash boundary')
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=10)

def setup_ready():
    global identity, signing, url
    data = snapshot('ready')
    task = record('ready')
    assert task['paused'], task
    identity = task['id']
    signing = key(identity)
    keys.append(signing)
    url = data['data']['webhookAddress'] + '/hooks/' + identity
    raw = json.dumps(payload('bad')).encode()
    send(raw, 401, secret='')
    send(raw, 401, secret='wrong')
    send(raw, 401, signature_body=b'{}')
    send(raw, 403, extra={'Origin': 'https://example.invalid'})
    send(raw, 403, extra={'Host': 'example.invalid'})
    send(b'x' * 65537, 413)
    send(b'not json', 400)
    first = send(payload('a', 'hook-old'))
    duplicate = send(payload('a', 'hook-old'), 200)
    assert duplicate['id'] == first['id']
    send(payload('b', 'hook-new'))
    assert send(payload('filtered', action='ignore'))['outcome'] == 'filtered'

def rotated_ready():
    global signing
    old = signing
    signing = key(identity)
    assert signing != old, 'Rotation did not replace vault key'
    keys.append(signing)
    send(payload('rotated'), 401, secret=old)
    send(payload('rotated', 'hook-rotated'))

def pending_ready():
    send(payload('pending', 'hook-after-restart'))

def restore_ready():
    global url
    data = snapshot('restored')
    url = data['data']['webhookAddress'] + '/hooks/' + identity
    task = record('restored')
    assert task['paused'] and sum(d['status'] == 'pending' for d in task['deliveries']) == 1, task
    assert not any(r['prompt'].startswith('hook-after-restart') for r in provider.requests if not r['summary'])
    send(payload('pending', 'hook-after-restart'), 200)

def crash_ready():
    global url
    url = snapshot('crash-ready')['data']['webhookAddress'] + '/hooks/' + identity
    send(payload('crash', 'hold-webhook'))

peak = 0
try:
    steps = [{'wait_ready': 30000}, {'open_folder': str(repo)}, {'wait': 1000}, {'wait_idle': 30000},
             {'send': 'source-context-marker'}, {'wait': 300}, {'wait_idle': 30000},
             {'open_settings': 'schedules'}, {'wait': 800}, action('new'), {'wait': 100},
             action('title', 'Webhook fixture'), action('kind', '3'), action('prompt', '{{body.text}} {{request}}'),
             action('replay-key', '{{body.id}}'), action('coalesce-key', '{{body.group}}'),
             action('coalesce-seconds', '10'), action('filter', 'body.action', '0', 'push'), action('save'), {'wait': 1000},
             action('pause', 'Webhook fixture'), {'wait': 500}, *dump('ready'), {'wait': 3000},
             *dump('accepted'), action('resume', 'Webhook fixture'), {'wait': 13000}, *dump('finished'),
             action('select', 'Webhook fixture'), {'wait': 100}, {'snapshot': str(out / 'webhook-editor.png')},
             action('rotate-key', 'Webhook fixture'), {'wait': 600}, *dump('rotated'), {'wait': 13000},
             *dump('rotation-finished'), action('pause', 'Webhook fixture'), {'wait': 500}, *dump('pending-ready'),
             {'wait': 2500}, *dump('pending'), {'quit': True}]
    peak = max(peak, run(steps, 'initial', lambda: (once('ready', setup_ready), once('rotated', rotated_ready), once('pending-ready', pending_ready))))
    accepted = record('accepted')
    assert [d['status'] for d in accepted['deliveries']] == ['coalesced', 'pending', 'filtered'], accepted
    assert record('finished')['deliveries'][1]['status'] == 'completed', record('finished')
    assert record('rotation-finished')['deliveries'][-1]['status'] == 'completed', record('rotation-finished')
    peak = max(peak, run([{'wait_ready': 30000}, {'open_settings': 'schedules'}, {'wait': 1500}, *dump('restored'),
        {'wait': 2000}, action('resume', 'Webhook fixture'), {'wait': 13000}, *dump('resumed'),
        *dump('crash-ready'), {'wait': 30000}], 'crash', lambda: (once('restored', restore_ready), once('crash-ready', crash_ready)), crash=True))
    peak = max(peak, run([{'wait_ready': 30000}, {'open_settings': 'schedules'}, {'wait': 2500}, *dump('uncertain'),
        action('resume', 'Webhook fixture'), {'wait': 3000}, *dump('no-replay'),
        action('delete', 'Webhook fixture'), {'wait': 700}, *dump('deleted'), {'quit': True}], 'recovery'))
    uncertain = record('uncertain')
    assert uncertain['paused'] and uncertain['active'] is None and uncertain['last']['status'] == 'needs_review', uncertain
    assert uncertain['deliveries'][-1]['status'] == 'needs_review', uncertain
    assert record('no-replay')['active'] is None
    assert snapshot('deleted')['data']['tasks'] == []
    requests = [r for r in provider.requests if not r['summary']]
    for prefix in ('hook-new', 'hook-rotated', 'hook-after-restart', 'hold-webhook'):
        assert sum(r['prompt'].startswith(prefix) for r in requests) == 1, (prefix, [r['prompt'] for r in requests])
    for r in requests:
        if r['prompt'].startswith(('hook-', 'hold-webhook')):
            assert 'must-not-reach-model' not in r['prompt'] and 'x-hub-signature-256' not in r['prompt'], 'Credential headers reached prompt'
            assert 'source-context-marker' not in json.dumps(r['body']), 'Parent context leaked into scheduled child'
    for path in out.rglob('*'):
        if path.is_file() and path.suffix in ('.json', '.yaml', '.log'):
            contents = path.read_text(encoding='utf-8', errors='ignore')
            assert all(k not in contents for k in keys), f'Signing key persisted outside vault: {path.name}'
    ptr = C.POINTER(Credential)()
    found = api.CredReadW('fastrock:webhook/' + identity, 1, 0, C.byref(ptr))
    if found:
        api.CredFree(ptr)
    assert not found, 'Deleting webhook did not remove its vault key'
    assert not provider.errors, provider.errors
    result = dict(binarySha256=hashlib.sha256(binary.read_bytes()).hexdigest(), peakPrivateBytes=peak,
                  passed=['native editor', 'HMAC and request boundaries', 'vault-only keys', 'replay guard', 'filters', 'coalescing',
                          'paused pending receipts', 'key rotation', 'restart Resume', 'uncertain run never replayed', 'key deletion'])
    (out / 'result.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
    print('PASS: native webhook scheduling, vault, restart and crash recovery', flush=True)
finally:
    provider.close()
    if identity:
        api.CredDeleteW('fastrock:webhook/' + identity, 1, 0)

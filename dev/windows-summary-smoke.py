#!/usr/bin/env python3
"""V40/V44: native embedded-Goose conversation, summary search and settings.

Run on Windows only. Every provider request goes to this local HTTP fixture.
"""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import importlib.util
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time

assert sys.platform == 'win32', "Run native acceptance on Windows only"
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--turns', type=int, default=1)
parser.add_argument('--navigation', action='store_true')
parser.add_argument('--windows', action='store_true', help='Exercise two in-process windows and draft transfer')
args = parser.parse_args()
assert 1 <= args.turns <= 100
assert not args.windows or args.navigation, '--windows requires --navigation'
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
for folder in ('home', 'codex', 'project', 'goose/config', 'local', 'roaming'):
    (out / folder).mkdir(parents=True)
(out / 'aws-empty').write_text('')
(out / 'goose/config/config.yaml').write_text(
    'GOOSE_PROVIDER: aws_bedrock\nGOOSE_MODEL: openai.gpt-6.1-sol\n'
    'BEDROCK_DISABLE_STREAMING: true\nBEDROCK_MAX_RETRIES: 0\n'
    'GOOSE_DISABLE_SESSION_NAMING: true\nextensions: {}\n', encoding='utf-8')
requests = []


class Provider(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        summary = 'search index sidecar' in json.dumps(body)
        requests.append({'path': self.path, 'summary': summary})
        assert self.path.startswith('/model/') and self.path.endswith('/converse'), self.path
        turn = sum(not request['summary'] for request in requests)
        text = ('ObsidianNeedle: compaction-length keyword fixture for searching this conversation.'
                if summary else f'Native embedded Goose answer {turn}, produced by the local fixture.')
        response = json.dumps({'output': {'message': {'role': 'assistant', 'content': [{'text': text}]}},
                              'stopReason': 'end_turn', 'usage': {'inputTokens': 3, 'outputTokens': 4, 'totalTokens': 7},
                              'metrics': {'latencyMs': 1}}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(response)))
        self.end_headers()
        self.wfile.write(response)


server = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}'
rally = None
if args.navigation:
    fixture_path = Path(__file__).with_name('windows-rally-smoke.py')
    spec = importlib.util.spec_from_file_location('rally_fixture', fixture_path)
    fixture_module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture_module)
    rally = fixture_module.RallyFixture()
    threading.Thread(target=rally.server.serve_forever, daemon=True).start()
    (out / 'home/rally.json').write_text(json.dumps({
        'rallyEndpoint': rally.base, 'rallyWorkspace': rally.workspace['_ref'],
        'rallyProject': rally.project['_ref'], 'projectChildren': True,
    }), encoding='utf-8')
steps = [{'wait_ready': 30000}, {'resize': [1440, 1000]},
         {'new_thread': str(out / 'project')}, {'wait_idle': 30000}]
for turn in range(1, args.turns + 1):
    steps += [{'send': f'Please give a short fixture answer {turn}'}, {'wait_idle': 30000}, {'wait': 2000}]
steps += [{'transcript': 'export-complete:' + str(out / 'complete.md')}, {'wait': 300},
         {'sidebar': ['search', 'OBSIDIANNEEDLE']}, {'wait': 1500},
         {'sidebar': ['dump', str(out / 'matches.json')]}, {'snapshot': str(out / 'search.png')},
         {'sidebar': ['search', 'absent-very-unique-keyword']}, {'wait': 1000},
         {'sidebar': ['dump', str(out / 'empty.json')]},
         {'sidebar': ['search', '']}, {'wait': 500}]
if args.navigation:
    steps += [{'server': 'fixture-failed'}, {'wait': 200}, {'click_text': 'Rally'}, {'wait_rally': {'rows': 1, 'tabs': 1, 'timeout_ms': 15000}},
              {'rally': ['item', rally.story['_ref']]},
              {'wait_rally': {'detail': True, 'timeout_ms': 15000}},
              {'rally': ['edit', 'Name', 'Keep this unsaved draft']},
              {'rally': ['open', 'tasks']}, {'wait_rally': {'rows': 1, 'tabs': 2, 'timeout_ms': 15000}},
              {'sidebar': ['dump', str(out / 'rally-two.json')]}, {'snapshot': str(out / 'rally.png')},
              {'tab_action': 'close-others'}, {'wait': 200},
              {'sidebar': ['dump', str(out / 'rally-bulk-blocked.json')]},
              {'select_tab': 1}, {'wait': 100},
              {'rally': ['dump', str(out / 'rally-kept-draft.json')]},
              {'rally': ['action', 'detail-back']}, {'rally': ['dirty-choice', 'discard']},
              {'select_tab': 2}, {'wait': 100},
              {'tab_action': 'close-others'}, {'wait': 200},
              {'sidebar': ['dump', str(out / 'rally-bulk-closed.json')]},
              {'rally': ['open', 'teamboard']}, {'wait_rally': {'rows': 1, 'tabs': 2, 'timeout_ms': 15000}},
              {'sidebar': ['dump', str(out / 'rally-two-final.json')]},
              {'select_tab': 0}, {'wait': 300},
              {'sidebar': ['dump', str(out / 'conversation-nav.json')]},
              {'snapshot': str(out / 'conversation.png')},
              {'click_text': 'Rally'}, {'wait': 300},
              {'sidebar': ['dump', str(out / 'rally-return.json')]},
              {'rally': ['close']}, {'wait': 100}, {'rally': ['close']}, {'wait': 100},
              {'sidebar': ['dump', str(out / 'rally-closed.json')]},
              {'click_text': 'Rally'}, {'wait_rally': {'rows': 1, 'tabs': 1, 'timeout_ms': 15000}},
              {'sidebar': ['dump', str(out / 'rally-reopened.json')]},
              {'rally': ['close']}, {'wait': 100}, {'server': 'fixture-ready'}]
if args.windows:
    child_steps = [{'wait_ready': 30000}, {'resize': [1200, 850]},
                   {'wait_rally': {'detail': True, 'tabs': 1, 'timeout_ms': 15000}},
                   {'wait': 1500}, {'rally': ['dump', str(out / 'child.json')]},
                   {'snapshot': str(out / 'child.png')},
                   {'new_thread': str(out / 'project')}, {'wait_idle': 30000},
                   {'send': 'Please answer from the second native window'},
                   {'wait_idle': 30000}, {'wait': 2000},
                   {'transcript': 'export-complete:' + str(out / 'child.md')},
                   {'sidebar': ['search', 'OBSIDIANNEEDLE']}, {'wait': 1500},
                   {'sidebar': ['dump', str(out / 'child-search.json')]},
                   {'select_tab': 0}, {'wait': 300}, {'rally': ['transfer-first']},
                   {'wait_rally': {'tabs': 0, 'timeout_ms': 15000}}]
    (out / 'child-script.json').write_text(json.dumps(child_steps), encoding='utf-8')
    steps += [{'click_text': 'Rally'}, {'wait_rally': {'rows': 1, 'tabs': 1, 'timeout_ms': 15000}},
              {'rally': ['item', rally.story['_ref']]},
              {'wait_rally': {'detail': True, 'timeout_ms': 15000}},
              {'rally': ['edit', 'Name', 'Draft across native windows']},
              {'rally': ['set', 'filter-value', 'Unapplied window filter']}, {'wait': 1500},
              {'click_text': 'New window'}, {'wait_rally': {'tabs': 0, 'timeout_ms': 15000}},
              {'wait_rally': {'detail': True, 'tabs': 1, 'timeout_ms': 30000}},
              {'rally': ['dump', str(out / 'returned.json')]},
              {'snapshot': str(out / 'returned.png')},
              {'rally': ['action', 'detail-back']}, {'rally': ['dirty-choice', 'discard']},
              {'rally': ['close']}, {'wait': 500}]
steps += [{'open_settings': 'goose'}, {'wait': 1000}, {'snapshot': str(out / 'settings.png')},
         {'click_text': 'Save cheap profile'}, {'wait': 500}, {'quit': True}]
(out / 'script.json').write_text(json.dumps(steps), encoding='utf-8')
env = dict(os.environ)
for key in ('AWS_PROFILE', 'AWS_SESSION_TOKEN', 'AWS_BEARER_TOKEN_BEDROCK', 'GOOSE_PROVIDER',
            'GOOSE_MODEL', 'FASTROCK_GOOSE_CONFIG', 'FASTROCK_GOOSE', 'FASTROCK_GOOSE_FIXTURE',
            'FASTROCK_FIXTURE_ARTIFACT', 'FASTROCK_RALLY_TOKEN', 'FASTROCK_RALLY_CHILD_AUTOMATION'):
    env.pop(key, None)
env.update(FASTROCK_HOME=str(out / 'home'), CODEX_HOME=str(out / 'codex'),
           GOOSE_PATH_ROOT=str(out / 'goose'), GOOSE_DISABLE_KEYRING='1',
           LOCALAPPDATA=str(out / 'local'), APPDATA=str(out / 'roaming'),
           AWS_CONFIG_FILE=str(out / 'aws-empty'), AWS_SHARED_CREDENTIALS_FILE=str(out / 'aws-empty'),
           AWS_REGION='us-east-1', AWS_ACCESS_KEY_ID='fixture-access', AWS_SECRET_ACCESS_KEY='fixture-secret',
           AWS_EC2_METADATA_DISABLED='true', AWS_ENDPOINT_URL_BEDROCK_RUNTIME=endpoint,
           AWS_ENDPOINT_URL_BEDROCK=endpoint, AWS_ENDPOINT_URL=endpoint,
           CODEX_GUI_AUTOMATION=str(out / 'script.json'), SLINT_BACKEND='winit-software')

if args.windows:
    env['FASTROCK_RALLY_CHILD_AUTOMATION'] = str(out / 'child-script.json')

if rally is not None:
    env['FASTROCK_RALLY_TOKEN'] = fixture_module.TOKEN


class Memory(ctypes.Structure):
    _fields_ = [('cb', wintypes.DWORD), ('PageFaultCount', wintypes.DWORD)] + [
        (name, ctypes.c_size_t) for name in ('PeakWorkingSetSize', 'WorkingSetSize', 'QuotaPeakPagedPoolUsage',
        'QuotaPagedPoolUsage', 'QuotaPeakNonPagedPoolUsage', 'QuotaNonPagedPoolUsage',
        'PagefileUsage', 'PeakPagefileUsage', 'PrivateUsage')]


memory_info = ctypes.WinDLL('psapi').GetProcessMemoryInfo
memory_info.argtypes = [wintypes.HANDLE, ctypes.POINTER(Memory), wintypes.DWORD]
memory_info.restype = wintypes.BOOL
peak = 0
peak_windows = 0
owners_seen = {}
user32 = ctypes.WinDLL('user32')
window_callback = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
user32.EnumWindows.argtypes = [window_callback, wintypes.LPARAM]
user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
user32.IsWindowVisible.argtypes = [wintypes.HWND]

def visible_windows(pid):
    windows = []
    @window_callback
    def collect(hwnd, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and user32.IsWindowVisible(hwnd):
            windows.append(hwnd)
        return True
    user32.EnumWindows(collect, 0)
    return len(windows)

with (out / 'app.log').open('w', encoding='utf-8') as log:
    process = subprocess.Popen([str(args.binary.resolve()), '--remote', 'goose'], cwd=out,
                               env=env, stdout=log, stderr=log)
    try:
        deadline = time.monotonic() + 90
        while process.poll() is None:
            memory = Memory()
            memory.cb = ctypes.sizeof(memory)
            if not memory_info(int(process._handle), ctypes.byref(memory), memory.cb):
                if process.poll() is not None:
                    break
                raise OSError('Could not monitor fixture memory')
            peak = max(peak, memory.PeakWorkingSetSize, memory.PeakPagefileUsage, memory.PrivateUsage)
            # This one-conversation smoke needs far less than the full 32 GB
            # benchmark allowance; leave additional headroom on the VM host.
            if peak >= 2_000_000_000 or time.monotonic() > deadline:
                raise RuntimeError('Fixture memory/time limit exceeded')
            if args.windows:
                peak_windows = max(peak_windows, visible_windows(process.pid))
                for path in (out / 'home/windows').glob('*/owner.json'):
                    try:
                        owner = json.loads(path.read_text())
                        owners_seen[owner['id']] = owner['pid']
                    except (OSError, ValueError):
                        pass  # An atomic retirement may race this read.
            time.sleep(0.1)
        assert process.returncode == 0, f'App exited {process.returncode}; see app.log'
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=10)
        server.shutdown()
        if rally is not None:
            rally.server.shutdown()

matches = json.loads((out / 'matches.json').read_text())['rows']
assert len(matches) == 1 and matches[0]['id'].startswith('goose:'), matches
assert json.loads((out / 'empty.json').read_text())['rows'] == []
assert len(requests) == 2 * (args.turns + int(args.windows)), f'Search or settings caused extra model calls: {requests}'
for ordinary, summary in zip(requests[::2], requests[1::2]):
    assert ordinary['summary'] is False and 'openai.gpt-6.1-sol' in ordinary['path']
    assert summary['summary'] is True and 'openai.gpt-6-luna' in summary['path']
transcript = (out / 'complete.md').read_text(encoding='utf-8')
for turn in range(1, args.turns + 1):
    assert transcript.count(f'Please give a short fixture answer {turn}') == 1, transcript
    assert transcript.count(f'Native embedded Goose answer {turn},') == 1, transcript
assert 'ObsidianNeedle' not in transcript, 'Search summary leaked into the source history'
saved = (out / 'goose/config/config.yaml').read_text(encoding='utf-8')
assert 'FASTROCK_CHEAP_PROFILE:' in saved and 'Bedrock Luna' in saved
assert (out / 'local/vwork/workspaces.redb').exists()
if args.navigation:
    navigation = {name: json.loads((out / (name + '.json')).read_text())['navigation']
                  for name in ('rally-two', 'rally-bulk-blocked', 'rally-bulk-closed', 'rally-two-final', 'conversation-nav', 'rally-return', 'rally-closed', 'rally-reopened')}
    assert 'server failed \"Fixture backend unavailable\"' in (out / 'app.log').read_text(encoding='utf-8')
    assert navigation['rally-two']['rallySelected'] is True, navigation
    assert navigation['conversation-nav']['rallySelected'] is False, navigation
    assert navigation['rally-bulk-blocked']['rallyCount'] == 2, navigation
    draft = json.loads((out / 'rally-kept-draft.json').read_text())
    assert draft['dirty'] and any(field['name'] == 'Name' and field['value'] == 'Keep this unsaved draft'
                                  for field in draft['fields']), draft
    assert navigation['rally-bulk-closed']['rallyCount'] == 1, navigation
    assert navigation['rally-two']['rallyCount'] == 2, navigation
    assert navigation['rally-two']['documentKinds'] == ['Rally', 'Rally'], navigation
    assert navigation['conversation-nav']['activeKind'] == 'Thread', navigation
    assert navigation['conversation-nav']['documentKinds'] == [], navigation
    assert navigation['rally-return']['activeId'] == navigation['rally-two-final']['activeId'], navigation
    assert navigation['rally-closed']['rallyCount'] == 0, navigation
    assert navigation['rally-reopened']['rallyCount'] == 1, navigation
    assert navigation['rally-reopened']['activeKind'] == 'Rally', navigation
    assert all(request['method'] == 'GET' for request in rally.requests), rally.requests
if args.windows:
    child = json.loads((out / 'child.json').read_text())
    returned = json.loads((out / 'returned.json').read_text())
    for document in (child, returned):
        assert document['dirty'], document
        assert document['filterDraft'] == 'Unapplied window filter', document
        assert any(field['name'] == 'Name' and field['value'] == 'Draft across native windows'
                   for field in document['fields']), document
    child_transcript = (out / 'child.md').read_text(encoding='utf-8')
    assert child_transcript.count('Please answer from the second native window') == 1, child_transcript
    assert f'Native embedded Goose answer {args.turns + 1},' in child_transcript, child_transcript
    assert 'Please give a short fixture answer' not in child_transcript, child_transcript
    child_hits = json.loads((out / 'child-search.json').read_text())['rows']
    assert len(child_hits) == 2, child_hits
    assert peak_windows >= 2 and len(owners_seen) == 2, (peak_windows, owners_seen)
    assert set(owners_seen.values()) == {process.pid}, owners_seen
receipt = dict(result='PASS', binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
               peak_process_bytes=peak, requests=requests, summary_only_match=True,
               exported_turns=args.turns, complete_history_preserved=True,
               native_cheap_profile_saved=True, all_provider_requests_local=True,
               independent_rally_navigation=args.navigation, backend_overlay_navigation=args.navigation,
               in_process_windows=args.windows, peak_native_windows=peak_windows, window_owner_pids=owners_seen)
(out / 'acceptance.json').write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
print(json.dumps(receipt, indent=2), flush=True)

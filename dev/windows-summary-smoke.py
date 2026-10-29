#!/usr/bin/env python3
"""V40/V44: native embedded-Goose conversation, summary search and settings.

Run on Windows only. Every provider request goes to this local HTTP fixture.
"""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
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
args = parser.parse_args()
assert 1 <= args.turns <= 100
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
steps = [{'wait_ready': 30000}, {'resize': [1440, 1000]},
         {'new_thread': str(out / 'project')}, {'wait_idle': 30000}]
for turn in range(1, args.turns + 1):
    steps += [{'send': f'Please give a short fixture answer {turn}'}, {'wait_idle': 30000}, {'wait': 2000}]
steps += [{'transcript': 'export-complete:' + str(out / 'complete.md')}, {'wait': 300},
         {'sidebar': ['search', 'OBSIDIANNEEDLE']}, {'wait': 1500},
         {'sidebar': ['dump', str(out / 'matches.json')]}, {'snapshot': str(out / 'search.png')},
         {'sidebar': ['search', 'absent-very-unique-keyword']}, {'wait': 1000},
         {'sidebar': ['dump', str(out / 'empty.json')]},
         {'open_settings': 'goose'}, {'wait': 1000}, {'snapshot': str(out / 'settings.png')},
         {'click_text': 'Save cheap profile'}, {'wait': 500}, {'quit': True}]
(out / 'script.json').write_text(json.dumps(steps), encoding='utf-8')
env = dict(os.environ)
for key in ('AWS_PROFILE', 'AWS_SESSION_TOKEN', 'AWS_BEARER_TOKEN_BEDROCK', 'GOOSE_PROVIDER',
            'GOOSE_MODEL', 'FASTROCK_GOOSE_CONFIG', 'FASTROCK_GOOSE', 'FASTROCK_GOOSE_FIXTURE',
            'FASTROCK_FIXTURE_ARTIFACT', 'FASTROCK_RALLY_TOKEN'):
    env.pop(key, None)
env.update(FASTROCK_HOME=str(out / 'home'), CODEX_HOME=str(out / 'codex'),
           GOOSE_PATH_ROOT=str(out / 'goose'), GOOSE_DISABLE_KEYRING='1',
           LOCALAPPDATA=str(out / 'local'), APPDATA=str(out / 'roaming'),
           AWS_CONFIG_FILE=str(out / 'aws-empty'), AWS_SHARED_CREDENTIALS_FILE=str(out / 'aws-empty'),
           AWS_REGION='us-east-1', AWS_ACCESS_KEY_ID='fixture-access', AWS_SECRET_ACCESS_KEY='fixture-secret',
           AWS_EC2_METADATA_DISABLED='true', AWS_ENDPOINT_URL_BEDROCK_RUNTIME=endpoint,
           AWS_ENDPOINT_URL_BEDROCK=endpoint, AWS_ENDPOINT_URL=endpoint,
           CODEX_GUI_AUTOMATION=str(out / 'script.json'), SLINT_BACKEND='winit-software')


class Memory(ctypes.Structure):
    _fields_ = [('cb', wintypes.DWORD), ('PageFaultCount', wintypes.DWORD)] + [
        (name, ctypes.c_size_t) for name in ('PeakWorkingSetSize', 'WorkingSetSize', 'QuotaPeakPagedPoolUsage',
        'QuotaPagedPoolUsage', 'QuotaPeakNonPagedPoolUsage', 'QuotaNonPagedPoolUsage',
        'PagefileUsage', 'PeakPagefileUsage', 'PrivateUsage')]


memory_info = ctypes.WinDLL('psapi').GetProcessMemoryInfo
memory_info.argtypes = [wintypes.HANDLE, ctypes.POINTER(Memory), wintypes.DWORD]
memory_info.restype = wintypes.BOOL
peak = 0
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
            time.sleep(0.1)
        assert process.returncode == 0, f'App exited {process.returncode}; see app.log'
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=10)
        server.shutdown()

matches = json.loads((out / 'matches.json').read_text())['rows']
assert len(matches) == 1 and matches[0]['id'].startswith('goose:'), matches
assert json.loads((out / 'empty.json').read_text())['rows'] == []
assert len(requests) == 2 * args.turns, f'Search or settings caused extra model calls: {requests}'
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
receipt = dict(result='PASS', binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
               peak_process_bytes=peak, requests=requests, summary_only_match=True,
               exported_turns=args.turns, complete_history_preserved=True,
               native_cheap_profile_saved=True, all_provider_requests_local=True)
(out / 'acceptance.json').write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
print(json.dumps(receipt, indent=2), flush=True)

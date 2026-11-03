#!/usr/bin/env python3
"""V86: real Goose shell tools and hooks must not open console windows."""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import threading
import time
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert os.name == 'nt', 'Native acceptance runs on Windows only'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
binary, out = a.binary.resolve(), a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
marker = out / 'console-observations.jsonl'
helper = out / 'console-observer.py'
helper.write_text('''import ctypes,json,os,sys,time
from pathlib import Path
k=ctypes.WinDLL('kernel32',use_last_error=True)
u=ctypes.WinDLL('user32',use_last_error=True)
k.GetConsoleWindow.restype=ctypes.c_void_p
u.IsWindowVisible.argtypes=[ctypes.c_void_p]
phase=sys.argv[2]
payload=json.load(sys.stdin) if phase=='hook' else None
window=k.GetConsoleWindow()
record=dict(phase=phase,event=payload.get('hook_event_name',payload.get('event')) if payload else None,pid=os.getpid(),window=window,visible=bool(window and u.IsWindowVisible(window)),payload=payload)
with Path(sys.argv[1]).open('a',encoding='utf-8') as f:f.write(json.dumps(record)+'\\n')
time.sleep(.35)
if phase=='hook':print('{"decision":"allow"}')
else:
 print('SHELL_WINDOW_FIXTURE_'+phase)
 print('SHELL_STDERR_FIXTURE',file=sys.stderr)
 if phase=='failure':sys.exit(7)
''', encoding='utf-8')


class ConsoleFixture(ProviderFixture):
    def __init__(self):
        super().__init__()
        self.rounds = 0
        self.results = []

    def respond(self, handler, body):
        if ('search index sidecar' in json.dumps(body)
                or 'Fastrock conservative memory extraction' in json.dumps(body.get('system', []))):
            return super().respond(handler, body)
        self.requests.append(dict(path=handler.path, summary=False, body=body))
        self.rounds += 1
        assert self.rounds <= 3, 'Unexpected tool loop'
        if self.rounds > 1:
            results = [c['toolResult'] for c in body['messages'][-1]['content'] if 'toolResult' in c]
            assert len(results) == 1, results
            self.results.extend(results)
        reason = 'end_turn'
        content = [{'text': 'Console fixture finished.'}]
        if self.rounds <= 2:
            names = [t['toolSpec']['name'] for t in body['toolConfig']['tools'] if 'toolSpec' in t]
            matches = [n for n in names if n == 'shell' or n.endswith(('__shell', '.shell'))]
            assert len(matches) == 1, names
            tool = matches[0]
            phase = 'success' if self.rounds == 1 else 'failure'
            command = subprocess.list2cmdline([sys.executable, str(helper), str(marker), phase])
            content = [{'toolUse': dict(toolUseId=f'console-{self.rounds}', name=tool,
                                       input={'command': command, 'timeout_secs': 10})}]
            reason = 'tool_use'
        return self.json_response(handler, dict(output={'message': {'role': 'assistant', 'content': content}},
            stopReason=reason, usage={'inputTokens': 3, 'outputTokens': 4, 'totalTokens': 7}, metrics={'latencyMs': 1}))


provider = ConsoleFixture()
env = provider.environment(out)
env.update(GOOSE_SHELL='cmd', FASTROCK_WEBHOOK_BIND='127.0.0.1:0')
config = out / 'goose/config/config.yaml'
settings = json.loads(config.read_text())
settings['extensions']['developer'] = dict(enabled=True, type='platform', name='developer')
config.write_text(json.dumps(settings), encoding='utf-8')
plugin = out / 'goose/.agents/plugins/console-fixture'
(plugin / 'hooks').mkdir(parents=True)
(plugin / 'plugin.json').write_text(json.dumps(dict(name='console-fixture', version='1.0.0')))
command = ' '.join(shlex.quote(str(x).replace('\\', '/')) for x in (sys.executable, helper, marker, 'hook'))
rules = [{'hooks': [{'type': 'command', 'command': command, 'timeout': 10, 'on_failure': 'block'}]}]
(plugin / 'hooks/hooks.json').write_text(json.dumps({'hooks': {
    event: rules for event in ('SessionStart', 'PreToolUse', 'PostToolUse', 'Stop')}}))
script = out / 'steps.json'
script.write_text(json.dumps([
    {'wait_ready': 30000}, {'new_thread': str(out / 'project')}, {'wait_idle': 30000},
    {'send': 'Run the console fixture tools'}, {'wait_idle': 30000}, {'wait_summaries': 30000},
    {'transcript': 'export-complete:' + str(out / 'transcript.md')}, {'wait': 400}, {'quit': True}]))

# Observe short-lived popups, including the managed vwork hook, not just the
# probe's own process. Restrict to console windows created during this fixture.
user = ctypes.WinDLL('user32', use_last_error=True)
user.GetClassNameW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
user.IsWindowVisible.argtypes = [wintypes.HWND]
callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
user.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]


def visible_consoles():
    found = set()
    @callback_type
    def visit(hwnd, _):
        name = ctypes.create_unicode_buffer(256)
        user.GetClassNameW(hwnd, name, len(name))
        if name.value in ('ConsoleWindowClass', 'CASCADIA_HOSTING_WINDOW_CLASS') and user.IsWindowVisible(hwnd):
            found.add(int(hwnd))
        return True
    user.EnumWindows(visit, 0)
    return found


baseline, popups, stop = visible_consoles(), set(), threading.Event()
def observe():
    while not stop.wait(.01):
        popups.update(visible_consoles() - baseline)
watcher = threading.Thread(target=observe, daemon=True)
watcher.start()
try:
    with (out / 'app.log').open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(binary)], env=dict(env, CODEX_GUI_AUTOMATION=str(script)), stdout=log, stderr=log)
        peak = ProcessGuard(process, timeout=90).wait()
    rows = [json.loads(line) for line in marker.read_text().splitlines()]
    (out / 'observations.json').write_text(json.dumps(dict(processes=rows, visible_console_windows=sorted(popups)), indent=2))
    assert not provider.errors, provider.errors
    assert provider.rounds == 3 and len(provider.results) == 2
    assert 'SHELL_WINDOW_FIXTURE_success' in json.dumps(provider.results[0]), provider.results
    assert 'Command exited with code 7' in json.dumps(provider.results[1]), provider.results
    assert {r['phase'] for r in rows} == {'hook', 'success', 'failure'}, rows
    assert len([r for r in rows if r['phase'] == 'hook']) >= 5, 'Real hook lifecycle not exercised'
    assert not any(r['visible'] for r in rows), 'Goose hook/tool inherited a visible console'
    assert not popups, f'Console popups observed: {popups}'
    transcript = (out / 'transcript.md').read_text(encoding='utf-8')
    assert all(t in transcript for t in ('SHELL_WINDOW_FIXTURE_success', 'SHELL_STDERR_FIXTURE', 'Console fixture finished.'))
    assert 'codex-gui automation:' in (out / 'app.log').read_text(encoding='utf-8'), 'Redirected diagnostic output was lost'
    receipt = dict(result='PASS', binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), peak_process_bytes=peak,
        real_goose_shell=True, real_hook_lifecycle=True, visible_console_windows=[], stdout_stderr_preserved=True,
        nonzero_exit_preserved=True, redirected_app_output_preserved=True, observations=rows)
    (out / 'acceptance.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print('PASS: real Goose shell/hooks, no console popups, output and exit status preserved', flush=True)
finally:
    stop.set()
    watcher.join(timeout=2)
    provider.close()
    provider.write_evidence(out)

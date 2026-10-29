#!/usr/bin/env python3
"""Measure published Fastrock conversation opening with a local ACP fixture.

No installed backend, inference, credentials, or external services are used.
The fixture reports pipe-delivery timing (including client backpressure), while
the GUI reports its existing thread-started/thread-resumed milestones. History
visibility is sampled through the GUI's transcript export at 100 ms intervals;
it is an upper bound, not a first-pixel or input-to-photon measurement.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import tempfile
import threading
import time


def fixture():
    project = os.environ['FASTROCK_THREAD_PROJECT']
    timing = Path(os.environ['FASTROCK_THREAD_TIMING'])
    scenario = os.environ['FASTROCK_THREAD_SCENARIO']
    serial = 0
    sessions = {'saved': project}
    config = [dict(id=key, name=key, type='select', currentValue=value,
                   options=[dict(value=value, name=value)])
              for key, value in [('provider', 'fixture'), ('model', 'fixture-model')]]

    def send(value):
        line = json.dumps(dict(jsonrpc='2.0', **value), separators=(',', ':'))
        print(line, flush=True)
        return len(line.encode('utf-8')) + 1

    def info(sid):
        return dict(sessionId=sid, cwd=sessions[sid], title='Benchmark ' + sid,
                    updatedAt='2026-10-29T12:00:00Z',
                    _meta=dict(createdAt='2026-10-29T12:00:00Z',
                               providerId='fixture', modelId='fixture-model'))

    for line in sys.stdin:
        request = json.loads(line)
        method, params = request.get('method'), request.get('params', {})
        if method is None:
            continue
        start = time.perf_counter_ns()
        replay_bytes = 0
        first_update = None
        last_update = None
        if method == 'initialize':
            result = dict(protocolVersion=1, agentInfo=dict(name='timing-fixture', version='1'),
                          agentCapabilities=dict(loadSession=True, mcpCapabilities=dict(http=True),
                                                 sessionCapabilities=dict(list={}, close={})))
        elif method == '_goose/unstable/defaults/read':
            result = dict(providerId='fixture', modelId='fixture-model')
        elif method == 'session/list':
            result = dict(sessions=[info(sid) for sid in sessions], nextCursor=None)
        elif method == '_goose/unstable/session/info':
            result = dict(session=info(params['sessionId']))
        elif method == 'session/new':
            serial += 1
            sid = 'new-' + str(serial)
            sessions[sid] = params['cwd']
            result = dict(sessionId=sid, configOptions=config)
        elif method == 'session/load':
            sid = params['sessionId']
            count = {'small': 10, 'long': 2000, 'tools': 200}[scenario]
            body = 'Markdown **history** with a [link](https://example.com). ' * 40
            for n in range(count):
                updates = [dict(sessionUpdate='user_message_chunk', messageId=f'u-{n}',
                                content=dict(type='text', text=f'Prompt {n}'))]
                if scenario == 'tools':
                    updates.append(dict(sessionUpdate='tool_call_update', toolCallId=f'tool-{n}',
                                        title='Saved tool output', status='completed',
                                        content=[dict(type='content', content=dict(type='text',
                                                      text=('tool output line\n' * 4096)))]))
                updates.append(dict(sessionUpdate='agent_message_chunk', messageId=f'a-{n}',
                                    content=dict(type='text', text=body +
                                                 (' HISTORY-END-SENTINEL' if n == count - 1 else ''))))
                for update in updates:
                    first_update = first_update or time.perf_counter_ns()
                    replay_bytes += send(dict(method='session/update',
                                              params=dict(sessionId=sid, update=update)))
                    last_update = time.perf_counter_ns()
            result = dict(configOptions=config)
        elif method in ('session/close', '_goose/unstable/session/system-prompt/set'):
            result = {}
        else:
            send(dict(id=request['id'], error=dict(code=-32601, message=method)))
            continue
        send(dict(id=request['id'], result=result))
        end = time.perf_counter_ns()
        with timing.open('a', encoding='utf-8') as log:
            log.write(json.dumps(dict(method=method, start_ns=start, end_ns=end,
                                      elapsed_ms=(end - start) / 1e6,
                                      first_update_ns=first_update, last_update_ns=last_update,
                                      replay_bytes=replay_bytes)) + '\n')


def benchmark():
    assert sys.platform == 'win32', "Run on Windows; never run GUI tests on Sean's Mac"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--repeats', type=int, default=3)
    args = parser.parse_args()
    binary = args.binary.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    receipt = dict(binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                   host=platform.node(), os=platform.platform(), backend='local immediate ACP fixture',
                   history_sampling_ms=100, measurements=[])
    # Outside the checkout: no inherited .git directory or trust prompt.
    workspace = tempfile.TemporaryDirectory(prefix='fastrock-thread-')
    for scenario in ('new', 'small', 'long', 'tools'):
        for repeat in range(args.repeats):
            folder = output / f'{scenario}-{repeat}'
            folder.mkdir()
            for name in ('home', 'codex'):
                (folder / name).mkdir()
            project = Path(workspace.name) / f'{scenario}-{repeat}'
            project.mkdir()
            timing = folder / 'rpc.jsonl'
            history = folder / 'history.md'
            operation = {'open_folder': str(project)} if scenario == 'new' else {'resume': 'goose:saved'}
            steps = [{'wait_ready': 30000}, {'resize': [1440, 1000]}, {'wait': 300},
                     {'mark': 'open-begin'}, operation, {'wait_idle': 120000}, {'mark': 'open-idle'}]
            if scenario != 'new':
                for n in range(20):
                    steps.extend([{'transcript': 'export:' + str(history)},
                                  {'mark': f'history-probe-{n}'}, {'wait': 100}])
            steps.extend([{'snapshot': str(folder / 'visible.png')}, {'quit': True}])
            script = folder / 'steps.json'
            script.write_text(json.dumps(steps), encoding='utf-8')
            env = dict(os.environ, FASTROCK_HOME=str(folder / 'home'), CODEX_HOME=str(folder / 'codex'),
                       FASTROCK_GOOSE=sys.executable, FASTROCK_GOOSE_FIXTURE=str(Path(__file__).resolve()),
                       FASTROCK_THREAD_PROJECT=str(project), FASTROCK_THREAD_TIMING=str(timing),
                       FASTROCK_THREAD_SCENARIO=scenario, CODEX_GUI_AUTOMATION=str(script),
                       CODEX_GUI_PERF='1', PYTHONUTF8='1', SLINT_BACKEND='winit-software', SLINT_SCALE_FACTOR='1')
            for key in ('FASTROCK_RALLY_TOKEN', 'FASTROCK_FIXTURE_ARTIFACT', 'SLINT_DEBUG_PERFORMANCE'):
                env.pop(key, None)
            marks = {}
            history_visible_ms = []
            pattern = re.compile(r'codex-gui perf: (\S+) ([\d.]+) ms')
            with (folder / 'app.log').open('w', encoding='utf-8') as log:
                process = subprocess.Popen([str(binary), '--remote', 'goose'], env=env,
                                           cwd=project, stdout=subprocess.PIPE,
                                           stderr=subprocess.STDOUT, text=True, encoding='utf-8', errors='replace')

                def capture():
                    for line in process.stdout:
                        log.write(line)
                        match = pattern.search(line)
                        if match:
                            name, ms = match.group(1), float(match.group(2))
                            marks[name] = ms
                            if name.startswith('history-probe') and not history_visible_ms and history.exists():
                                if 'HISTORY-END-SENTINEL' in history.read_text(encoding='utf-8'):
                                    history_visible_ms.append(ms)

                reader = threading.Thread(target=capture)
                reader.start()
                try:
                    code = process.wait(timeout=180)
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait()
                    reader.join(timeout=10)
                assert code == 0 and not reader.is_alive(), (scenario, 'process failed', code)
            rpc = [json.loads(line) for line in timing.read_text(encoding='utf-8').splitlines()]
            method = 'session/new' if scenario == 'new' else 'session/load'
            wire = next(r for r in rpc if r['method'] == method)
            milestone = 'thread-started' if scenario == 'new' else 'thread-resumed'
            assert 'open-begin' in marks and milestone in marks and 'open-idle' in marks, marks
            assert scenario == 'new' or history_visible_ms, (scenario, 'history never displayed')
            measurement = dict(scenario=scenario, repeat=repeat,
                               response_ms=round(marks[milestone] - marks['open-begin'], 2),
                               idle_observed_ms=round(marks['open-idle'] - marks['open-begin'], 2),
                               history_visible_upper_bound_ms=round(history_visible_ms[0] - marks['open-begin'], 2) if history_visible_ms else None,
                               fixture_delivery_ms=round(wire['elapsed_ms'], 2), replay_bytes=wire['replay_bytes'])
            receipt['measurements'].append(measurement)
            (output / 'summary.json').write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
            print(json.dumps(measurement), flush=True)
    workspace.cleanup()


if __name__ == '__main__':
    if sys.argv[1:] == ['acp']:
        fixture()
    else:
        benchmark()

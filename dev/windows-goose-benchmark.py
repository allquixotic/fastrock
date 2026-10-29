#!/usr/bin/env python3
"""Time installed Goose ACP on Windows with isolated config and local fixtures.

Uses the installed executable, never installs a backend, and never invokes real
inference or external tools. Each variant gets its own GOOSE_PATH_ROOT. Timings
measure protocol replies, not pixels. Run in the signed-in Windows session.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
import queue
import re
import subprocess
import sys
import threading
import time


class Fixture(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, status, body, kind='application/json'):
        data = body.encode() if isinstance(body, str) else json.dumps(body).encode()
        self.send_response(status)
        self.send_header('Content-Type', kind)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        self.server.events.append(dict(path=self.path, method='GET', at=time.perf_counter()))
        if self.path.endswith('/models'):
            self.reply(200, dict(object='list', data=[dict(id='gpt-4o', object='model',
                                                       created=0, owned_by='fixture')]))
        elif self.path.startswith('/mcp'):
            self.reply(405, {})
        else:
            self.reply(404, {})

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        method = request.get('method', 'completion')
        self.server.events.append(dict(path=self.path, method=method, at=time.perf_counter()))
        if self.path.startswith('/mcp'):
            if method == 'initialize':
                time.sleep(self.server.mcp_delay)
                result = dict(protocolVersion='2025-03-26', capabilities=dict(tools={}),
                              serverInfo=dict(name='local-startup-fixture', version='1'))
            elif method == 'tools/list':
                result = dict(tools=[])
            elif 'id' not in request:
                self.reply(202, '')
                return
            else:
                self.reply(200, dict(jsonrpc='2.0', id=request['id'],
                                    error=dict(code=-32601, message='Method not found')))
                return
            self.reply(200, dict(jsonrpc='2.0', id=request['id'], result=result))
        elif self.path.endswith('/chat/completions'):
            text = 'Local fixture response. HISTORY-END-SENTINEL'
            common = dict(id='fixture', created=0, model='gpt-4o')
            if request.get('stream'):
                chunks = [dict(**common, object='chat.completion.chunk',
                               choices=[dict(index=0, delta=dict(role='assistant', content=text),
                                             finish_reason=None)]),
                          dict(**common, object='chat.completion.chunk',
                               choices=[dict(index=0, delta={}, finish_reason='stop')])]
                self.reply(200, ''.join('data: ' + json.dumps(c) + '\n\n' for c in chunks)
                           + 'data: [DONE]\n\n', 'text/event-stream')
            else:
                self.reply(200, dict(**common, object='chat.completion',
                                    choices=[dict(index=0, message=dict(role='assistant', content=text),
                                                  finish_reason='stop')],
                                    usage=dict(prompt_tokens=10, completion_tokens=5, total_tokens=15)))
        else:
            self.reply(404, {})


class Client:
    def __init__(self, binary, env, folder):
        self.incoming = queue.Queue()
        self.stderr = (folder / 'goose-stderr.log').open('w', encoding='utf-8')
        self.wire = (folder / 'acp.jsonl').open('w', encoding='utf-8')
        self.serial = 0
        self.spawned = time.perf_counter()
        self.process = subprocess.Popen([str(binary), 'acp'], env=env, cwd=folder,
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=self.stderr, text=True, encoding='utf-8', errors='replace')
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()

    def read(self):
        for line in self.process.stdout:
            self.wire.write(line)
            self.wire.flush()
            try:
                self.incoming.put((time.perf_counter(), json.loads(line)))
            except json.JSONDecodeError:
                continue
        self.incoming.put((time.perf_counter(), None))

    def request(self, method, params):
        self.serial += 1
        start = time.perf_counter()
        self.process.stdin.write(json.dumps(dict(jsonrpc='2.0', id=self.serial,
                                                method=method, params=params)) + '\n')
        self.process.stdin.flush()
        updates, history = [], []
        while True:
            at, message = self.incoming.get(timeout=max(0.1, 90 - (time.perf_counter() - start)))
            assert message is not None, 'Goose exited; see goose-stderr.log'
            if message.get('id') == self.serial and 'method' not in message:
                assert 'error' not in message, (method, message)
                timing = dict(method=method, elapsed_ms=round((at - start) * 1000, 2),
                              updates=len(updates),
                              first_update_ms=round((updates[0] - start) * 1000, 2) if updates else None,
                              last_update_ms=round((updates[-1] - start) * 1000, 2) if updates else None,
                              first_history_ms=round((history[0] - start) * 1000, 2) if history else None,
                              last_history_ms=round((history[-1] - start) * 1000, 2) if history else None)
                return message['result'], timing
            if message.get('method') == 'session/update' and at >= start:
                updates.append(at)
                event = message.get('params', {})
                if (event.get('sessionId') == params.get('sessionId') and
                        event.get('update', {}).get('sessionUpdate') in ('user_message_chunk', 'agent_message_chunk')):
                    history.append(at)
            if 'id' in message and 'method' in message:
                self.process.stdin.write(json.dumps(dict(jsonrpc='2.0', id=message['id'],
                    error=dict(code=-32601, message='Not supported by benchmark'))) + '\n')
                self.process.stdin.flush()

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            subprocess.run(['taskkill', '/PID', str(self.process.pid), '/T', '/F'],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
            self.process.wait(timeout=10)
        self.reader.join(timeout=5)
        assert not self.reader.is_alive(), 'Goose stdout remained open'
        self.stderr.close()
        self.wire.close()


def gui_sample(binary, goose, env, folder, session_id, resume):
    folder.mkdir()
    (folder / 'home').mkdir()
    (folder / 'codex').mkdir()
    history = folder / 'history.md'
    steps = [dict(wait_ready=30000), dict(resize=[1440, 1000]), dict(wait=300),
             dict(mark='open-begin'),
             dict(resume='goose:' + session_id) if resume else dict(open_folder=str(folder.parent / 'project')),
             dict(wait_idle=120000), dict(mark='open-idle')]
    if resume:
        for n in range(20):
            steps.extend([dict(transcript='export:' + str(history)),
                          dict(mark=f'history-probe-{n}'), dict(wait=100)])
    steps.extend([dict(snapshot=str(folder / 'visible.png')), dict(quit=True)])
    script = folder / 'steps.json'
    script.write_text(json.dumps(steps), encoding='utf-8')
    env = dict(env, FASTROCK_HOME=str(folder / 'home'), CODEX_HOME=str(folder / 'codex'),
               FASTROCK_GOOSE=str(goose), CODEX_GUI_AUTOMATION=str(script), CODEX_GUI_PERF='1',
               SLINT_BACKEND='winit-software', SLINT_SCALE_FACTOR='1')
    for key in ('FASTROCK_GOOSE_FIXTURE', 'FASTROCK_RALLY_TOKEN', 'SLINT_DEBUG_PERFORMANCE'):
        env.pop(key, None)
    marks, available = {}, []
    with (folder / 'app.log').open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(binary), '--remote', 'goose'], env=env, cwd=folder,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   text=True, encoding='utf-8', errors='replace')

        def capture():
            for line in process.stdout:
                log.write(line)
                match = re.search(r'codex-gui perf: (\S+) ([\d.]+) ms', line)
                if match:
                    name, ms = match[1], float(match[2])
                    marks[name] = ms
                    if name.startswith('history-probe') and not available and history.exists():
                        if 'HISTORY-END-SENTINEL' in history.read_text(encoding='utf-8'):
                            available.append(ms)

        reader = threading.Thread(target=capture, daemon=True)
        reader.start()
        try:
            code = process.wait(timeout=180)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            reader.join(timeout=10)
        assert code == 0 and not reader.is_alive(), ('GUI failed', folder)
    milestone = 'thread-resumed' if resume else 'thread-started'
    assert all(k in marks for k in ('open-begin', 'open-idle', milestone)), marks
    assert not resume or available, 'Seeded conversation content was not available'
    return dict(operation='resume' if resume else 'new',
                response_ms=round(marks[milestone] - marks['open-begin'], 2),
                idle_observed_ms=round(marks['open-idle'] - marks['open-begin'], 2),
                history_available_upper_bound_ms=round(available[0] - marks['open-begin'], 2) if available else None)


def main():
    assert sys.platform == 'win32', 'Run on Windows only'
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--repeats', type=int, default=3)
    parser.add_argument('--variants', nargs='+', default=['default', 'developer', 'mcp', 'mcp-delayed'])
    parser.add_argument('--fastrock', type=Path, help='Also measure native GUI (MCP variants: resume only)')
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    server = ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
    server.events = []
    server.mcp_delay = 0
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    base = f'http://127.0.0.1:{server.server_port}'
    receipt = dict(host=platform.node(), os=platform.platform(), executable=str(binary),
                   sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                   version=subprocess.check_output([str(binary), '--version'], text=True).strip(),
                   provider='local HTTP fixture; no real inference', measurements=[])
    if args.fastrock:
        args.fastrock = args.fastrock.resolve()
        receipt['fastrock_sha256'] = hashlib.sha256(args.fastrock.read_bytes()).hexdigest()
    try:
        for variant in args.variants:
            assert variant in ('default', 'developer', 'mcp', 'mcp-delayed')
            server.mcp_delay = 1 if variant == 'mcp-delayed' else 0
            for repeat in range(args.repeats):
                folder = output / f'{variant}-{repeat}'
                (folder / 'root' / 'config').mkdir(parents=True)
                (folder / 'project').mkdir()
                config = 'GOOSE_PROVIDER: openai\nGOOSE_MODEL: gpt-4o\nGOOSE_MODE: auto\n'
                config += 'GOOSE_TOOL_PAIR_SUMMARIZATION: false\n'
                config += ('extensions:\n  developer:\n    enabled: true\n    type: builtin\n'
                           '    name: developer\n    description: Local developer tools\n'
                           if variant == 'developer' else 'extensions: {}\n')
                (folder / 'root' / 'config' / 'config.yaml').write_text(config, encoding='utf-8')
                env = dict(os.environ, GOOSE_PATH_ROOT=str(folder / 'root'),
                           GOOSE_PROVIDER='openai', GOOSE_MODEL='gpt-4o',
                           OPENAI_API_KEY='local-fixture-not-a-secret', OPENAI_BASE_URL=base + '/v1',
                           OPENAI_HOST=base, OPENAI_BASE_PATH='v1/chat/completions',
                           GOOSE_DISABLE_SESSION_NAMING='true', NO_PROXY='127.0.0.1,localhost')
                servers = [dict(type='http', name='fixture', url=base + '/mcp', headers=[])] if variant.startswith('mcp') else []
                client = Client(binary, env, folder)
                row = dict(variant=variant, repeat=repeat, requests=[])
                http_start = len(server.events)
                try:
                    _, timing = client.request('initialize', dict(protocolVersion=1, clientCapabilities={}))
                    row['spawn_to_initialized_ms'] = round((time.perf_counter() - client.spawned) * 1000, 2)
                    row['requests'].append(timing)
                    for cycle in range(2):
                        result, timing = client.request('session/new', dict(cwd=str(folder / 'project'), mcpServers=servers))
                        sid = result['sessionId']
                        extension_results = result.get('_meta', {}).get('extensionResults', [])
                        assert extension_results and all(e['success'] for e in extension_results), extension_results
                        row.setdefault('extension_results', []).append(extension_results)
                        row['requests'].append(dict(timing, cycle=cycle))
                        extensions, _ = client.request('_goose/unstable/session/extensions/list', dict(sessionId=sid))
                        row.setdefault('extensions', []).append(extensions)
                        _, timing = client.request('session/prompt', dict(sessionId=sid, prompt=[dict(type='text', text='Fixture prompt')]))
                        row['requests'].append(dict(timing, cycle=cycle))
                        _, timing = client.request('session/close', dict(sessionId=sid))
                        row['requests'].append(dict(timing, cycle=cycle))
                        loaded, timing = client.request('session/load', dict(sessionId=sid, cwd=str(folder / 'project'), mcpServers=servers))
                        load_extensions = loaded.get('_meta', {}).get('extensionResults', [])
                        assert all(e['success'] for e in load_extensions), load_extensions
                        assert timing['last_history_ms'] is not None, 'No seeded history replayed'
                        row['requests'].append(dict(timing, cycle=cycle))
                        client.request('session/close', dict(sessionId=sid))
                finally:
                    client.close()
                if servers:
                    initialized = sum(e['method'] == 'initialize' for e in server.events[http_start:])
                    listed = sum(e['method'] == 'tools/list' for e in server.events[http_start:])
                    # Goose can reuse the persisted tool catalog on reload.
                    assert initialized == 4 and listed >= 2, (initialized, listed)
                if args.fastrock:
                    operations = [('gui-resume', True)] if servers else [('gui-new', False), ('gui-resume', True)]
                    row['gui'] = [gui_sample(args.fastrock, binary, env, folder / name, sid, resume)
                                  for name, resume in operations]
                receipt['measurements'].append(row)
                (output / 'summary.json').write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
                print(json.dumps({k: v for k, v in row.items() if k not in ('extensions', 'extension_results')}), flush=True)
    finally:
        (output / 'http.json').write_text(json.dumps(server.events, indent=2) + '\n', encoding='utf-8')
        server.shutdown()
        server.server_close()


if __name__ == '__main__':
    main()

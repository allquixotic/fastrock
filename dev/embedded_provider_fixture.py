"""Loopback Bedrock/MCP fixtures for the real embedded Goose runtime.

No ACP implementation, installed backend, user configuration, or live credentials.
The optional MCP endpoint is a configured test tool, not an agent service.
"""
import ctypes
from ctypes import wintypes
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import struct
import threading
import time
import zlib


def event_frame(kind, payload):
    headers = b''
    for name, value in ((':message-type', 'event'), (':event-type', kind),
                        (':content-type', 'application/json')):
        name, value = name.encode(), value.encode()
        headers += bytes([len(name)]) + name + b'\x07' + struct.pack('!H', len(value)) + value
    body = json.dumps(payload, ensure_ascii=False).encode()
    prelude = struct.pack('!II', 16 + len(headers) + len(body), len(headers))
    message = prelude + struct.pack('!I', zlib.crc32(prelude)) + headers + body
    return message + struct.pack('!I', zlib.crc32(message))


class ProviderFixture:
    def __init__(self, artifact=None, tools=False, workspace_repo=None, stream_chunks=1000):
        self.artifact, self.tools = artifact, tools
        self.stream_chunks = stream_chunks
        self.stream_lock = threading.Lock()
        self.active_streams = self.peak_streams = 0
        self.workspace_repo = workspace_repo
        self.workspace_steps, self.workspace_results = {}, []
        self.requests, self.tool_calls, self.errors = [], [], []
        self.stop = threading.Event()
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                self.send_error(405)

            def do_DELETE(self):
                self.send_response(204)
                self.end_headers()

            def do_POST(self):
                self.tracked_stream = False
                try:
                    length = int(self.headers['Content-Length'])
                    assert length < 64 * 1024 * 1024, 'Oversized fixture request'
                    body = json.loads(self.rfile.read(length))
                    if self.path == '/mcp':
                        owner.mcp(self, body)
                    else:
                        owner.respond(self, body)
                except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                    pass  # Cancellation deliberately closes an active stream.
                except Exception as error:
                    owner.errors.append(repr(error))
                    self.send_error(500, 'Fixture rejected request')
                finally:
                    if self.tracked_stream:
                        with owner.stream_lock:
                            owner.active_streams -= 1

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.endpoint = f'http://127.0.0.1:{self.server.server_port}'
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def environment(self, directory, streaming=False):
        root = Path(directory).resolve()
        for name in ('home', 'codex', 'project', 'goose/config', 'local', 'roaming'):
            (root / name).mkdir(parents=True, exist_ok=True)
        (root / 'aws-empty').write_text('', encoding='utf-8')
        extensions = {}
        if self.tools:
            extensions['fixture'] = dict(enabled=True, type='streamable_http', name='fixture',
                                         uri=self.endpoint + '/mcp', timeout=30, description='Local test tools')
        config = dict(GOOSE_PROVIDER='aws_bedrock', GOOSE_MODEL='openai.gpt-6.1-sol',
                      GOOSE_MODE='auto', BEDROCK_DISABLE_STREAMING=not streaming,
                      BEDROCK_MAX_RETRIES=0, GOOSE_DISABLE_SESSION_NAMING=True, extensions=extensions)
        (root / 'goose/config/config.yaml').write_text(json.dumps(config), encoding='utf-8')
        env = {key: value for key, value in os.environ.items()
               if not key.upper().startswith(('AWS_', 'GOOSE_', 'FASTROCK_', 'OPENAI_', 'ANTHROPIC_', 'BEDROCK_'))}
        env.update(FASTROCK_HOME=str(root / 'home'), CODEX_HOME=str(root / 'codex'),
                   GOOSE_PATH_ROOT=str(root / 'goose'), GOOSE_DISABLE_KEYRING='1',
                   LOCALAPPDATA=str(root / 'local'), APPDATA=str(root / 'roaming'),
                   AWS_CONFIG_FILE=str(root / 'aws-empty'), AWS_SHARED_CREDENTIALS_FILE=str(root / 'aws-empty'),
                   AWS_REGION='us-east-1', AWS_ACCESS_KEY_ID='fixture-access', AWS_SECRET_ACCESS_KEY='fixture-secret',
                   AWS_EC2_METADATA_DISABLED='true', AWS_ENDPOINT_URL_BEDROCK_RUNTIME=self.endpoint,
                   AWS_ENDPOINT_URL_BEDROCK=self.endpoint, AWS_ENDPOINT_URL=self.endpoint,
                   NO_PROXY='127.0.0.1,localhost', no_proxy='127.0.0.1,localhost', SLINT_BACKEND='winit-software')
        return env

    @staticmethod
    def json_response(handler, value, status=200):
        body = json.dumps(value, ensure_ascii=False).encode()
        handler.send_response(status)
        handler.send_header('Content-Type', 'application/json')
        handler.send_header('Content-Length', str(len(body)))
        handler.end_headers()
        handler.wfile.write(body)

    def mcp(self, handler, body):
        method = body['method']
        if 'id' not in body:
            return self.json_response(handler, {}, 202)
        if method == 'initialize':
            result = dict(protocolVersion=body['params']['protocolVersion'], capabilities={'tools': {}},
                          serverInfo={'name': 'fastrock-fixture-tools', 'version': '1'})
        elif method == 'tools/list':
            result = {'tools': [dict(name=name, description=name, inputSchema=schema,
                                    annotations={'readOnlyHint': True, 'destructiveHint': False}) for name, schema in (
                ('selection_tool', {'type': 'object', 'properties': {'command': {'type': 'string'}}}),
                ('benchmark_tool', {'type': 'object', 'properties': {'fixture': {'type': 'integer'}}}),
            )]}
        elif method == 'tools/call':
            call = body['params']
            self.tool_calls.append(call)
            if call['name'] == 'selection_tool':
                text = '\n'.join(f'Selection output {i:02d}: **literal** <tag> é 👩‍💻' for i in range(24))
            else:
                assert call['name'] == 'benchmark_tool', call
                n = call['arguments']['fixture']
                text = '\n'.join(f'Output {n}/{i}: line of tool text with preserved source order' for i in range(200))
            result = {'content': [{'type': 'text', 'text': text}], 'isError': False}
        elif method == 'ping':
            result = {}
        else:
            return self.json_response(handler, dict(jsonrpc='2.0', id=body['id'],
                                                   error={'code': -32601, 'message': 'Unsupported fixture method'}))
        self.json_response(handler, dict(jsonrpc='2.0', id=body['id'], result=result))

    def respond(self, handler, body):
        assert handler.path.startswith('/model/') and handler.path.endswith(('/converse', '/converse-stream')), handler.path
        assert len(self.requests) < 512, 'Provider request loop'
        messages = body['messages']
        prompt = next((' '.join(c.get('text', '') for c in message['content'])
                       for message in reversed(messages) if message['role'] == 'user'
                       and any('text' in c for c in message['content'])), '')
        prompt = prompt.split('<turn-context>', 1)[0].strip()
        summary = 'search index sidecar' in json.dumps(body)
        self.requests.append(dict(path=handler.path, summary=summary, prompt=prompt, body=body))
        serial = len(self.requests)
        content, reason = [], 'end_turn'
        tool_result = any('toolResult' in c for c in messages[-1]['content'])
        names = [t['toolSpec']['name'] for t in body.get('toolConfig', {}).get('tools', []) if 'toolSpec' in t]

        def tool(suffix, arguments):
            matches = [name for name in names if name.endswith(suffix)]
            assert len(matches) == 1, (suffix, names)
            return {'toolUse': dict(toolUseId=f'fixture-{serial}-{len(content)}', name=matches[0], input=arguments)}

        if not summary and self.workspace_repo and prompt.startswith('vwork-'):
            result = None
            if tool_result:
                results = [c['toolResult'] for c in messages[-1]['content'] if 'toolResult' in c]
                assert all(r.get('status') != 'error' for r in results), results
                text = '\n'.join(c.get('text', '') for r in results for c in r['content'])
                result = json.loads(text)
                self.workspace_results.append(dict(prompt=prompt, result=result))
            step = self.workspace_steps.get(prompt, 0)
            self.workspace_steps[prompt] = step + 1
            operation = None
            if prompt == 'vwork-seed':
                if step == 0:
                    operation = ('repo_register', dict(repo='mono', path=str(self.workspace_repo)))
                elif step == 1:
                    operation = ('workspace_create', dict(repo='mono', workspace='task'))
            elif prompt == 'vwork-conflict':
                if step == 0:
                    operation = ('files_read', dict(workspace='task', path='src/main.rs'))
                elif step == 1:
                    operation = ('files_write', dict(workspace='task', path='src/main.rs',
                        expected_revision=result['revision'], content='Concurrent tool edit\r\n', mode='100755'))
            elif prompt == 'vwork-inspect':
                if step == 0:
                    operation = ('files_read', dict(workspace='task', path='src/main.rs', binary=True))
                elif step == 1:
                    operation = ('files_stat', dict(workspace='task', paths=['src/main.rs']))
            elif prompt in ('vwork-bound-parent', 'vwork-parent-restored', 'vwork-child-restored', 'vwork-project-import'):
                if step == 0:
                    operation = ('files_read', dict(path='src/main.rs', binary=True))
            elif prompt == 'vwork-child-edit':
                if step == 0:
                    operation = ('files_read', dict(path='src/main.rs'))
                elif step == 1:
                    operation = ('files_write', dict(path='src/main.rs', expected_revision=result['revision'],
                        content='fork isolated λ\r\n', mode='100755'))
                elif step == 2:
                    operation = ('files_read', dict(path='src/main.rs', binary=True))
            if operation:
                content.append(tool(*operation))
        elif not summary and not tool_result:
            if self.artifact and 'Suggest a title change' in prompt:
                content.append(tool('rally_propose', dict(summary='Fixture suggestion requires review', changes=[
                    dict(operation='update', kind='HierarchicalRequirement', ref=self.artifact, fields={'Name': 'AI reviewed title'})])))
            elif prompt == 'selection-tools':
                content.append(tool('selection_tool', {'command': 'echo ' + 'argument-é ' * 25 + 'ARGUMENT_END'}))
            elif prompt == 'benchmark-tools':
                for index in range(6):
                    content.append(tool('benchmark_tool', {'fixture': index}))
        if content:
            reason = 'tool_use'
        else:
            text = ('ObsidianNeedle: current keyword summary for this fixture conversation.' if summary
                    else 'Fixture answer for ' + prompt + ' finished')
            content = [{'text': text}]
        if handler.path.endswith('/converse'):
            assert not prompt.startswith('hold'), 'Hold/cancellation checks require streaming'
            return self.json_response(handler, dict(output={'message': {'role': 'assistant', 'content': content}},
                                                    stopReason=reason, usage={'inputTokens': 3, 'outputTokens': 4, 'totalTokens': 7},
                                                    metrics={'latencyMs': 1}))
        handler.send_response(200)
        handler.send_header('Content-Type', 'application/vnd.amazon.eventstream')
        handler.end_headers()

        def event(kind, payload):
            handler.wfile.write(event_frame(kind, payload))
            handler.wfile.flush()

        event('messageStart', {'role': 'assistant'})
        if not summary and (prompt == 'benchmark-stream' or prompt.startswith('hold')):
            with self.stream_lock:
                self.active_streams += 1
                self.peak_streams = max(self.peak_streams, self.active_streams)
                handler.tracked_stream = True
            for index in range(self.stream_chunks):
                if self.stop.wait(.02):
                    return
                text = (f'Streamed sentence {index} with **formatted** text.\n\n' if prompt == 'benchmark-stream'
                        else f'Holding {prompt} {index}\n')
                event('contentBlockDelta', {'contentBlockIndex': 0, 'delta': {'text': text}})
            event('contentBlockStop', {'contentBlockIndex': 0})
        else:
            for index, block in enumerate(content):
                if 'toolUse' in block:
                    call = block['toolUse']
                    event('contentBlockStart', {'contentBlockIndex': index, 'start': {'toolUse': {k: call[k] for k in ('toolUseId', 'name')}}})
                    event('contentBlockDelta', {'contentBlockIndex': index, 'delta': {'toolUse': {'input': json.dumps(call['input'])}}})
                else:
                    event('contentBlockDelta', {'contentBlockIndex': index, 'delta': {'text': block['text']}})
                event('contentBlockStop', {'contentBlockIndex': index})
        event('messageStop', {'stopReason': reason})
        event('metadata', {'usage': {'inputTokens': 3, 'outputTokens': 4, 'totalTokens': 7}, 'metrics': {'latencyMs': 1}})

    def close(self):
        self.stop.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)

    def write_evidence(self, directory):
        Path(directory, 'provider.json').write_text(json.dumps(dict(requests=self.requests,
            tool_calls=self.tool_calls, peak_streams=self.peak_streams, workspace_results=self.workspace_results, errors=self.errors), ensure_ascii=False, indent=2), encoding='utf-8')
        assert not self.errors, self.errors


class ProcessGuard:
    """Bound a single fixture-owned process; never terminate other applications."""
    def __init__(self, process, timeout=120, cap=2_000_000_000):
        assert os.name == 'nt'
        self.process, self.cap = process, cap
        self.deadline, self.peak = time.monotonic() + timeout, 0
        class Memory(ctypes.Structure):
            _fields_ = [('cb', wintypes.DWORD), ('PageFaultCount', wintypes.DWORD)] + [
                (name, ctypes.c_size_t) for name in ('PeakWorkingSetSize', 'WorkingSetSize', 'QuotaPeakPagedPoolUsage',
                'QuotaPagedPoolUsage', 'QuotaPeakNonPagedPoolUsage', 'QuotaNonPagedPoolUsage',
                'PagefileUsage', 'PeakPagefileUsage', 'PrivateUsage')]
        self.memory = Memory()
        self.memory.cb = ctypes.sizeof(Memory)
        self.read = ctypes.WinDLL('psapi').GetProcessMemoryInfo
        self.read.argtypes = [wintypes.HANDLE, ctypes.POINTER(Memory), wintypes.DWORD]
        self.read.restype = wintypes.BOOL

    def sample(self):
        assert time.monotonic() < self.deadline, 'Native fixture timed out'
        if self.process.poll() is not None:
            return
        assert self.read(int(self.process._handle), ctypes.byref(self.memory), self.memory.cb), 'Cannot read fixture memory'
        self.peak = max(self.peak, self.memory.PeakWorkingSetSize, self.memory.PeakPagefileUsage)
        assert self.peak < self.cap, f'Fixture exceeded memory cap: {self.peak} >= {self.cap}'

    def wait(self, tick=None):
        try:
            while self.process.poll() is None:
                self.sample()
                if tick:
                    tick()
                time.sleep(.1)
            assert self.process.returncode == 0, f'Native process failed: {self.process.returncode}'
        finally:
            if self.process.poll() is None:
                self.process.kill()
                self.process.wait(timeout=10)
        return self.peak

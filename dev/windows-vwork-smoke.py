#!/usr/bin/env python3
"""Exercise the signed native virtual workspace window against local fixtures."""
import base64
import hashlib
import http.server
import json
import os
from pathlib import Path
import subprocess
import sys
import threading

assert sys.platform == 'win32', 'GUI acceptance runs only on Windows'
root = Path(__file__).resolve().parents[1]
binary = Path(sys.argv[1]).resolve()
out = root / 'build' / 'vwork-smoke'
out.mkdir(parents=True, exist_ok=True)
state = out / 'state'
state.mkdir(exist_ok=True)
fake = out / 'vwork.exe'
fake.write_bytes(b'Fixture marker; Fastrock must never execute this file.')
config = out / 'goose-config.yaml'
content = '\ufefffn main() {}\r\n'
revision = 'r1'
requests = []


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        global revision, content
        assert self.headers.get('Authorization') == 'Bearer local-fixture'
        payload = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        requests.append(payload)
        op, args = payload['operation'], payload['arguments']
        entry = dict(path='src/main.rs', size=len(content.encode()), mode='100755')
        if op == 'health':
            result = dict(capabilities=dict(shared_mcp=False))
        elif op == 'repo_list':
            result = [dict(id='mono')]
        elif op == 'workspace_list':
            result = [dict(id='task', repo='mono', head=revision)]
        elif op in ('files_list', 'files_stat'):
            result = dict(revision=revision, entries=[entry], total=1, next_offset=None)
        elif op == 'files_read':
            assert args['binary'] is True
            result = dict(revision=revision, base64=base64.b64encode(content.encode()).decode())
        elif op == 'database_compact':
            assert args == {}
            result = dict(compacted=True, reason='compacted', reclaimed_bytes=1200000000, after_bytes=680000000)
        elif op == 'files_write':
            assert args['mode'] == '100755'
            if revision == 'r1':
                revision = 'r2'
                result = None
            else:
                assert args['expected_revision'] == 'r2'
                content = base64.b64decode(args['base64']).decode()
                revision = 'r3'
                result = dict(revision=revision)
        else:
            raise AssertionError(op)
        body = json.dumps(dict(error='revision conflict') if result is None else dict(result=result)).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
(state / 'connection.json').write_text(json.dumps(dict(url=f'http://127.0.0.1:{server.server_port}/', token='local-fixture')))


def run(enabled, steps, name):
    config.write_text(json.dumps(dict(extensions=dict(vwork=dict(enabled=enabled, type='stdio', cmd=str(fake), args=['--home', str(state), 'mcp'])))))
    script = out / f'{name}.json'
    script.write_text(json.dumps(steps))
    env = dict(os.environ, FASTROCK_GOOSE_CONFIG=str(config), FASTROCK_HOME=str(out / name),
               CODEX_HOME=str(out / (name + '-codex')), FASTROCK_GOOSE=sys.executable,
               FASTROCK_GOOSE_FIXTURE=str(root / 'dev/external-goose-fixture.py'),
               CODEX_GUI_AUTOMATION=str(script), SLINT_BACKEND='winit-software')
    env.pop('FASTROCK_RALLY_TOKEN', None)
    with (out / f'{name}.log').open('w') as log:
        subprocess.run([str(binary), '--remote', 'goose'], env=env, stdout=log, stderr=log, timeout=90, check=True)


def virtual(*args):
    return dict(virtual_workspaces=list(args))


try:
    run(True, [dict(wait=2000), virtual('open'), dict(wait=300),
               virtual('action', 'virtual-compact'), dict(wait=500), virtual('dump', str(out / 'database-only.json')),
               virtual('select', '1'), dict(wait=500),
               virtual('file', '0'), dict(wait=500), virtual('action', 'virtual-edit:edited\r\n'),
               virtual('action', 'virtual-compact'), dict(wait=500), virtual('dump', str(out / 'compacted.json')),
               dict(wait=300), virtual('action', 'virtual-save'), dict(wait=500), virtual('dump', str(out / 'conflict.json')),
               virtual('snapshot', str(out / 'virtual.png')), dict(wait=300),
               virtual('action', 'virtual-discard'), virtual('action', 'virtual-reload'), dict(wait=500),
               virtual('action', 'virtual-edit:edited\r\n'), virtual('action', 'virtual-save'), dict(wait=500),
               virtual('dump', str(out / 'saved.json')), dict(wait=500), dict(quit=True)], 'enabled')
    database_only = json.loads((out / 'database-only.json').read_text())
    assert database_only['canCompact'] and '1.20 GB' in database_only['report'], database_only
    compacted = json.loads((out / 'compacted.json').read_text())
    assert compacted['dirty'] and compacted['text'] == 'edited\r\n', compacted
    assert '1.20 GB' in compacted['report'], compacted
    conflict = json.loads((out / 'conflict.json').read_text())
    saved = json.loads((out / 'saved.json').read_text())
    assert conflict['visible'] and conflict['dirty'] and conflict['text'] == 'edited\r\n', conflict
    assert 'revision conflict' in conflict['status'], conflict
    assert saved['visible'] and not saved['dirty'] and saved['text'] == 'edited\r\n', saved
    assert revision == 'r3' and content == 'edited\r\n'
    assert (out / 'virtual.png').stat().st_size > 1000
    run(False, [dict(wait=2000), virtual('dump', str(out / 'disabled.json')), dict(wait=500), dict(quit=True)], 'disabled')
    assert not json.loads((out / 'disabled.json').read_text())['visible']
    (out / 'acceptance.json').write_text(json.dumps(dict(result='PASS', binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
        native_window=True, compaction_preserves_draft=True, conflict_preserves_draft=True, revision_checked_save=True, hidden_without_vwork=True), indent=2))
    print('PASS: native vwork window, conflict-safe edits and hidden controls when disabled')
finally:
    server.shutdown()

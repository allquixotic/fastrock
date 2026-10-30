#!/usr/bin/env python3
"""Native isolated delegation through real embedded Goose and vwork; local model only."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import time
from embedded_provider_fixture import ProviderFixture, ProcessGuard, event_frame

assert sys.platform == 'win32', 'Native acceptance runs only on Windows'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
binary, out = args.binary.resolve(), args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
original = b'parent source remains unchanged\r\n'
child_text = 'isolated delegate change λ\r\n'


class DelegationFixture(ProviderFixture):
    def __init__(self):
        super().__init__(stream_chunks=3000)
        self.results, self.steps, self.child, self.holding = {}, {}, None, None
        self.crashing = None

    def respond(self, handler, body):
        messages = body['messages']
        prompt = next((' '.join(c.get('text', '') for c in m['content']) for m in reversed(messages)
                       if m['role'] == 'user' and any('text' in c for c in m['content'])), '')
        prompt = prompt.split('<turn-context>', 1)[0].strip()
        if 'search index sidecar' in json.dumps(body) or not prompt.startswith('delegation-'):
            return super().respond(handler, body)
        assert len(self.requests) < 150, 'Delegation request loop'
        self.requests.append(dict(prompt=prompt, body=body, path=handler.path, summary=False))
        names = [t['toolSpec']['name'] for t in body.get('toolConfig', {}).get('tools', []) if 'toolSpec' in t]
        assert not any(n == 'delegate' or n.endswith('__delegate') for n in names), 'Goose shared-cwd delegation is still exposed'
        assert any(name.endswith('delegate_task') for name in names), names
        results = [c['toolResult'] for c in messages[-1]['content'] if 'toolResult' in c]
        if any('text' in c for c in messages[-1]['content']):
            # After interruption Goose can combine a synthetic interrupted-tool
            # result and the next user prompt in one Bedrock user message.
            results = []
        result = None
        if results:
            assert len(results) == 1, results
            result = results[0]
            text = '\n'.join(c.get('text', '') for c in result['content'])
            self.results.setdefault(prompt, []).append(result)
            if prompt != 'delegation-parent' or self.steps.get(prompt) != 8:
                assert result.get('status') != 'error', result
                result = json.loads(text)
            else:
                # Goose's Bedrock conversion retains MCP error text but may
                # report toolResult.status=success; assert the actual refusal.
                assert 'This conversation is not your direct delegate' in text, result
        step = self.steps.get(prompt, 0)
        self.steps[prompt] = step + 1
        operation = None
        if prompt == 'delegation-parent':
            if step in (0, 1):
                if step == 1:
                    self.child = result['thread_id']
                operation = ('delegate_task', dict(task_id='worker', instructions='delegation-child'))
            elif step == 2:
                assert result['thread_id'] == self.child, result
                operation = ('wait_delegate', dict(thread_id=self.child))
            elif step == 3:
                assert result['status'] == 'completed' and 'child-finished' in result['result'], result
                operation = ('send_to_delegate', dict(thread_id=self.child, message='delegation-followup'))
            elif step == 4:
                operation = ('wait_delegate', dict(thread_id=self.child))
            elif step == 5:
                assert result['status'] == 'completed' and 'followup-finished' in result['result'], result
                operation = ('files_read', dict(path='src/main.rs', binary=True))
            elif step == 6:
                assert base64.b64decode(result['base64']) == original, result
                operation = ('list_delegates', {})
            elif step == 7:
                assert len(result['children']) == 1 and result['children'][0]['thread_id'] == self.child, result
                operation = ('send_to_delegate', dict(thread_id='goose:unrelated', message='must fail'))
        elif prompt == 'delegation-child':
            if step == 0:
                assert not any('delegation-parent' in json.dumps(m) for m in messages), 'Parent context copied into delegate'
                operation = ('files_read', dict(path='src/main.rs'))
            elif step == 1:
                operation = ('files_write', dict(path='src/main.rs', expected_revision=result['revision'], content=child_text))
        elif prompt == 'delegation-followup':
            if step == 0:
                operation = ('files_read', dict(path='src/main.rs', binary=True))
            elif step == 1:
                assert base64.b64decode(result['base64']) == child_text.encode(), result
        elif prompt in ('delegation-hold-parent', 'delegation-crash-parent'):
            crash = prompt == 'delegation-crash-parent'
            if step == 0:
                operation = ('delegate_task', dict(task_id='crashing' if crash else 'holding',
                    instructions='hold-delegation-crash' if crash else 'hold-delegation-child'))
            else:
                if step == 1:
                    if crash:
                        self.crashing = result['thread_id']
                    else:
                        self.holding = result['thread_id']
                operation = ('wait_delegate', dict(thread_id=self.crashing if crash else self.holding))
        elif prompt == 'delegation-recovered-parent':
            if step == 0:
                operation = ('list_delegates', {})
            elif step == 1:
                assert any(c['thread_id'] == self.holding and c['status'] == 'paused' for c in result['children']), result
                operation = ('send_to_delegate', dict(thread_id=self.holding, message='delegation-paused-followup'))
            elif step == 2:
                operation = ('wait_delegate', dict(thread_id=self.holding))
            elif step == 3:
                assert result['status'] == 'paused' and result['pending'] == 1, result
        else:
            raise AssertionError('Unexpected dispatched prompt: ' + prompt)
        content = [{'text': 'child-finished' if prompt == 'delegation-child' else 'followup-finished' if prompt == 'delegation-followup' else 'parent-finished'}]
        reason = 'end_turn'
        if operation:
            suffix, arguments = operation
            matches = [name for name in names if name.endswith(suffix)]
            assert len(matches) == 1, (suffix, names)
            content = [{'toolUse': dict(toolUseId=f'fixture-{len(self.requests)}', name=matches[0], input=arguments)}]
            reason = 'tool_use'
        if handler.path.endswith('/converse'):
            return self.json_response(handler, dict(output={'message': {'role': 'assistant', 'content': content}},
                stopReason=reason, usage={'inputTokens': 3, 'outputTokens': 4, 'totalTokens': 7}, metrics={'latencyMs': 1}))
        handler.send_response(200)
        handler.send_header('Content-Type', 'application/vnd.amazon.eventstream')
        handler.end_headers()
        def event(kind, payload):
            handler.wfile.write(event_frame(kind, payload))
            handler.wfile.flush()
        event('messageStart', {'role': 'assistant'})
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


provider = DelegationFixture()
env = provider.environment(out, streaming=True)
repo = out / 'project'
(repo / 'src').mkdir()
(repo / 'src/main.rs').write_bytes(original)
git_env = {k: v for k, v in os.environ.items() if not k.upper().startswith('GIT_')}
git_env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=str(out / 'aws-empty'))
for command in (['init', '.'], ['add', '.'], ['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-m', 'Fixture']):
    subprocess.run(['git', '-c', 'core.hooksPath=NUL', '-c', 'core.autocrlf=false', *command], cwd=repo,
                   env=git_env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)


def run(steps, name):
    script = out / (name + '-steps.json')
    script.write_text(json.dumps(steps), encoding='utf-8')
    with (out / (name + '.log')).open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(binary), '--remote', 'goose'], cwd=repo,
            env=dict(env, CODEX_GUI_AUTOMATION=str(script)), stdout=log, stderr=log)
        if name == 'crash':
            guard = ProcessGuard(process, timeout=45)
            try:
                while process.poll() is None:
                    guard.sample()
                    if provider.crashing and provider.active_streams == 1:
                        # Kill only this fixture-owned app after the child has
                        # durably dispatched and both agents are still running.
                        process.kill()
                        process.wait(timeout=10)
                        return guard.peak
                    time.sleep(.1)
                raise AssertionError('App exited before the forced-crash boundary')
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=10)
        def tick():
            if name == 'initial' and (out / 'stopped.json').exists():
                assert provider.active_streams == 0, 'Parent Stop left a child streaming'
        return ProcessGuard(process, timeout=150).wait(tick)


def turn(prompt):
    return [{'send': prompt}, {'wait': 300}, {'wait_idle': 45000}, {'wait': 300}]


try:
    peak = run([{'wait_ready': 30000}, {'open_folder': str(repo)}, {'wait': 1000}, {'wait_idle': 30000},
        *turn('delegation-parent'), {'send': '/status'}, {'wait': 500},
        {'snapshot': str(out / 'delegates.png')}, {'wait': 300},
        {'transcript': 'export-complete:' + str(out / 'parent.md')}, {'wait': 300},
        {'sidebar': ['dump', str(out / 'agents.json')]}, {'wait': 300},
        {'send': 'delegation-hold-parent'}, {'wait': 4000}, {'click_text': 'Stop'}, {'wait': 500},
        {'wait_idle': 30000}, {'wait': 1000}, {'sidebar': ['dump', str(out / 'stopped.json')]},
        {'wait': 1000}, {'quit': True}], 'initial')
    assert provider.steps.get('delegation-parent') == 9, provider.steps
    assert provider.child and provider.holding, provider.steps
    agents = json.loads((out / 'agents.json').read_text(encoding='utf-8'))['navigation']['agents']
    assert any(a['id'] == provider.child and a['label'] == 'worker' for a in agents), agents
    assert (repo / 'src/main.rs').read_bytes() == original
    bindings = [json.loads(p.read_text(encoding='utf-8')) for p in out.rglob('*.binding.json')]
    children = [b for b in bindings if b.get('delegation')]
    assert len(children) == 2, children
    parent_ids = {b['delegation']['parent'] for b in children}
    assert len(parent_ids) == 1
    parent_id = parent_ids.pop()
    assert len({b['workspace']['cwd'] for b in bindings}) == 3, bindings
    assert len({b['workspace']['selection']['workspace'] for b in bindings}) == 3, bindings
    indexes = list(out.rglob('*.delegates.json'))
    assert len(indexes) == 1 and len(json.loads(indexes[0].read_text(encoding='utf-8'))) == 2
    database_path, = (out / 'goose').rglob('sessions.db')
    with sqlite3.connect(database_path) as database:
        rows = database.execute('SELECT id, parent_session_id FROM sessions WHERE parent_session_id IS NOT NULL').fetchall()
        assert len(rows) == 2 and all(parent == parent_id for _, parent in rows), rows
    held = [r for r in provider.requests if r['prompt'] == 'hold-delegation-child']
    assert len(held) == 1, held
    peak = max(peak, run([{'wait_ready': 30000}, {'resume': 'goose:' + parent_id}, {'wait': 1000},
        {'wait_idle': 30000}, {'send': '/status'}, {'wait': 500},
        {'sidebar': ['dump', str(out / 'restored-agents.json')]}, {'wait': 300},
        {'snapshot': str(out / 'restored-delegates.png')}, {'wait': 300},
        {'click_text': 'Resume'}, {'wait': 500}, *turn('delegation-recovered-parent'),
        {'quit': True}], 'restored'))
    assert provider.steps.get('delegation-recovered-parent') == 4, provider.steps
    restored = json.loads((out / 'restored-agents.json').read_text(encoding='utf-8'))['navigation']['agents']
    assert {a['id'] for a in restored} == {provider.child, provider.holding}, restored
    assert len([r for r in provider.requests if r['prompt'] == 'hold-delegation-child']) == 1, 'Recovered task was replayed'
    assert not any(r['prompt'] == 'delegation-paused-followup' for r in provider.requests), 'Paused queue ran'
    peak = max(peak, run([{'wait_ready': 30000}, {'resume': 'goose:' + parent_id}, {'wait': 1000},
        {'wait_idle': 30000}, {'click_text': 'Resume'}, {'wait': 500},
        {'send': 'delegation-crash-parent'}, {'wait': 60000}], 'crash'))
    peak = max(peak, run([{'wait_ready': 30000}, {'resume': 'goose:' + parent_id}, {'wait': 1500},
        {'wait_idle': 30000}, {'resume': provider.crashing}, {'wait': 1000}, {'wait_idle': 30000},
        {'click_text': 'Review interrupted sends (1)'}, {'wait': 300},
        {'sidebar': ['dump', str(out / 'crash-restored.json')]},
        {'wait': 1000}, {'quit': True}], 'after-crash'))
    assert len([r for r in provider.requests if r['prompt'] == 'hold-delegation-crash']) == 1, 'Crashed task replayed'
    recovered_chats = json.loads((out / 'crash-restored.json').read_text(encoding='utf-8'))['navigation']['openChats']
    assert any(c['id'] == provider.crashing and c['paused'] for c in recovered_chats), recovered_chats
    crash_binding, = [p for p in out.rglob('*.binding.json') if
        (json.loads(p.read_text(encoding='utf-8')).get('delegation') or {}).get('task_id') == 'crashing']
    queue = json.loads(crash_binding.with_name(crash_binding.name.replace('.binding.json', '.json')).read_text(encoding='utf-8'))
    assert queue['paused'] and len(queue['uncertain']) == 1 and not queue['pending'], queue
    (out / 'acceptance.json').write_text(json.dumps(dict(result='PASS', binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
        peak_process_bytes=peak, fresh_child_context=True, isolated_workspace=True, source_preserved=True,
        idempotent_delegation=True, wait_and_followup=True, child_ownership_guard=True,
        parent_stop_cancels_child=True, restart_requires_resume=True, paused_followup_retained=True,
        forced_crash_no_tool_replay=True, native_child_pane_restored=True,
        no_goose_shared_cwd_delegate=True, no_backend_installed=True), indent=2), encoding='utf-8')
    print('PASS: isolated delegation, retry identity, followup/wait, ownership, Stop and restart Resume', flush=True)
finally:
    provider.close()
    provider.write_evidence(out)

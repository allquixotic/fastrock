#!/usr/bin/env python3
"""Native schedules: local model, isolated vwork runs, timer/controls and crash recovery."""
import argparse
import base64
from datetime import datetime, timedelta
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from embedded_provider_fixture import ProviderFixture, ProcessGuard, event_frame

assert os.name == 'nt', 'Native acceptance runs only on Windows'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
binary, out = args.binary.resolve(), args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
repo = out / 'project'
original = b'original scheduled source\r\n'


class ScheduleFixture(ProviderFixture):
    def __init__(self):
        super().__init__(workspace_repo=repo, stream_chunks=3000)
        self.finished = []

    def respond(self, handler, body):
        messages = body['messages']
        prompt = next((' '.join(c.get('text', '') for c in m['content']) for m in reversed(messages)
                       if m['role'] == 'user' and any('text' in c for c in m['content'])), '')
        prompt = prompt.split('<turn-context>', 1)[0].strip()
        if 'search index sidecar' in json.dumps(body) or not prompt.startswith('schedule-check'):
            return super().respond(handler, body)
        assert not any('source-context-marker' in json.dumps(m) for m in messages), 'Schedule copied source conversation'
        assert len(self.requests) < 150, 'Schedule provider loop'
        self.requests.append(dict(prompt=prompt, body=body, path=handler.path, summary=False))
        names = [t['toolSpec']['name'] for t in body.get('toolConfig', {}).get('tools', []) if 'toolSpec' in t]
        uses = [c['toolUse'] for m in messages for c in m['content'] if 'toolUse' in c]
        results = [c['toolResult'] for c in messages[-1]['content'] if 'toolResult' in c]
        result = None
        if results:
            assert all(r.get('status') != 'error' for r in results), results
            result = json.loads('\n'.join(c.get('text', '') for r in results for c in r['content']))
        if len(uses) == 0:
            operation = ('files_read', dict(path='src/main.rs', binary=True))
        elif len(uses) == 1:
            assert base64.b64decode(result['base64']) == original, result
            operation = ('files_write', dict(path='src/main.rs', expected_revision=result['revision'], content='isolated schedule change λ\r\n'))
        else:
            assert len(uses) == 2, uses
            operation = None
            self.finished.append(prompt)
        content, reason = [{'text': 'Scheduled work finished'}], 'end_turn'
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


provider = ScheduleFixture()
env = provider.environment(out, streaming=True)
(repo / 'src').mkdir()
(repo / 'src/main.rs').write_bytes(original)
git_env = {k: v for k, v in os.environ.items() if not k.upper().startswith('GIT_')}
git_env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=str(out / 'aws-empty'))
for command in (['init', '.'], ['add', '.'], ['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-m', 'Fixture']):
    subprocess.run(['git', '-c', 'core.hooksPath=NUL', '-c', 'core.autocrlf=false', *command], cwd=repo,
                   env=git_env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)
store = out / 'home/goose-queues/schedules.json'


def action(*args):
    return {'settings': ['schedules', *args]}


def dump(name):
    return [action('refresh'), {'wait': 500}, action('dump', str(out / (name + '.json'))), {'wait': 300}]


def create(title, prompt, kind='0', *fields):
    # Controller callbacks defer while automation holds its mutable borrow.
    return [action('new'), {'wait': 100}, action('title', title), action('prompt', prompt), action('kind', kind),
            *fields, action('save'), {'wait': 700}]


def run(steps, name):
    script = out / (name + '-steps.json')
    script.write_text(json.dumps(steps), encoding='utf-8')
    with (out / (name + '.log')).open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(binary), '--remote', 'goose'], cwd=repo,
            env=dict(env, CODEX_GUI_AUTOMATION=str(script)), stdout=log, stderr=log)
        guard = ProcessGuard(process, timeout=150)
        if name == 'crash':
            try:
                while process.poll() is None:
                    guard.sample()
                    if provider.active_streams == 1:
                        data = json.loads(store.read_text(encoding='utf-8'))
                        task = next(t for t in data['tasks'].values() if t['title'] == 'Hold')
                        if task['active'] and task['active']['child']:
                            process.kill()
                            process.wait(timeout=10)
                            return guard.peak
                    time.sleep(.1)
                raise AssertionError('App exited before crash boundary')
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=10)
        def tick():
            if name == 'restored' and (out / 'stopped.json').exists():
                assert provider.active_streams == 0, 'Schedule Stop left its child running'
        return guard.wait(tick)


def read(name):
    return json.loads((out / (name + '.json')).read_text(encoding='utf-8'))


def task(snapshot, title):
    return next(t for t in snapshot['data']['tasks'] if t['title'] == title)


try:
    once = (datetime.now() + timedelta(seconds=20)).strftime('%Y-%m-%d %H:%M:%S')
    daily = (datetime.now() + timedelta(hours=2)).strftime('%H:%M')
    peak = run([{'wait_ready': 30000}, {'open_folder': str(repo)}, {'wait': 1000}, {'wait_idle': 30000},
        {'send': 'source-context-marker'}, {'wait': 300}, {'wait_idle': 30000},
        {'open_settings': 'schedules'}, {'wait': 800},
        *create('Once', 'schedule-check-once', '2', action('once', once)),
        {'wait': 23000}, action('retry-create'), {'wait': 500}, *dump('once'),
        {'snapshot': str(out / 'schedules.png')}, {'wait': 300},
        *create('Recurring', 'schedule-check-interval', '0', action('minutes', '1')),
        action('run', 'Recurring'), {'wait': 2000}, *dump('first'),
        action('select', 'Recurring'), {'wait': 100}, action('title', 'Edited recurring'), action('save'), {'wait': 700},
        action('pause', 'Edited recurring'), {'wait': 700}, action('run', 'Edited recurring'), {'wait': 2000}, *dump('second'),
        # Keep an old edit revision while a manual run changes the record.
        action('select', 'Edited recurring'), {'wait': 100}, action('title', 'Must not overwrite'),
        action('run', 'Edited recurring'), {'wait': 2000}, action('save'), {'wait': 500}, *dump('stale'),
        *create('Daily', 'schedule-check-daily', '1', action('time', daily)),
        *dump('before-quit'), {'open_settings': 'appearance'}, {'wait': 200},
        {'click_text': 'Goose'}, {'wait': 300}, {'click_text': 'Schedules'}, {'wait': 300},
        {'quit': True}], 'initial')
    assert task(read('once'), 'Once')['last']['status'] == 'completed', read('once')
    assert len(read('once')['data']['tasks']) == 1 and read('once')['status'] == 'Schedule saved', read('once')
    first, second = task(read('first'), 'Recurring'), task(read('second'), 'Edited recurring')
    assert first['last']['status'] == second['last']['status'] == 'completed'
    assert first['last']['child'] != second['last']['child'] and second['paused'], (first, second)
    assert 'changed' in read('stale')['status'].lower() and task(read('stale'), 'Edited recurring'), read('stale')
    assert provider.finished.count('schedule-check-once') == 1
    assert provider.finished.count('schedule-check-interval') == 3, provider.finished
    assert (repo / 'src/main.rs').read_bytes() == original
    data = json.loads(store.read_text(encoding='utf-8'))
    # Fixture snapshot of an enabled, overdue task at a crash boundary. Startup
    # must pause it before the timer sees it; it must never catch up by running.
    for t in data['tasks'].values():
        if t['title'] == 'Daily':
            t['paused'], t['next_at'] = False, 1
    store.write_text(json.dumps(data), encoding='utf-8')
    before = len(provider.finished)
    peak = max(peak, run([{'wait_ready': 30000}, {'open_settings': 'schedules'}, {'wait': 2500},
        *dump('restored'), action('resume', 'Daily'), {'wait': 700}, *dump('resumed'),
        action('run', 'Daily'), {'wait': 2000}, action('pause', 'Daily'), {'wait': 500},
        *create('Hold', 'hold-schedule', '0', action('minutes', '1')),
        action('run', 'Hold'), {'wait': 2000}, action('stop', 'Hold'), {'wait': 1200},
        *dump('stopped'), {'wait': 500}, {'quit': True}], 'restored'))
    assert all(t['paused'] and t['active'] is None for t in read('restored')['data']['tasks']), read('restored')
    assert task(read('resumed'), 'Daily')['next_at'] > time.time() and not task(read('resumed'), 'Daily')['paused']
    assert len(provider.finished) == before + 1, provider.finished
    stopped = task(read('stopped'), 'Hold')
    assert stopped['paused'] and stopped['active'] is None and stopped['last']['child'], stopped
    peak = max(peak, run([{'wait_ready': 30000}, {'open_settings': 'schedules'}, {'wait': 1000},
        action('run', 'Hold'), {'wait': 60000}], 'crash'))
    active = next(t for t in json.loads(store.read_text(encoding='utf-8'))['tasks'].values() if t['title'] == 'Hold')['active']
    peak = max(peak, run([{'wait_ready': 30000}, {'open_settings': 'schedules'}, {'wait': 2000},
        *dump('after-crash'), {'resume': 'goose:' + active['child']}, {'wait': 1000}, {'wait_idle': 30000},
        {'sidebar': ['dump', str(out / 'child-recovered.json')]}, {'wait': 300}, {'quit': True}], 'after-crash'))
    restored = task(read('after-crash'), 'Hold')
    assert restored['paused'] and restored['active'] is None and restored['last']['status'] == 'needs_review', restored
    assert restored['last']['id'] == active['id'] and restored['last']['child'] == active['child']
    assert any(c['id'] == 'goose:' + active['child'] and c['paused'] for c in read('child-recovered')['navigation']['openChats'])
    assert len([r for r in provider.requests if r['prompt'] == 'hold-schedule' and not r['summary']]) == 2
    bindings = [json.loads(p.read_text(encoding='utf-8')) for p in out.rglob('*.binding.json')]
    children = [b for b in bindings if (b.get('delegation') or {}).get('scheduled')]
    assert len(children) == 7 and len({b['workspace']['selection']['workspace'] for b in children}) == 7, children
    assert all((b['delegation'].get('label') or '') in ('Once', 'Recurring', 'Edited recurring', 'Daily', 'Hold') for b in children)
    assert (repo / 'src/main.rs').read_bytes() == original
    (out / 'acceptance.json').write_text(json.dumps(dict(result='PASS', binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
        peak_process_bytes=peak, timer_dispatch=True, fresh_isolated_runs=True, named_child_runs=True,
        manual_run_while_paused=True, stale_edit_rejected=True, pause_resume_skip_missed=True,
        stop_cancels_child=True, forced_crash_no_replay=True, native_recovery_visible=True,
        source_checkout_preserved=True, provider_requests_local=True), indent=2), encoding='utf-8')
    print('PASS: native schedules, timer, isolated runs, controls and crash recovery', flush=True)
finally:
    provider.close()
    provider.write_evidence(out)

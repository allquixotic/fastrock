#!/usr/bin/env python3
"""Windows-only 0.4.0 queue/steer, tooltip bounds, title fit and read-state regression."""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
assert sys.platform == 'win32', "Never run GUI tests on Sean's Mac"
import mock_responses

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--renderer', choices=['software', 'gpu'], default='software')
parser.add_argument('--output', type=Path, default=ROOT / 'dist/conversation-smoke')
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
out = args.output.resolve()
with tempfile.TemporaryDirectory(prefix='codex-gui-conversation-') as directory:
    work = Path(directory)
    home, project = work / 'home', work / 'project'
    project.mkdir()
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        port = listener.getsockname()[1]
    mock_responses.write_config(home, port)
    config = home / 'config.toml'
    config.write_text(config.read_text().replace('sandbox_mode = "workspace-write"', 'sandbox_mode = "danger-full-access"') + f'\n[projects.{json.dumps(str(project))}]\ntrust_level = "trusted"\n')
    requests = out / 'requests.jsonl'
    requests.unlink(missing_ok=True)
    with open(out / 'mock.log', 'w') as log:
        mock = subprocess.Popen([sys.executable, str(ROOT / 'dev/mock_responses.py'), '--port', str(port), '--request-log', str(requests)], stdout=log, stderr=log)
        try:
            for _ in range(100):
                try:
                    with socket.create_connection(('127.0.0.1', port), timeout=.1): break
                except OSError: time.sleep(.1)
            else: raise RuntimeError('Mock server unavailable')
            def run(name, steps):
                script = work / f'{name}.json'
                script.write_text(json.dumps(steps))
                with open(out / f'{name}.log', 'w') as log:
                    process = subprocess.Popen([str(args.binary.resolve()), '--renderer', args.renderer], env=dict(os.environ, CODEX_HOME=str(home), CODEX_GUI_AUTOMATION=str(script)), stdout=log, stderr=log)
                    try: assert process.wait(timeout=180) == 0
                    finally:
                        if process.poll() is None: process.kill(); process.wait(timeout=10)
                text = (out / f'{name}.log').read_text()
                assert 'timed out' not in text and 'target not found' not in text, text
            def dump(name): return {'sidebar': ['dump', str(out / f'{name}.json')]}
            run('controls', [
                {'wait_ready': 60000}, {'new_thread': str(project)}, {'wait_idle': 30000},
                {'send': 'plan slow'}, {'wait': 1500},
                {'composer': ['type', 'QUEUED_REQUEST']}, {'key': 'Return'}, {'wait': 300},
                {'composer': ['type', 'STEERED_REQUEST']}, {'key': 'Shift+Return'}, {'wait': 12000}, {'wait_idle': 30000},
                {'send': 'purpose-layout'}, {'wait': 500}, {'wait_idle': 30000}, {'wait_purpose': 60000}, {'wait': 500},
                dump('before-hover'), {'sidebar': ['hover', '10']}, {'wait_sidebar_tooltip': 5000}, dump('left-hover'), {'snapshot': str(out / 'left-hover.png')},
                {'pointer': ['move', 500, 300]}, {'wait': 100}, {'sidebar': ['hover', '250']}, {'wait_sidebar_tooltip': 5000}, dump('right-hover'),
                {'resize': [700, 440]}, {'wait': 300}, {'shortcut': 'toggle-sidebar'}, {'wait': 400}, {'pointer': ['move', 500, 300]}, {'wait': 200}, {'sidebar': ['hover', '10']}, {'wait_sidebar_tooltip': 5000}, dump('narrow-hover'), {'snapshot': str(out / 'narrow-hover.png')},
                {'resize': [1280, 820]}, {'wait': 500},
                {'new_thread': str(project)}, {'wait_idle': 30000}, {'send': 'slow background'}, {'wait': 500}, {'select_tab': 0}, {'wait': 9000}, dump('unread'),
                {'select_tab': 1}, {'wait': 500}, dump('read'), {'quit': True},
            ])
            bodies = [json.loads(line) for line in requests.read_text().splitlines()]
            ordinary = [body for body in bodies if not mock_responses.wants_json_schema(body)]
            texts = [mock_responses.last_user_text(body) or '' for body in ordinary]
            steered = next(i for i, text in enumerate(texts) if 'STEERED_REQUEST' in text)
            queued = next(i for i, text in enumerate(texts) if 'QUEUED_REQUEST' in text)
            assert queued > steered, f'Queue was submitted before steering: {texts}'
            assert any(item.get('name') == 'update_plan' for body in ordinary for item in body.get('input', [])), 'Multi-step tool turn was not exercised'
            def data(name): return json.loads((out / f'{name}.json').read_text())
            for name in ['left-hover', 'right-hover', 'narrow-hover']:
                shot = data(name)
                assert shot['tooltip_text'], (name, shot)
                x, y, w, h = shot['tooltip_bounds']
                px, py, pw, ph = shot['tooltip_pane']
                assert x >= px and y >= py and x + w <= px + pw + .5 and y + h <= py + ph + .5, shot
                assert any(text['thread'] for text in shot['rendered_text']), (name, shot)
                for text in shot['rendered_text']:
                    if text['thread'] and '…' not in text['text']:
                        assert text['measured'] <= text['width'] + .5, text
            unread, read = data('unread'), data('read')
            assert any(row['status'] == 'Unread' for row in unread['rows']), unread
            assert all(row['status'] != 'Unread' for row in read['rows']), read
            run('pending', [
                {'wait_ready': 60000}, {'resume': read['rows'][0]['id']}, {'wait_idle': 30000},
                {'send': 'slow pending editing'}, {'wait': 300},
                {'composer': ['type', 'ORIGINAL_PENDING']}, {'composer': ['send']}, {'wait': 400}, {'pending': ['click']}, {'wait_pending': {'open': True, 'timeout_ms': 5000}},
                {'pending': ['dump', str(out / 'editing.json')]}, {'snapshot': str(out / 'editing.png')},
                {'pending': ['text', 'EDITED_PENDING']}, {'pending': ['save']}, {'wait_pending': {'open': False, 'timeout_ms': 5000}},
                {'pending': ['dump', str(out / 'saved.json')]},
                {'composer': ['type', 'DELETED_PENDING']}, {'composer': ['send']}, {'wait': 400}, {'pending': ['click']}, {'wait_pending': {'open': True, 'timeout_ms': 5000}},
                {'pending': ['delete']}, {'wait_pending': {'open': False, 'timeout_ms': 5000}}, {'pending': ['dump', str(out / 'deleted.json')]},
                {'wait': 16000}, {'wait_idle': 30000},
                {'send': 'pending-tool'}, {'wait': 1500},
                {'composer': ['type', 'STEER_ORIGINAL']}, {'composer': ['steer']}, {'wait': 400},
                {'pending': ['click']}, {'wait_pending': {'open': True, 'timeout_ms': 5000}}, {'pending': ['dump', str(out / 'steer-editing.json')]},
                {'pending': ['text', 'STEER_EDITED']}, {'pending': ['save']}, {'wait_pending': {'open': False, 'timeout_ms': 5000}},
                {'pending': ['dump', str(out / 'steer-saved.json')]},
                {'composer': ['type', 'STEER_DELETED']}, {'composer': ['steer']}, {'wait': 400},
                {'pending': ['click']}, {'wait_pending': {'open': True, 'timeout_ms': 5000}}, {'pending': ['delete']}, {'wait_pending': {'open': False, 'timeout_ms': 5000}},
                {'pending': ['dump', str(out / 'steer-deleted.json')]}, {'wait': 16000}, {'wait_idle': 30000},
                {'send': 'slow dispatch race'}, {'wait': 300}, {'composer': ['type', 'RACE_ORIGINAL']}, {'composer': ['send']}, {'wait': 400},
                {'pending': ['click']}, {'wait_pending': {'open': True, 'timeout_ms': 5000}}, {'pending': ['text', 'RACE_CHANGED']},
                {'wait': 9000}, {'pending': ['save']}, {'wait_pending': {'open': True, 'timeout_ms': 5000}},
                {'pending': ['dump', str(out / 'race.json')]}, {'pending': ['cancel']}, {'wait_purpose': 60000}, {'wait': 500}, {'quit': True},
            ])
            assert data('editing')['open'] and data('editing')['text'] == 'ORIGINAL_PENDING', data('editing')
            assert not data('saved')['open'] and not data('saved')['error'], data('saved')
            assert not data('deleted')['open'] and not data('deleted')['error'], data('deleted')
            assert data('steer-editing')['open'] and data('steer-editing')['text'] == 'STEER_ORIGINAL', data('steer-editing')
            assert not data('steer-saved')['open'] and not data('steer-saved')['error'], data('steer-saved')
            assert not data('steer-deleted')['open'] and not data('steer-deleted')['error'], data('steer-deleted')
            assert data('race')['open'] and data('race')['error'] and data('race')['text'] == 'RACE_CHANGED', data('race')
            bodies = [json.loads(line) for line in requests.read_text().splitlines()]
            texts = [mock_responses.last_user_text(body) or '' for body in bodies if not mock_responses.wants_json_schema(body)]
            assert any('EDITED_PENDING' in text for text in texts), texts
            assert any('STEER_EDITED' in text for text in texts), texts
            assert all('ORIGINAL_PENDING' not in text and 'DELETED_PENDING' not in text and 'RACE_CHANGED' not in text and 'STEER_ORIGINAL' not in text and 'STEER_DELETED' not in text for text in texts), texts
            initial = json.loads((home / 'gui-thread-activity.json').read_text())
            (out / 'activity-before-reopen.json').write_text(json.dumps(initial, indent=2))
            prior = len(bodies)
            run('reopen', [{'wait_ready': 60000}, {'resume': read['rows'][0]['id']}, {'wait_idle': 30000}, {'wait': 1500}, dump('reopened'), {'quit': True}])
            assert len(requests.read_text().splitlines()) == prior, 'Viewing generated inference'
            restored = json.loads((home / 'gui-thread-activity.json').read_text())
            (out / 'activity.json').write_text(json.dumps(restored, indent=2))
            for key in initial:
                assert initial[key]['at'] == restored[key]['at'], ('Viewing advanced activity', key, initial[key], restored[key])
            print('Conversation smoke passed: queue waits for full multi-step turn, Shift+Enter steers, pane-confined tooltips at both edges/narrow window, measured titles, unread/read and stable reopen activity.')
        finally:
            mock.terminate(); mock.wait(timeout=10)

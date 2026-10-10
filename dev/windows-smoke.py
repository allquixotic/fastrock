#!/usr/bin/env python3
"""Run the real GUI against the mock on Windows; verify async replies and screenshots."""
import argparse
import json
import os
from pathlib import Path
import socket
import shutil
import subprocess
import sys
import tempfile
import time

assert sys.platform == 'win32', "GUI smoke tests run on Windows only; never on Sean's Mac"
ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--output', type=Path, default=ROOT / 'dist/smoke')
options = parser.parse_args()
options.output.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix='codex-gui-smoke-') as temporary:
    work = Path(temporary)
    home, project = work / 'home', work / 'project'
    project.mkdir()
    (project / 'README.md').write_text('Isolated GUI mock test project.\n')
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        port = listener.getsockname()[1]
    # Generate deterministic catalog and config before launching the HTTP server.
    import mock_responses
    mock_responses.write_config(home, port)
    config = home / 'config.toml'
    config.write_text(config.read_text().replace('sandbox_mode = "workspace-write"', 'sandbox_mode = "danger-full-access"'))
    requests = options.output / 'requests.jsonl'
    script = work / 'script.json'
    script.write_text(json.dumps([
        {'wait_ready': 60000}, {'new_thread': str(project)}, {'wait_idle': 30000},
        {'send': 'askasync'}, {'wait': 1000}, {'wait_idle': 30000}, {'wait': 500},
        {'snapshot': str(options.output / 'async-questions.png')},
        {'approval': 'select:0:1'}, {'approval': 'text:1:SMP-42'},
        {'approval': 'submit'}, {'wait': 500}, {'wait_idle': 30000}, {'wait': 500},
        {'snapshot': str(options.output / 'async-answer.png')}, {'quit': True},
    ]))
    environment = dict(os.environ, CODEX_HOME=str(home), CODEX_GUI_AUTOMATION=str(script))
    with open(options.output / 'mock.log', 'w') as mock_log, open(options.output / 'gui.log', 'w') as gui_log:
        mock = subprocess.Popen([sys.executable, str(ROOT / 'dev/mock_responses.py'), '--port', str(port), '--request-log', str(requests)], stdout=mock_log, stderr=mock_log)
        try:
            for _ in range(100):
                try:
                    with socket.create_connection(('127.0.0.1', port), timeout=.1):
                        break
                except OSError:
                    time.sleep(.1)
            else:
                raise RuntimeError('Mock did not start')
            subprocess.run([str(options.binary.resolve()), '--renderer', 'software'], env=environment, stdout=gui_log, stderr=gui_log, timeout=150, check=True)
        finally:
            for startup_log in home.glob("log/*.log"):
                shutil.copyfile(startup_log, options.output / startup_log.name)
            mock.terminate()
            mock.wait(timeout=10)
    bodies = [json.loads(line) for line in requests.read_text().splitlines()]
    replies = []
    for body in bodies:
        for item in body.get('input', []):
            if item.get('role') == 'user':
                for part in item.get('content', []):
                    text = part.get('text', '')
                    if text.startswith('<send_user_message_question_reply>'):
                        replies.extend(json.loads(text.removeprefix('<send_user_message_question_reply>').removesuffix('</send_user_message_question_reply>')))
    assert any(reply['answer'] == 'Mark it ready' for reply in replies), 'Selected suggestion never reached backend'
    assert any(reply['answer'] == 'SMP-42' for reply in replies), 'Free-text answer never reached backend'
    assert len({reply['questionItemId'] for reply in replies}) == 2, 'Question identity mismatch'
    log = (options.output / 'gui.log').read_text()
    assert 'timed out' not in log, log
    assert 'snapshot failed' not in log, log
    for name in ('async-questions.png', 'async-answer.png'):
        assert (options.output / name).stat().st_size > 1000, name
    print('Windows async question smoke passed: selected option + free text delivered; screenshots saved.')

#!/usr/bin/env python3
"""V53: bounded historical pages, stable reading during streaming, and full export."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sqlite3
import sys
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert sys.platform == 'win32', 'Native acceptance runs only on Windows'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--seed-executable', type=Path)
parser.add_argument('--output', type=Path, default=Path('build/history-smoke'))
args = parser.parse_args()
binary, out = args.binary.resolve(), args.output.resolve()
seed = args.seed_executable
if seed is None:
    target = json.loads((binary.parent / 'TEST-TARGETS.json').read_text())['embedded_goose']
    assert Path(target).name == target
    seed = binary.parent / target
out.mkdir(parents=True, exist_ok=False)
provider = ProviderFixture(stream_chunks=10000)
env = provider.environment(out, streaming=True)
parameters = out / 'seed-parameters.json'
parameters.write_text(json.dumps(dict(sessions=3, messages=400, project=str(out / 'project'),
    vwork_home=str(out / 'local/vwork'), output=str(out / 'seed.json'))), encoding='utf-8')


def bounded(command, name):
    with (out / (name + '.log')).open('w', encoding='utf-8') as log:
        process = subprocess.Popen(command, cwd=out, env=env, stdout=log, stderr=log)
        return ProcessGuard(process, timeout=120).wait()


def capture(name):
    return [{'transcript': 'history-dump:' + str(out / (name + '.json'))},
            {'transcript': 'export:' + str(out / (name + '.md'))}, {'wait': 300}]


def load(name):
    return json.loads((out / (name + '.json')).read_text(encoding='utf-8'))


try:
    peak = bounded([str(seed.resolve()), '--seed-benchmark', str(parameters)], 'seed')
    sid = json.loads((out / 'seed.json').read_text())['sessions'][0]
    steps = [{'wait_ready': 30000}, {'resize': [1440, 1000]}, {'resume': 'goose:' + sid},
             {'wait_idle': 30000}, {'wait': 500},
             {'transcript': 'history-dump:' + str(out / 'before-open.json')},
             {'snapshot': str(out / 'before-open.png')},
             {'click_text': 'History'}, {'wait': 500},
             {'transcript': 'history-dump:' + str(out / 'after-open.json')},
             {'snapshot': str(out / 'after-open.png')}, {'wait': 250},
             {'click_text': 'Oldest'}, {'wait': 500}, *capture('first'),
             {'selection': ['drag', 'Fixture prompt 0/0:', '0', '14']}, {'key': 'Ctrl+C'}, {'wait': 150},
             {'selection': ['capture', str(out / 'history-selection.txt')]},
             {'transcript': 'history-position:201'}, {'click_text': 'Go'}, {'wait': 500}, *capture('middle'),
             {'click_text': 'Earlier'}, {'wait': 500}, *capture('earlier'),
             {'click_text': 'Later'}, {'wait': 500}, *capture('later'),
             {'snapshot': str(out / 'history.png')},
             # Sending from a historical page returns to live history first.
             {'send': 'hold reading-snapshot'}, {'wait': 1200}, *capture('send-live'),
             {'click_text': 'History'}, {'wait': 500}, {'click_text': 'Oldest'}, {'wait': 500},
             *capture('reading-before'), {'wait': 1000}, *capture('reading-after'),
             {'click_text': 'Latest'}, {'wait': 1000}, *capture('latest'),
             {'shortcut': 'escape'}, {'wait_idle': 30000},
             {'transcript': 'export-complete:' + str(out / 'complete.md')}, {'wait': 800}, {'quit': True}]
    script = out / 'steps.json'
    script.write_text(json.dumps(steps), encoding='utf-8')
    env['CODEX_GUI_AUTOMATION'] = str(script)
    peak = max(peak, bounded([str(binary)], 'app'))
    for name, start in [('first', 0), ('middle', 200), ('earlier', 100), ('later', 200),
                        ('reading-before', 0), ('reading-after', 0)]:
        data = load(name)
        assert data['browsing'] and not data['loading'] and data['start'] == start, (name, data)
        assert data['end'] == start + 100 and data['rows'] < 2000 and data['bytes'] < 8 * 1024 * 1024, (name, data)
    assert 'Fixture prompt 0/0:' in (out / 'first.md').read_text(encoding='utf-8')
    assert (out / 'history-selection.txt').read_text(encoding='utf-8') == 'Fixture prompt'
    assert 'Fixture prompt 0/200:' in (out / 'middle.md').read_text(encoding='utf-8')
    assert (out / 'reading-before.md').read_bytes() == (out / 'reading-after.md').read_bytes(), 'Live stream displaced the historical page'
    assert load('reading-after')['activity'], 'New activity was not indicated'
    assert not load('send-live')['browsing'] and not load('latest')['browsing']
    assert 'Holding hold reading-snapshot' in (out / 'latest.md').read_text(encoding='utf-8')
    full = (out / 'complete.md').read_text(encoding='utf-8')
    assert full.count('## User\n') == 201 and full.count('## Assistant\n') == 201, 'Full export lost original or streamed messages'
    assert 'HISTORY-END-SENTINEL' in full and 'Fixture prompt 0/0:' in full
    # Legacy sessions had no stored tool selection. Restoration must preserve
    # inherited tools while restricting Goose's shared-directory delegation.
    database = next((out / 'goose').rglob('sessions.db'))
    with sqlite3.connect(database.as_uri() + '?mode=ro', uri=True) as connection:
        selection = json.loads(connection.execute('SELECT extension_data FROM sessions WHERE id=?', (sid,)).fetchone()[0])['enabled_extensions.v0']['extensions']
    by_name = {extension['name']: extension for extension in selection}
    assert 'developer' in by_name, 'Legacy restore discarded inherited tools'
    assert by_name['summon']['available_tools'] == ['load'], 'Legacy restore bypassed delegation isolation'
    assert not provider.errors, provider.errors
    (out / 'acceptance.json').write_text(json.dumps(dict(result='PASS', binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
        peak_process_bytes=peak, legacy_tools_preserved=True, historical_pages=True, bounded_rows=True, stream_preserves_page=True,
        send_returns_to_live=True, latest_catches_up=True, complete_export_messages=402,
        fetch_ms={name:load(name)['fetch_ms'] for name in ('first','middle','earlier','later')}), indent=2), encoding='utf-8')
    print('PASS: bounded history pages, live streaming preserves reading, Latest catches up, full export intact')
finally:
    provider.close()
    provider.write_evidence(out)

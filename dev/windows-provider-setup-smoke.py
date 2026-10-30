#!/usr/bin/env python3
"""V52: native provider defaults, unchanged credentials and restart, loopback only."""
import argparse
import hashlib
import json
import re
import sqlite3
from pathlib import Path
import subprocess
import sys
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert sys.platform == 'win32', 'Run native acceptance on Windows only'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--output', type=Path, default=Path('build/provider-setup-smoke'))
args = parser.parse_args()
binary, out = args.binary.resolve(), args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
provider = ProviderFixture()
env = provider.environment(out)
config = out / 'goose/config/config.yaml'
value = json.loads(config.read_text(encoding='utf-8'))
value['CUSTOM_SETTING'] = 'preserve me'
config.write_text(json.dumps(value), encoding='utf-8')
secrets = config.with_name('secrets.yaml')
original_secrets = b'ANTHROPIC_API_KEY: fixture-unused-preserve\n'
secrets.write_bytes(original_secrets)


def control(*args):
    return {'settings': ['goose-setup', *args]}


def dump(name):
    return [control('dump', str(out / (name + '.json'))), {'wait': 300}]


def run(steps, name):
    script = out / (name + '-steps.json')
    script.write_text(json.dumps(steps), encoding='utf-8')
    with (out / (name + '.log')).open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(binary)], env=dict(env, CODEX_GUI_AUTOMATION=str(script)),
                                   stdout=log, stderr=log)
        return ProcessGuard(process, timeout=90).wait()


try:
    peak = run([{'wait_ready': 30000}, {'resize': [1440, 1100]},
                {'open_settings': 'goose'}, {'wait': 1000}, {'click_text': 'Provider defaults'},
                {'wait': 300}, *dump('initial'),
                control('field', 'OPENAI_BASE_URL', 'https://fixture.invalid?key=secret'),
                {'click_text': 'Save provider defaults'}, {'wait': 500}, *dump('invalid'),
                control('field', 'OPENAI_BASE_URL', ''),
                control('field', 'AWS_REGION', 'us-west-2'),
                control('field', 'AWS_ENDPOINT_URL_BEDROCK_RUNTIME', provider.endpoint),
                control('model', 'openai.gpt-6-luna'),
                {'click_text': 'Save provider defaults'}, {'wait': 1000}, *dump('saved'),
                {'snapshot': str(out / 'provider-defaults.png')}, {'quit': True}], 'setup')
    initial = json.loads((out / 'initial.json').read_text(encoding='utf-8'))
    invalid = json.loads((out / 'invalid.json').read_text(encoding='utf-8'))
    saved = json.loads((out / 'saved.json').read_text(encoding='utf-8'))
    assert initial['loaded'] and not initial['busy'] and initial['model'] == 'openai.gpt-6.1-sol', initial
    assert 'cannot contain' in invalid['status'], invalid
    assert 'Saved in Goose' in saved['status'] and saved['model'] == 'openai.gpt-6-luna', saved
    assert saved['fields']['AWS_REGION'] == 'us-west-2', saved
    assert secrets.read_bytes() == original_secrets, 'Setup changed credentials'
    assert 'preserve me' in config.read_text(encoding='utf-8'), 'Setup lost unrelated configuration'
    peak = max(peak, run([{'wait_ready': 30000}, {'new_thread': str(out / 'project')},
                {'wait_idle': 30000}, control('choose', 'provider', 'codex'), {'wait': 700}, *dump('blocked-provider'),
                {'send': 'Verify saved provider defaults'},
                {'wait_idle': 30000}, {'wait': 1200},
                {'transcript': 'export-complete:' + str(out / 'conversation.md')}, {'wait': 500},
                {'open_settings': 'goose'}, {'wait': 1000}, *dump('restored'), {'quit': True}], 'restart'))
    restored = json.loads((out / 'restored.json').read_text(encoding='utf-8'))
    blocked = json.loads((out / 'blocked-provider.json').read_text(encoding='utf-8'))
    assert 'External agent backends' in blocked['sessionError'], blocked
    offered = next(o['values'] for o in blocked['sessionOptions'] if o['id'] == 'provider')
    assert offered and set(offered) <= {'goose', 'aws_bedrock', 'openai', 'anthropic'}, offered
    assert restored['model'] == 'openai.gpt-6-luna', restored
    assert restored['fields']['AWS_REGION'] == 'us-west-2', restored
    assert provider.requests and all('/model/openai.gpt-6-luna/' in r['path'] for r in provider.requests), provider.requests
    assert not provider.errors, provider.errors
    assert secrets.read_bytes() == original_secrets
    assert (out / 'provider-defaults.png').stat().st_size > 1000
    # Unsupported persisted/default provider names must be rejected before
    # Goose constructs an agent. Use a nonexistent fixture name, never a CLI.
    database_path, = (out / 'goose').rglob('sessions.db')
    with sqlite3.connect(database_path) as database:
        conversation_id, = database.execute('SELECT id FROM sessions WHERE provider_name = ?', ('aws_bedrock',)).fetchone()
        database.execute('UPDATE sessions SET provider_name = ? WHERE id = ?', ('fastrock_fixture_disallowed', conversation_id))
        preserved = database.execute('SELECT * FROM sessions WHERE id = ?', (conversation_id,)).fetchone()
        count, = database.execute('SELECT count(*) FROM sessions').fetchone()
    peak = max(peak, run([{'wait_ready': 30000}, {'resume': 'goose:' + conversation_id}, {'wait': 1200},
                *dump('blocked-restore'), {'quit': True}], 'blocked-restore'))
    blocked_restore = json.loads((out / 'blocked-restore.json').read_text(encoding='utf-8'))
    assert 'External agent backends' in (blocked_restore['threadError'] or ''), blocked_restore
    with sqlite3.connect(database_path) as database:
        assert database.execute('SELECT * FROM sessions WHERE id = ?', (conversation_id,)).fetchone() == preserved
    value, replacements = re.subn(r'(?m)^active_provider:.*$', 'active_provider: fastrock_fixture_disallowed',
                                  config.read_text(encoding='utf-8'))
    assert replacements == 1, 'Fixture did not find the saved Goose provider'
    config.write_text(value, encoding='utf-8')
    peak = max(peak, run([{'wait_ready': 30000}, {'new_thread': str(out / 'project')}, {'wait': 1200},
                *dump('blocked-default'), {'quit': True}], 'blocked-default'))
    blocked_default = json.loads((out / 'blocked-default.json').read_text(encoding='utf-8'))
    assert 'External agent backends' in (blocked_default['threadError'] or ''), blocked_default
    with sqlite3.connect(database_path) as database:
        assert database.execute('SELECT count(*) FROM sessions').fetchone() == (count,)
    assert secrets.read_bytes() == original_secrets
    (out / 'acceptance.json').write_text(json.dumps(dict(result='PASS',
        binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), peak_process_bytes=peak,
        native_provider_setup=True, invalid_url_rejected=True, credentials_preserved=True,
        restart_uses_saved_model=True, external_provider_refused_before_dispatch=True,
        native_provider_choices_scoped=True, unsupported_restore_preserves_history=True,
        unsupported_default_cannot_create_session=True), indent=2), encoding='utf-8')
    print('PASS: native provider defaults, URL validation, credentials preserved, restart uses saved model')
finally:
    provider.close()
    provider.write_evidence(out)

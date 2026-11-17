#!/usr/bin/env python3
"""Native Foundry profile save, restart and Azure CLI auth. Uses a local fake CLI."""
import argparse
import json
import os
from pathlib import Path
import subprocess

from embedded_provider_fixture import ProcessGuard, ProviderFixture

assert os.name == 'nt', 'Native acceptance runs on Windows only'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
binary, out = args.binary.resolve(), args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
subscription = '12345678-1234-1234-1234-123456789abc'
endpoint = 'https://fixture.services.ai.azure.com/api/projects/fixture'
deployment = 'flash-work-deployment'
fixture_token = 'fixture.foundry-token'
cli = out / 'cli tools'
cli.mkdir()
trace = out / 'cli-arguments.txt'
(cli / 'az.cmd').write_text(
    '@echo off\n'
    '> "%FASTROCK_FIXTURE_CLI_TRACE%" echo %*\n'
    f'echo {fixture_token}\n',
    encoding='ascii',
)


def profile(*parts):
    return {'settings': ['profiles', *parts]}


def launch(env, name, steps):
    script = out / f'{name}-steps.json'
    script.write_text(json.dumps([
        {'wait_ready': 30000}, {'open_settings': 'goose'}, {'wait': 800},
        *steps, profile('dump', str(out / f'{name}.json')), {'wait': 500}, {'quit': True},
    ]), encoding='utf-8')
    with (out / f'{name}.log').open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(binary)], env=dict(env, CODEX_GUI_AUTOMATION=str(script)),
                                   stdout=log, stderr=log)
        ProcessGuard(process, timeout=90).wait()
    return json.loads((out / f'{name}.json').read_text(encoding='utf-8'))


provider = ProviderFixture()
try:
    env = provider.environment(out)
    env['PATH'] = str(cli) + os.pathsep + env['PATH']
    env['FASTROCK_FIXTURE_CLI_TRACE'] = str(trace)
    created = launch(env, 'created', [
        # Let the queued Slint selection callback finish clearing the editor.
        profile('new'), {'wait': 100}, profile('name', 'Work Foundry'), profile('engine', 'azure-foundry'),
        profile('url', endpoint), profile('model', deployment),
        profile('azure-subscription', subscription), profile('save'), {'wait': 1200},
    ])
    assert created['secretEmpty'] and 'Azure CLI sign-in' in created['status'], created
    assert len(created['profiles']) == 1, created
    entry = created['profiles'][0]
    assert entry['engine'] == 'azure-foundry' and entry['azure_subscription'] == subscription, entry
    assert entry['url'] == endpoint and entry['model'] == deployment, entry
    config_path = out / 'goose/config/custom_providers' / f"{entry['id']}.json"
    config = json.loads(config_path.read_text(encoding='utf-8'))
    assert config['api_key_env'] == '', config
    assert config['base_path'] == 'api/projects/fixture/openai/v1/responses', config
    assert config['auth']['args'] == ['--fastrock-foundry-credential', entry['credential_id'], subscription]
    restored = launch(env, 'restored', [])
    assert restored['profiles'] == created['profiles'], restored
    assert not trace.exists(), 'Setup or restart executed Azure CLI before a model request'
    result = subprocess.run([str(binary), *config['auth']['args']], env=env, capture_output=True, timeout=20)
    assert result.returncode == 0 and result.stdout.strip() == fixture_token.encode(), result.stderr
    assert trace.read_text(encoding='ascii').strip().split() == [
        'account', 'get-access-token', '--subscription', subscription, '--scope', 'https://ai.azure.com/.default',
        '--query', 'accessToken', '--output', 'tsv', '--only-show-errors',
    ]
    assert fixture_token not in config_path.read_text(encoding='utf-8'), 'Token persisted in the profile'
    print('PASS: native Foundry profile, restart and scoped Azure CLI credential process', flush=True)
finally:
    provider.close()

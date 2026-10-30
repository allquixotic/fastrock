#!/usr/bin/env python3
"""Native edits/revision conflicts against embedded vwork and its real redb.

The local model requests actual vwork tools to seed and edit the workspace.
No daemon, fake storage service, or installed Goose/vwork.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sqlite3
import sys
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert sys.platform == 'win32', 'GUI acceptance runs only on Windows'
root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--output', type=Path, default=root / 'build/vwork-smoke')
args = parser.parse_args()
binary, out = args.binary.resolve(), args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
repo = out / 'project'
provider = ProviderFixture(workspace_repo=repo)
env = provider.environment(out)
state = out / 'local/vwork'
missing_vwork = out / 'not-installed/vwork.exe'
config = out / 'goose/config/config.yaml'
value = json.loads(config.read_text(encoding='utf-8'))
value['extensions'] = {'vwork': dict(enabled=True, name='vwork', type='stdio', cmd=str(missing_vwork),
                                    args=['--home', str(state), 'mcp'], timeout=30)}
config.write_text(json.dumps(value), encoding='utf-8')
(repo / 'src').mkdir()
original = b'\xef\xbb\xbffn main() {}\r\n'
(repo / 'src/main.rs').write_bytes(original)
# Git prepares only the fixture input. Fastrock imports it through embedded
# vwork; repository/global hooks, credential helpers and filters are disabled.
git_env = {key: value for key, value in os.environ.items() if not key.upper().startswith('GIT_')}
git_env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=str(out / 'aws-empty'))
for command in (['init', '.'], ['add', 'src/main.rs'], ['update-index', '--chmod=+x', 'src/main.rs'],
                ['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-m', 'Fixture']):
    subprocess.run(['git', '-c', 'core.hooksPath=NUL', '-c', 'core.autocrlf=false', *command],
                   cwd=repo, env=git_env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)


def virtual(*arguments):
    return {'virtual_workspaces': list(arguments)}


def turn(prompt):
    return [{'send': prompt}, {'wait': 300}, {'wait_idle': 30000}, {'wait': 300}]


def run(steps, name, environment):
    script = out / (name + '-steps.json')
    script.write_text(json.dumps(steps), encoding='utf-8')
    with (out / (name + '.log')).open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(binary), '--remote', 'goose'], cwd=root,
            env=dict(environment, CODEX_GUI_AUTOMATION=str(script)), stdout=log, stderr=log)
        return ProcessGuard(process, timeout=120).wait()


def dump(name):
    return [virtual('dump', str(out / (name + '.json'))), {'wait': 300}]


try:
    peak = run([{'wait_ready': 30000}, {'new_thread': str(repo)}, {'wait_idle': 30000},
                *turn('vwork-seed'), virtual('action', 'virtual-refresh'), {'wait': 1000}, virtual('open'),
                virtual('action', 'virtual-compact'), {'wait': 1000}, *dump('database-only'),
                virtual('select', '1'), {'wait': 500}, virtual('file', '0'), {'wait': 500}, *dump('original'),
                virtual('action', 'virtual-edit:edited\r\n'), virtual('action', 'virtual-compact'),
                {'wait': 1000}, *dump('compacted'), *turn('vwork-conflict'),
                virtual('action', 'virtual-save'), {'wait': 500}, *dump('conflict'),
                virtual('snapshot', str(out / 'virtual.png')), {'wait': 300},
                virtual('action', 'virtual-discard'), virtual('action', 'virtual-reload'), {'wait': 500},
                virtual('action', 'virtual-edit:edited\r\n'), virtual('action', 'virtual-save'),
                {'wait': 500}, *dump('saved'), *turn('vwork-inspect'),
                virtual('action', 'virtual-fork:review-child'), {'wait': 500},
                virtual('file', '0'), {'wait': 500},
                virtual('action', 'virtual-edit:child change λ\r\n'), virtual('action', 'virtual-save'),
                {'wait': 500}, virtual('select', '1'), {'wait': 500}, virtual('file', '0'), {'wait': 500},
                virtual('action', 'virtual-merge-source:review-child'),
                virtual('action', 'virtual-merge-review'), {'wait': 500}, *dump('merge-review'),
                virtual('snapshot', str(out / 'merge-review.png')), {'wait': 300},
                virtual('action', 'virtual-edit:unsaved review draft'), *dump('merge-draft'),
                virtual('action', 'virtual-discard'), virtual('action', 'virtual-merge-review'), {'wait': 500},
                virtual('action', 'virtual-merge-apply'), {'wait': 500}, *dump('merged'),
                virtual('file', '0'), {'wait': 500}, *dump('merged-file'),
                virtual('action', 'virtual-chat'), {'wait': 800}, {'wait_idle': 30000},
                *turn('vwork-bound-parent'), {'tab_action': 'worktree'}, {'wait': 800}, {'wait_idle': 30000},
                *turn('vwork-child-edit'), {'transcript': 'export-complete:' + str(out / 'fork-history.md')}, {'wait': 300},
                {'quit': True}], 'enabled', env)
    load = lambda name: json.loads((out / (name + '.json')).read_text(encoding='utf-8'))
    database = load('database-only')
    assert database['visible'] and database['canCompact'] and database['report'], database
    assert load('original')['text'] == '\ufefffn main() {}\r\n', load('original')
    compacted = load('compacted')
    assert compacted['dirty'] and compacted['text'] == 'edited\r\n' and compacted['report'], compacted
    conflict, saved = load('conflict'), load('saved')
    assert conflict['dirty'] and conflict['text'] == 'edited\r\n', conflict
    assert 'revision' in conflict['status'].lower() and 'conflict' in conflict['status'].lower(), conflict
    assert saved['visible'] and not saved['dirty'] and saved['text'] == 'edited\r\n', saved
    inspected = [r['result'] for r in provider.workspace_results if r['prompt'] == 'vwork-inspect']
    assert len(inspected) == 2, inspected
    assert base64.b64decode(inspected[0]['base64']) == b'edited\r\n', inspected
    assert inspected[1]['entries'][0]['mode'] == '100755', inspected
    assert (repo / 'src/main.rs').read_bytes() == original, 'Virtual edits changed the source checkout'
    assert (state / 'workspaces.redb').is_file() and not (state / 'connection.json').exists()
    assert not missing_vwork.exists(), 'Fixture unexpectedly installed vwork'
    assert (out / 'virtual.png').stat().st_size > 1000
    review, merged = load('merge-review'), load('merged')
    assert review['canMerge'] and review['widgetCanMerge'] and 'child change λ' in review['report'], review
    assert not load('merge-draft')['canMerge'] and not load('merge-draft')['widgetCanMerge'], load('merge-draft')
    assert not merged['canMerge'] and 'Merge applied' in merged['report'], merged
    assert load('merged-file')['text'] == 'child change λ\r\n', load('merged-file')
    assert (out / 'merge-review.png').stat().st_size > 1000
    bindings = [json.loads(p.read_text(encoding='utf-8')) for p in out.rglob('*.binding.json')]
    assert len(bindings) == 2, bindings
    child_binding = next(b for b in bindings if b['forked_from'])
    parent_binding = next(b for b in bindings if not b['forked_from'])
    assert parent_binding['workspace']['selection']['workspace'] == 'task', bindings
    assert child_binding['workspace']['selection']['workspace'].startswith('fork-'), bindings
    assert parent_binding['workspace']['cwd'] != child_binding['workspace']['cwd'], bindings
    parent_id = child_binding['forked_from']
    database_path, = (out / 'goose').rglob('sessions.db')
    with sqlite3.connect(database_path) as database:
        rows = database.execute('SELECT id, working_dir FROM sessions').fetchall()
        child_id, = [sid for sid, cwd in rows if cwd == child_binding['workspace']['cwd']]
        parent_messages = database.execute('SELECT content_json FROM messages WHERE session_id = ? ORDER BY id', (parent_id,)).fetchall()
        child_messages = database.execute('SELECT content_json FROM messages WHERE session_id = ? ORDER BY id', (child_id,)).fetchall()
        assert child_messages[:len(parent_messages)] == parent_messages, 'Fork lost conversation history'
    peak = max(peak, run([{'wait_ready': 30000}, {'resume': 'goose:' + parent_id}, {'wait': 800},
        {'wait_idle': 30000}, {'click_text': 'Resume'}, {'wait': 500}, *turn('vwork-parent-restored'),
        {'resume': 'goose:' + child_id}, {'wait': 800}, {'wait_idle': 30000},
        {'click_text': 'Resume'}, {'wait': 500}, *turn('vwork-child-restored'), {'wait': 1000},
        {'sidebar': ['dump', str(out / 'project-groups.json')]}, {'quit': True}], 'restored', env))
    for prompt, expected in [('vwork-bound-parent', b'child change \xce\xbb\r\n'),
                             ('vwork-parent-restored', b'child change \xce\xbb\r\n'),
                             ('vwork-child-restored', b'fork isolated \xce\xbb\r\n')]:
        result, = [r['result'] for r in provider.workspace_results if r['prompt'] == prompt]
        assert base64.b64decode(result['base64']) == expected, (prompt, result)
    assert (repo / 'src/main.rs').read_bytes() == original, 'Fork touched source checkout'
    groups = load('project-groups')['projectFolders']
    assert any(row['count'] >= 2 and Path(row['path']).name == 'project' for row in groups), groups
    assert not any('fastrock-agent-workdirs' in row['path'] for row in groups), groups
    # Vwork is always available in 0.4.0, including without legacy configuration.
    default_env = provider.environment(out / 'default')
    peak = max(peak, run([{'wait_ready': 30000}, {'wait': 1000}, virtual('open'),
                         {'open_folder': str(repo)}, {'wait': 1000}, {'wait_idle': 30000},
                         *turn('vwork-project-import'),
                         *dump('default'), {'quit': True}], 'default', default_env))
    assert load('default')['visible'] and load('default')['canCompact'], load('default')
    imported, = [r['result'] for r in provider.workspace_results if r['prompt'] == 'vwork-project-import']
    assert base64.b64decode(imported['base64']) == original, imported
    (out / 'acceptance.json').write_text(json.dumps(dict(result='PASS', backend='embedded-goose-vwork',
        binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), peak_process_bytes=peak,
        native_window=True, compaction_preserves_draft=True, conflict_preserves_draft=True,
        revision_checked_save=True, source_checkout_preserved=True, no_backend_installed=True,
        default_embedded_database=True, reviewed_child_merge=True, bound_conversation=True,
        isolated_conversation_fork=True, copied_history=True, binding_survives_restart=True,
        repository_project_groups=True, native_repository_registration=True), indent=2), encoding='utf-8')
    print('PASS: embedded vwork, native edits/compaction/conflicts, source preservation, no installed backends')
finally:
    provider.close()
    provider.write_evidence(out)

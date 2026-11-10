#!/usr/bin/env python3
"""V85: native Compact uses real Goose compaction, retains history and survives restart."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import threading
from urllib.parse import unquote
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert os.name == 'nt', 'Run native acceptance on Windows only'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
binary, out = a.binary.resolve(), a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
SEED = 'ORIGINAL_CONTEXT_ZIRCON ' + 'retained history detail ' * 150
SUMMARY = 'COMPACTED_CONTEXT_QUARTZ: remember the original task and pending work.'
MODEL = 'us.anthropic.claude-opus-5-5-v1:0'


class CompactionFixture(ProviderFixture):
    def __init__(self):
        super().__init__()
        self.compactions = []
        self.mode = 'success'
        self.release = threading.Event()

    def respond(self, handler, body):
        payload = json.dumps(body)
        manual = ('Please summarize the conversation history provided in the system prompt.' in payload
                  and 'search index sidecar' not in payload)
        if not manual:
            return super().respond(handler, body)
        self.compactions.append(dict(path=handler.path, body=body, mode=self.mode))
        assert not body.get('toolConfig'), 'Compaction exposed agent tools'
        assert unquote(handler.path) == f'/model/{MODEL}/converse', 'Compaction lost the selected provider/model'
        if self.mode == 'failure':
            return self.json_response(handler, {'message': 'COMPACTION_FIXTURE_FAILURE', '__type': 'ValidationException'}, 400)
        if self.mode == 'cancel':
            self.release.wait(25)
        return self.json_response(handler, dict(output={'message': {'role': 'assistant', 'content': [{'text': SUMMARY}]}},
            stopReason='end_turn', usage={'inputTokens': 1000, 'outputTokens': 15, 'totalTokens': 1015}, metrics={'latencyMs': 1}))


provider = CompactionFixture()
env = provider.environment(out)
env['FASTROCK_WEBHOOK_BIND'] = '127.0.0.1:0'
config = out / 'goose/config/config.yaml'
settings = json.loads(config.read_text(encoding='utf-8'))
settings['GOOSE_MODEL'] = MODEL
config.write_text(json.dumps(settings), encoding='utf-8')


def export(name):
    return [{'transcript': 'export-complete:' + str(out / (name + '.md'))}, {'wait': 400}]


def run(name, steps):
    path = out / (name + '-steps.json')
    path.write_text(json.dumps(steps), encoding='utf-8')
    with (out / (name + '.log')).open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(binary)], env=dict(env, CODEX_GUI_AUTOMATION=str(path)), stdout=log, stderr=log)
        return ProcessGuard(process, timeout=120).wait()


def stored():
    database = next((out / 'goose').rglob('sessions.db'))
    with sqlite3.connect(database.as_uri() + '?mode=ro', uri=True) as conn:
        return conn.execute('SELECT message_id,session_id,content_json,metadata_json FROM messages ORDER BY id').fetchall()


def metadata(row): return json.loads(row[3])
def text(name): return (out / (name + '.md')).read_text(encoding='utf-8')
def open_session():
    return [{'wait_ready': 30000}, {'resume': sid}, {'wait_idle': 30000}, {'wait_summaries': 30000},
]


try:
    peak = run('compact', [
        {'wait_ready': 30000}, {'resize': [1440, 1000]}, {'new_thread': str(out / 'project')}, {'wait_idle': 30000},
        {'send': SEED}, {'wait_idle': 30000}, {'wait_summaries': 30000},
        {'key': 'ctrl+shift+P'}, {'wait': 200}, {'palette': ['query', 'compact conversation']},
        {'wait': 150}, {'click_text': 'Compact conversation'}, {'wait': 150},
        {'wait_idle': 30000}, {'wait_summaries': 30000},
        *export('compacted'), {'snapshot': str(out / 'compacted.png')}, {'quit': True}])
    assert len(provider.compactions) == 1, 'Native Compact never reached Goose compaction'
    assert 'Compaction complete' in text('compacted') and 'Could not compact' not in text('compacted')
    assert SEED in text('compacted'), 'Compaction lost visible source history'
    assert SUMMARY not in text('compacted'), 'Agent-only summary leaked into the transcript'
    assert sum(r['summary'] for r in provider.requests) == 1, 'Manual compaction queued a search summary'
    before = stored()
    original = [row for row in before if 'ORIGINAL_CONTEXT_ZIRCON' in row[2]]
    assert original and all(metadata(row)['userVisible'] and not metadata(row)['agentVisible'] for row in original)
    compacted = [row for row in before if SUMMARY in row[2]]
    assert compacted and all(not metadata(row)['userVisible'] and metadata(row)['agentVisible'] for row in compacted)
    sid = 'goose:' + original[0][1]
    peak = max(peak, run('restart', [*open_session(), {'send': 'Continue after compact restart'}, {'wait_idle': 30000},
        {'wait_summaries': 30000}, {'composer': ['type', '/compact']}, {'composer': ['send']},
        {'wait_idle': 30000}, *export('restarted'), {'quit': True}]))
    continuation = next(r for r in provider.requests if not r['summary'] and r['prompt'] == 'Continue after compact restart')
    context = json.dumps(continuation['body']['messages'])
    assert SUMMARY in context and 'ORIGINAL_CONTEXT_ZIRCON' not in context, 'Restart did not use compacted model context'
    assert len(provider.compactions) == 2 and text('restarted').count('Compaction complete') >= 2
    assert SEED in text('restarted'), 'Restart lost original visible history'

    before_failure = {row[0]: row for row in stored()}
    provider.mode = 'failure'
    peak = max(peak, run('failure', [*open_session(), {'tab_action': 'compact'}, {'wait_idle': 30000},
        *export('failed'), {'quit': True}]))
    assert len(provider.compactions) == 3 and 'COMPACTION_FIXTURE_FAILURE' in text('failed')
    assert text('failed').count('Compaction complete') == 2, 'Failed compaction claimed success'
    after_failure = {row[0]: row for row in stored()}
    assert all(after_failure[mid] == row for mid, row in before_failure.items()), 'Failed compaction changed original context/history'

    provider.mode = 'cancel'
    before_cancel = {row[0]: row for row in stored()}
    peak = max(peak, run('cancel', [*open_session(), {'tab_action': 'compact'}, {'wait': 1500},
        {'click_text': 'Stop'}, {'wait_idle': 10000}, *export('cancelled'), {'quit': True}]))
    provider.release.set()
    assert len(provider.compactions) == 4, 'Cancellation did not exercise a stalled compaction'
    assert text('cancelled').count('Compaction complete') == 2, 'Cancelled compaction claimed success'
    after_cancel = {row[0]: row for row in stored()}
    assert all(after_cancel[mid] == row for mid, row in before_cancel.items()), 'Cancelled compaction changed existing context/history'

    settings['GOOSE_SLASH_COMMANDS_ENABLED'] = False
    config.write_text(json.dumps(settings), encoding='utf-8')
    peak = max(peak, run('disabled', [{'wait_ready': 30000}, {'new_thread': str(out / 'project')},
        {'wait_idle': 30000}, {'tab_action': 'compact'}, {'wait_idle': 30000},
        {'transcript': 'export:' + str(out / 'disabled.md')}, {'wait': 400}, {'quit': True}]))
    assert len(provider.compactions) == 4, 'Disabled command invoked compaction'
    assert 'Manual compaction requires slash commands' in text('disabled')
    assert not any(not r['summary'] and r['prompt'] == '/compact' for r in provider.requests), 'Compaction fell through to ordinary inference'
    assert not provider.errors, provider.errors
    receipt = dict(result='PASS', binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), peak_process_bytes=peak,
        native_palette_tab_action_and_slash=True, selected_model_preserved=True, model_context_replaced=True,
        visible_history_preserved=True, restart_preserves_context=True, no_search_summary_after_compact=True,
        failed_compaction_preserves_history=True, stop_cancels_compaction=True, disabled_commands_refused=True)
    (out / 'acceptance.json').write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
    print('PASS: native compaction, retained history, restart, failure, Stop and disabled-command guard', flush=True)
finally:
    provider.release.set()
    provider.close()
    provider.write_evidence(out)
    (out / 'compaction-requests.json').write_text(json.dumps(provider.compactions, indent=2) + '\n', encoding='utf-8')

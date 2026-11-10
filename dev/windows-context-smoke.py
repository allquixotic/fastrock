#!/usr/bin/env python3
"""V104/V105: actual embedded context report and native thinking controls."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
from embedded_provider_fixture import ProviderFixture, ProcessGuard, event_frame

assert os.name == 'nt', 'Native acceptance runs on Windows only'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
binary, out = a.binary.resolve(), a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
THOUGHT = 'VISIBLE_THINKING_050: compare the fixture choices, then answer. λ'


class ContextFixture(ProviderFixture):
    def respond(self, handler, body):
        messages = body.get('messages', [])
        prompt = next((' '.join(c.get('text', '') for c in m['content'])
                       for m in reversed(messages) if m['role'] == 'user'), '')
        if not prompt.startswith('context-thinking') or 'search index sidecar' in json.dumps(body):
            return super().respond(handler, body)
        self.requests.append(dict(path=handler.path, summary=False, prompt=prompt, body=body))
        assert handler.path.endswith('/converse-stream'), handler.path
        handler.send_response(200)
        handler.send_header('Content-Type', 'application/vnd.amazon.eventstream')
        handler.end_headers()

        def event(kind, value):
            handler.wfile.write(event_frame(kind, value))
            handler.wfile.flush()

        event('messageStart', {'role': 'assistant'})
        for text in (THOUGHT[:25], THOUGHT[25:]):
            event('contentBlockDelta', {'contentBlockIndex': 0, 'delta': {'reasoningContent': {'text': text}}})
            time.sleep(.05)
        event('contentBlockDelta', {'contentBlockIndex': 0, 'delta': {'reasoningContent': {'signature': 'fixture-signature'}}})
        event('contentBlockStop', {'contentBlockIndex': 0})
        event('contentBlockDelta', {'contentBlockIndex': 1, 'delta': {'text': 'Context fixture answer.'}})
        event('contentBlockStop', {'contentBlockIndex': 1})
        event('messageStop', {'stopReason': 'end_turn'})
        event('metadata', {'usage': {'inputTokens': 30, 'outputTokens': 40, 'totalTokens': 70}, 'metrics': {'latencyMs': 1}})


provider = ContextFixture()
env = provider.environment(out, streaming=True)
config_path = out / 'goose/config/config.yaml'
config = json.loads(config_path.read_text(encoding='utf-8'))
config['GOOSE_MODEL'] = 'us.anthropic.claude-opus-5-5-v1:0'
config['FASTROCK_CHEAP_PROFILE'] = dict(name='Context fixture cheap', provider='aws_bedrock', model='us.openai.gpt-6-luna')
config_path.write_text(json.dumps(config), encoding='utf-8')


def snapshot(name):
    return [{'context_report': ['snapshot', str(out / (name + '.json'))]}, {'wait': 300}]


def run(name, steps):
    path = out / (name + '-steps.json')
    path.write_text(json.dumps(steps), encoding='utf-8')
    with (out / (name + '.log')).open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(binary)], env=dict(env, CODEX_GUI_AUTOMATION=str(path)), stdout=log, stderr=log)
        return ProcessGuard(process, timeout=150).wait()


def read(name):
    return json.loads((out / (name + '.json')).read_text(encoding='utf-8'))


try:
    peak = run('context', [
        {'wait_ready': 30000}, {'resize': [1440, 1000]}, {'new_thread': str(out / 'project')}, {'wait_idle': 30000},
        {'send': 'context-thinking-one'}, {'wait_idle': 30000}, {'wait_summaries': 30000},
        {'click_text': 'View context window'}, {'wait_context': 30000}, *snapshot('expanded'),
        {'click_text': 'Refresh'}, {'wait_context': 30000}, *snapshot('refreshed'),
        {'snapshot': str(out / 'context.png')}, {'key': 'escape'}, {'wait': 200},
        {'open_settings': 'appearance'}, {'wait': 300}, {'settings': ['collapse-thinking', 'true']},
        {'snapshot': str(out / 'settings.png')}, {'select_tab': 0}, {'wait': 300},
        {'send': 'context-thinking-two'}, {'wait_idle': 30000}, {'wait_summaries': 30000}, *snapshot('collapsed'),
        {'transcript': 'expand-all'}, {'wait': 200}, *snapshot('manual'),
        {'transcript': 'export-complete:' + str(out / 'transcript.md')}, {'wait': 400}, {'quit': True},
    ])
    expanded, refreshed, collapsed, manual = map(read, ('expanded', 'refreshed', 'collapsed', 'manual'))
    assert expanded['rows'] and not expanded['busy'], expanded
    assert 'tokens' in expanded['summary'] and 'claude-opus' in expanded['summary'], expanded['summary']
    assert 'Local tokenizer estimate' in expanded['note']
    assert any('messages' in r['detail'] for r in expanded['rows'])
    assert any('tool definitions' in r['detail'] for r in expanded['rows'])
    assert any('context-thinking-one' in r['preview'] for r in expanded['rows']), 'Missing inspectable message preview'
    assert expanded['thinking'] and all(t['expanded'] for t in expanded['thinking']), expanded['thinking']
    assert not expanded['collapseThinking']
    assert refreshed['summary'] == expanded['summary'], 'Refresh changed unchanged context'
    assert collapsed['collapseThinking'] and len(collapsed['thinking']) >= 2
    assert all(not t['expanded'] for t in collapsed['thinking']), collapsed['thinking']
    assert all(t['expanded'] for t in manual['thinking']), manual['thinking']
    assert THOUGHT in (out / 'transcript.md').read_text(encoding='utf-8'), 'Export lost thinking text'
    assert len([r for r in provider.requests if not r['summary']]) == 2, 'Context inspection dispatched inference'
    assert json.loads((out / 'codex/gui.json').read_text())['collapse_thinking'] is True
    sid = expanded['thread']
    before = len(provider.requests)
    peak = max(peak, run('restart', [
        {'wait_ready': 30000}, {'resume': sid}, {'wait_idle': 30000}, {'wait_summaries': 30000},
        {'context_report': ['open']}, {'wait_context': 30000}, *snapshot('paused'),
        {'key': 'escape'}, {'wait': 200},  {'wait_goose': 30000},
        {'context_report': ['open']}, {'wait_context': 30000}, *snapshot('restored'),
        {'new_thread': str(out / 'project')}, {'wait_idle': 30000}, *snapshot('switched'), {'quit': True},
    ]))
    paused, restored, switched = map(read, ('paused', 'restored', 'switched'))
    assert paused['rows'] and not paused['status'].startswith('Resume'), paused
    assert restored['rows'] and restored['collapseThinking'], restored
    assert restored['thinking'] and all(not t['expanded'] for t in restored['thinking'])
    assert not switched['open'] and not switched['rows'], 'Other conversation retained context report'
    assert len(provider.requests) == before, 'Restore/report replayed a model request'
    receipt = dict(result='PASS', binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), peak_process_bytes=peak,
                   native_report=True, previews=True, no_inference=True, thinking_opt_in=True,
                   manual_expansion=True, export_preserved=True, persisted_restart=True, restored_lazy_activation=True,
                   conversation_switch_clears_report=True)
    (out / 'acceptance.json').write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
    print('PASS: native context report, thinking settings, history and Resume guards', flush=True)
finally:
    provider.close()
    provider.write_evidence(out)

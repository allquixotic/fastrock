#!/usr/bin/env python3
"""Real embedded multi-tool turns preempt a stalled search-summary request."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import threading
import time
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert os.name == 'nt', 'Native acceptance runs only on Windows'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--output', type=Path, required=True)
args = p.parse_args()
binary, out = args.binary.resolve(), args.output.resolve()
out.mkdir(parents=True, exist_ok=False)


class PriorityFixture(ProviderFixture):
    def __init__(self):
        super().__init__(tools=True)
        self.summary_started = threading.Event()
        self.release_summary = threading.Event()
        self.in_turn = False
        self.final_answer = False
        self.rounds = 0
        self.audit = []

    def respond(self, handler, body):
        system = ' '.join(c.get('text', '') for c in body.get('system', []))
        summary = 'search index sidecar' in json.dumps(body)
        naming = 'Generate a short title (four words or less)' in system
        self.audit.append(dict(kind='summary' if summary else 'naming' if naming else 'foreground',
                               at=time.monotonic(), in_turn=self.in_turn))
        if naming:
            # Real default configuration, including Goose's independent naming.
            return self.answer(handler, [{'text': 'Priority fixture conversation'}])
        if summary:
            assert not self.in_turn, 'A search summary started between tool calls'
            if not self.summary_started.is_set():
                self.summary_started.set()
                assert self.release_summary.wait(25), 'Foreground did not preempt the stalled summary'
            return super().respond(handler, body)
        prompt = ' '.join(c.get('text', '') for m in body['messages'] for c in m['content'])
        if 'priority-tool-turn' not in prompt:
            return super().respond(handler, body)
        assert self.summary_started.is_set(), 'Regression did not exercise in-flight backfill contention'
        self.in_turn = True
        self.release_summary.set()
        self.rounds += 1
        assert self.rounds <= 4, 'Unexpected provider loop or enrichment request'
        time.sleep(0.3)  # Leave time for an incorrectly triggered summary to start.
        if self.rounds <= 3:
            names = [t['toolSpec']['name'] for t in body['toolConfig']['tools'] if 'toolSpec' in t]
            tool = next(name for name in names if name.endswith('selection_tool'))
            return self.answer(handler, [{'text': f'Continuing tool step {self.rounds}.'},
                {'toolUse': dict(toolUseId=f'priority-{self.rounds}', name=tool,
                                 input={'command': 'fixture-only'})}], 'tool_use')
        self.in_turn = False
        self.final_answer = True
        return self.answer(handler, [{'text': 'Priority turn finished; control returns to the user.'}])

    def answer(self, handler, content, reason='end_turn'):
        assert handler.path.endswith('/converse'), handler.path
        return self.json_response(handler, dict(output={'message': {'role': 'assistant', 'content': content}},
            stopReason=reason, usage={'inputTokens': 3, 'outputTokens': 4, 'totalTokens': 7}, metrics={'latencyMs': 1}))


provider = PriorityFixture()
env = provider.environment(out)
config_path = out / 'goose/config/config.yaml'
config = json.loads(config_path.read_text())
config.pop('GOOSE_DISABLE_SESSION_NAMING')  # Do not hide upstream auxiliary requests.
config['GOOSE_MODEL'] = 'openai.gpt-6-luna'
config_path.write_text('# Working Bedrock route and user comments must survive startup\n' + json.dumps(config))
steps = [{'wait_ready': 30000}, {'new_thread': str(out / 'project')}, {'wait_idle': 30000},
         {'send': 'Seed a completed conversation'}, {'wait_idle': 10000}, {'wait': 4000},
         {'new_thread': str(out / 'project')}, {'wait_idle': 10000},
         {'send': 'priority-tool-turn'}, {'wait_idle': 10000}, {'wait_summaries': 20000},
         {'transcript': 'export-complete:' + str(out / 'transcript.md')}, {'wait': 300},
         {'open_settings': 'diagnostics'}, {'wait': 400},
         {'snapshot': str(out / 'diagnostics.png')}, {'quit': True}]
script = out / 'script.json'
script.write_text(json.dumps(steps))
env['CODEX_GUI_AUTOMATION'] = str(script)
try:
    with (out / 'app.log').open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(binary), '--remote', 'goose'], cwd=out / 'project', env=env, stdout=log, stderr=log)
        peak = ProcessGuard(process, timeout=90).wait()
    assert not provider.errors, provider.errors
    assert provider.final_answer and provider.rounds == 4, provider.audit
    assert len(provider.tool_calls) == 3, provider.tool_calls
    summaries = [r for r in provider.audit if r['kind'] == 'summary']
    # Setup and Send each preempt background work; a retry can start between
    # those guards. At least one canceled attempt must be followed by a retry.
    assert 3 <= len(summaries) <= 5, provider.audit
    assert not any(r['in_turn'] for r in summaries), provider.audit
    transcript = (out / 'transcript.md').read_text(encoding='utf-8')
    assert 'Priority turn finished' in transcript and 'ObsidianNeedle' not in transcript
    assert config_path.read_text().startswith('# Working Bedrock route and user comments must survive startup\n')
    receipt = dict(result='PASS', binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                   peak_process_bytes=peak, three_tool_calls_one_completed_turn=True,
                   foreground_preempts_stalled_summary=True, original_comments_preserved=True, requests=provider.audit)
    (out / 'acceptance.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print('PASS: stalled summary preemption, three tool calls, final-turn summary, config comments')
finally:
    provider.release_summary.set()
    provider.close()
    provider.write_evidence(out)

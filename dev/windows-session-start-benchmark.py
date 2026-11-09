#!/usr/bin/env python3
"""Cold and warm native conversation creation against a synthetic Git source.

Uses embedded Goose and a loopback provider, with no model turn or user config.
Run inside the signed-in games desktop through asuser.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
from benchmark_guard import BoundedJob, HARD_CAP, headroom, guest_headroom
from embedded_provider_fixture import ProcessGuard, ProviderFixture

assert os.name == 'nt'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--source', type=Path, required=True)
p.add_argument('--output', type=Path, required=True)
p.add_argument('--host-guard', type=Path, required=True)
a = p.parse_args()
out = a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
provider = ProviderFixture()
env = provider.environment(out)
env.update(CODEX_GUI_PERF='1', FASTROCK_WEBHOOK_BIND='127.0.0.1:0')
steps = [{'wait_ready': 30000}, {'resize': [1440, 1000]}]
for name in ['cold', 'warm', 'warm_again']:
    steps += [{'mark': name + '-begin'}, {'new_thread': str(a.source.resolve())},
              {'wait_idle': 120000}, {'mark': name + '-idle'}, {'wait': 300}]
steps += [{'sidebar': ['dump', str(out / 'sidebar.json')]}, {'wait': 300}, {'quit': True}]
script = out / 'steps.json'
script.write_text(json.dumps(steps), encoding='utf-8')
env['CODEX_GUI_AUTOMATION'] = str(script)
receipt = dict(result='RUNNING', binary_sha256=hashlib.sha256(a.binary.read_bytes()).hexdigest(),
               measurement='native dispatch to idle observation, 100 ms polling; first opens/imports repository')
try:
    cap = min(HARD_CAP, headroom(a.host_guard), guest_headroom())
    with (out / 'app.log').open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(a.binary.resolve())], env=env, stdout=log, stderr=log)
        job = BoundedJob(process, cap)
        guard = ProcessGuard(process, timeout=420, cap=HARD_CAP)
        try:
            def tick():
                headroom(a.host_guard)
                guest_headroom()
                job.sample()
            guard.wait(tick)
        finally:
            receipt.update(peak_process_bytes=guard.peak, peak_tree_commit_bytes=job.peak, cap_bytes=cap)
            job.close()
    marks = {name: float(value) for name, value in re.findall(r'codex-gui perf: (\S+) ([\d.]+) ms', (out / 'app.log').read_text(encoding='utf-8'))}
    receipt['creation_observed_ms'] = {name: round(marks[name + '-idle'] - marks[name + '-begin'], 3)
                                       for name in ['cold', 'warm', 'warm_again']}
    assert not provider.requests, 'Creating conversations must not dispatch a model turn'
    receipt.update(result='PASS', no_provider_turn=True)
except BaseException as error:
    receipt.update(result='FAIL', error=str(error))
    raise
finally:
    provider.close()
    provider.write_evidence(out)
    (out / 'summary.json').write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')

#!/usr/bin/env python3
"""Bounded 1,000-chat / eight-stream / 10,000-message native benchmark.

Use the same seed executable and loopback Bedrock responses for 0.3.x and 0.4.0.
The legacy baseline alone needs --legacy-goose. Hypervisor heartbeat is required.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import time
from benchmark_guard import BoundedJob, GuardStopped, HARD_CAP, headroom, guest_headroom
from embedded_provider_fixture import ProcessGuard, ProviderFixture

assert os.name == 'nt', 'Native benchmark runs only on games'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--seed-executable', type=Path, required=True)
parser.add_argument('--legacy-goose', type=Path)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--host-guard', type=Path, required=True)
parser.add_argument('--frames', type=int, default=360)
parser.add_argument('--divisor', type=int, choices=[1, 10, 100, 1000], default=1)
args = parser.parse_args()
assert 120 <= args.frames <= 600
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
receipt = dict(binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
               seed_sha256=hashlib.sha256(args.seed_executable.read_bytes()).hexdigest(),
               host=platform.node(), os=platform.platform(),
               backend='external Goose 1.57.0' if args.legacy_goose else 'embedded Goose 1.57.0',
               hard_cap_bytes=HARD_CAP, attempts=[])
if args.legacy_goose:
    receipt['goose_sha256'] = hashlib.sha256(args.legacy_goose.read_bytes()).hexdigest()


def save():
    (out / 'summary.json').write_text(json.dumps(receipt, indent=2), encoding='utf-8')


def bounded(command, env, folder, phase, measurements):
    budget = min(HARD_CAP, headroom(args.host_guard), guest_headroom())
    if budget < 128_000_000:
        raise GuardStopped('Insufficient headroom to start fixture')
    headroom_record = dict(cap_bytes=budget, peak_app_bytes=0, peak_tree_commit_bytes=0)
    measurements[phase] = headroom_record
    with (folder / (phase + '.log')).open('w', encoding='utf-8') as log:
        process = subprocess.Popen(command, cwd=folder, env=env, stdout=log, stderr=log)
        job = BoundedJob(process, budget)
        guard = ProcessGuard(process, timeout=300, cap=HARD_CAP)
        def tick():
            headroom(args.host_guard)
            guest_headroom()
            job.sample()
            headroom_record.update(peak_app_bytes=guard.peak, peak_tree_commit_bytes=job.peak)
        try:
            guard.wait(tick)
            tick()
        finally:
            headroom_record.update(peak_app_bytes=guard.peak, peak_tree_commit_bytes=job.peak)
            job.close()
            if process.poll() is None:
                process.kill()
            process.wait(timeout=10)


def distribution(values):
    values = sorted(values)
    assert values
    return {key: round(values[int((len(values) - 1) * fraction)] / 1000, 3)
            for key, fraction in [('p50_ms', .5), ('p95_ms', .95), ('p99_ms', .99)]}


divisor = args.divisor
while divisor <= 1000:
    folder = out / f'scale-{divisor}'
    folder.mkdir()
    sessions, messages, streams = max(1, 1000 // divisor), max(2, 10000 // divisor), max(1, math.ceil(8 / divisor))
    messages -= messages % 2
    attempt = dict(divisor=divisor, stored_conversations=sessions, history_messages=messages,
                   concurrent_streams=streams, phases={})
    receipt['attempts'].append(attempt)
    provider = ProviderFixture(stream_chunks=10000)
    env = provider.environment(folder, streaming=True)
    if args.legacy_goose:
        env['FASTROCK_GOOSE'] = str(args.legacy_goose.resolve())
    env['CODEX_GUI_PERF'] = '1'
    parameters = folder / 'seed-parameters.json'
    parameters.write_text(json.dumps(dict(sessions=sessions, messages=messages,
        project=str(folder / 'project'), vwork_home=str(folder / 'local/vwork'),
        output=str(folder / 'seed.json'))), encoding='utf-8')
    try:
        bounded([str(args.seed_executable.resolve()), '--seed-benchmark', str(parameters)], env, folder, 'seed', attempt['phases'])
        seeded = json.loads((folder / 'seed.json').read_text(encoding='utf-8'))
        steps = [{'wait_ready': 30000}, {'resize': [1440, 1000]}, {'wait': 1000},
                 {'mark': 'history-open-begin'}, {'resume': 'goose:' + seeded['sessions'][0]},
                 {'wait_idle': 120000}, {'mark': 'history-open-idle'}, {'wait': 1000},
                 {'transcript': 'export-complete:' + str(folder / 'complete.md')}, {'wait': 1000},
                 {'snapshot': str(folder / 'history.png')}]
        for index in range(streams):
            steps += [{'new_thread': str(folder / 'project')}, {'wait_idle': 30000},
                      {'send': f'hold benchmark {index}'}, {'wait': 400}]
        steps += [{'mark': 'streams-started'}, {'select_tab': 0}, {'wait': 500},
                  {'transcript': 'scroll-top'}, {'wait': 500},
                  {'scroll_benchmark': dict(frames=args.frames, delta=-12, output=str(folder / 'scroll.json'))},
                  {'snapshot': str(folder / 'streaming-history.png')}]
        # Instrument the synchronous switch work. A 200 ms settle allows native
        # redraws between samples; this is not an input-to-photon measurement.
        for index in range(5):
            steps += [{'select_tab': 1}, {'wait': 200}, {'mark': f'switch-{index}-begin'},
                      {'select_tab': 0}, {'mark': f'switch-{index}-end'}, {'wait': 200}]
        steps += [{'sidebar': ['dump', str(folder / 'sidebar.json')]}, {'wait': 500}, {'quit': True}]
        script = folder / 'steps.json'
        script.write_text(json.dumps(steps), encoding='utf-8')
        env['CODEX_GUI_AUTOMATION'] = str(script)
        bounded([str(args.binary.resolve()), '--remote', 'goose'], env, folder, 'app', attempt['phases'])
        text = (folder / 'complete.md').read_text(encoding='utf-8')
        assert text.count('## User\n') == messages // 2 and text.count('## Assistant\n') == messages // 2, 'Full history lost messages'
        assert 'HISTORY-END-SENTINEL' in text and 'Fixture prompt 0/0' in text, 'History endpoints missing'
        assert provider.peak_streams == streams, ('Not all agents streamed concurrently', provider.peak_streams, streams)
        assert not any(request['summary'] for request in provider.requests), 'Seeded current summaries were unnecessarily regenerated'
        data = json.loads((folder / 'scroll.json').read_text(encoding='utf-8'))
        frames = data['frames'][10:]
        assert len(frames) >= args.frames * .75, 'Insufficient scrolling frames'
        assert abs(data['positions'][-1] - data['positions'][0]) > 1000, 'History did not scroll'
        marks = {name: float(value) for name, value in re.findall(r'codex-gui perf: (\S+) ([\d.]+) ms', (folder / 'app.log').read_text(encoding='utf-8'))}
        attempt.update(result='PASS', peak_concurrent_streams=provider.peak_streams,
            history_open_observed_ms=round(marks['history-open-idle'] - marks['history-open-begin'], 3),
            switch_dispatch_ms=[round(marks[f'switch-{index}-end'] - marks[f'switch-{index}-begin'], 3) for index in range(5)],
            frame=distribution([frame['total_us'] for frame in frames]),
            render=distribution([frame['render_us'] for frame in frames]), event_loop=distribution(data['tick_us'][10:]),
            history_export_messages=messages, current_summaries_preserved=True)
        save()
        print(json.dumps(attempt, indent=2), flush=True)
        break
    except GuardStopped as error:
        attempt.update(result='ABORTED', reason=str(error), next_divisor=divisor * 10)
        save()
        print(f'Guard stopped scale {divisor}: {error}; next workload is tenfold smaller', flush=True)
        divisor *= 10
        time.sleep(2)
    except BaseException as error:
        attempt.update(result='FAILED', reason=str(error))
        save()
        raise
    finally:
        provider.close()
        provider.write_evidence(folder)
else:
    raise SystemExit('No workload completed within available headroom; see summary.json')

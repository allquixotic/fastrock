#!/usr/bin/env python3
"""Compare synthetic repo_fetch calls; only run on games with a host heartbeat."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from benchmark_guard import BoundedJob, GuardStopped, HARD_CAP, headroom, guest_headroom

assert os.name == 'nt', 'Run only on the Windows verification host'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--baseline', type=Path, required=True)
parser.add_argument('--candidate', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--host-guard', type=Path, required=True)
args = parser.parse_args()
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
receipt = dict(result='RUNNING', attempts=[],
               baseline_sha256=hashlib.sha256(args.baseline.read_bytes()).hexdigest(),
               candidate_sha256=hashlib.sha256(args.candidate.read_bytes()).hexdigest())
assert receipt['baseline_sha256'] != receipt['candidate_sha256'], 'Comparison needs distinct binaries'


def save():
    (out / 'summary.json').write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')


def run(binary, command, folder, name):
    cap = min(HARD_CAP, headroom(args.host_guard), guest_headroom())
    record = dict(cap_bytes=cap)
    with (folder / (name + '.jsonl')).open('w', encoding='utf-8') as log, \
         (folder / (name + '.stderr')).open('w', encoding='utf-8') as errors:
        # Attach containment before the child can start its workload.
        launch = [sys.executable, '-c',
                  'import sys,time,subprocess; time.sleep(.5); sys.exit(subprocess.call(sys.argv[1:]))',
                  str(binary.resolve()), *map(str, command)]
        process = subprocess.Popen(launch, stdout=log, stderr=errors)
        job = BoundedJob(process, cap)
        started = time.monotonic()
        try:
            while process.poll() is None:
                headroom(args.host_guard)
                guest_headroom()
                job.sample()
                assert time.monotonic() - started < 900, 'Fixture timeout'
                time.sleep(.1)
            job.sample()
            assert process.returncode == 0, f'{name}: exit {process.returncode}'
            record['events'] = [json.loads(line) for line in
                                (folder / (name + '.jsonl')).read_text(encoding='utf-8').splitlines()]
            # Baseline used an older, verbose register receipt. Keep evidence
            # bounded and free of machine paths/configuration author names.
            for event in record['events']:
                if event.get('operation') == 'register':
                    event['result'] = dict(registered=True)
            return record
        finally:
            record.update(elapsed_seconds=time.monotonic() - started,
                          peak_tree_commit_bytes=job.peak, exit_code=process.poll())
            (folder / (name + '-process.json')).write_text(json.dumps(record, indent=2), encoding='utf-8')
            job.close()
            process.wait(timeout=10)


try:
    for divisor in (1, 10):
        folder = out / f'scale-{divisor}'
        folder.mkdir()
        attempt = dict(divisor=divisor, files=400 // divisor, branches=2000 // divisor + 1)
        receipt['attempts'].append(attempt)
        save()
        try:
            for name, binary in [('baseline', args.baseline), ('candidate', args.candidate)]:
                # Never reuse a source after an update: its unreachable new
                # objects would otherwise be imported during registration.
                source = folder / (name + '-source')
                attempt[name + '_seed'] = run(args.candidate, ['seed', source, divisor], folder, name + '-seed')
                attempt[name] = run(binary, ['run', source, folder / (name + '-store')], folder, name)
                assert attempt[name]['events'][-1]['verified']
            events = {e['operation']: e for e in attempt['candidate']['events'] if 'operation' in e}
            imported = events['updated_fetch']['result']['import']
            assert imported['decoded_objects'] == 3 and imported['reused_objects'] > 0
            for name in ['unchanged_fetch', 'repeat_fetch']:
                assert events[name]['result']['received_objects'] == 0
                assert events[name]['result']['import']['decoded_objects'] == 0
            attempt['result'] = receipt['result'] = 'PASS'
            break
        except GuardStopped as error:
            attempt.update(result='GUARD_STOP', error=str(error))
            save()
    assert receipt['result'] == 'PASS', 'No guarded scale completed'
except BaseException as error:
    receipt.update(result='FAIL', error=str(error))
    raise
finally:
    save()

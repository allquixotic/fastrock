#!/usr/bin/env python3
"""Compare identical synthetic Git workloads with host/guest memory guards.

Compile workspace_performance against both releases. Process I/O counters cover
the whole fixture (concurrent calls overlap); individual elapsed times come from
the fixture itself. The generated repository and stores contain no user data.
"""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
from benchmark_guard import BoundedJob, GuardStopped, HARD_CAP, headroom, guest_headroom
from embedded_provider_fixture import ProcessGuard

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
               candidate_sha256=hashlib.sha256(args.candidate.read_bytes()).hexdigest(),
               io_scope='whole process; includes registration and fsck, concurrent calls overlap')


def save():
    (out / 'summary.json').write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')


class Counters(ctypes.Structure):
    _fields_ = [(name, ctypes.c_ulonglong) for name in
                ('read_operations', 'write_operations', 'other_operations',
                 'read_bytes', 'write_bytes', 'other_bytes')]


def run(binary, command, folder, name):
    cap = min(HARD_CAP, headroom(args.host_guard), guest_headroom())
    record = dict(cap_bytes=cap)
    with (folder / (name + '.jsonl')).open('w', encoding='utf-8') as log, \
         (folder / (name + '.stderr')).open('w', encoding='utf-8') as errors:
        process = subprocess.Popen([str(binary.resolve()), *map(str, command)], stdout=log, stderr=errors)
        job = BoundedJob(process, cap)
        guard = ProcessGuard(process, timeout=900, cap=HARD_CAP)
        api = ctypes.WinDLL('kernel32', use_last_error=True)
        api.GetProcessIoCounters.argtypes = [wintypes.HANDLE, ctypes.POINTER(Counters)]
        started = time.monotonic()
        try:
            def tick():
                headroom(args.host_guard)
                guest_headroom()
                job.sample()
            guard.wait(tick)
            counters = Counters()
            if not api.GetProcessIoCounters(int(process._handle), ctypes.byref(counters)):
                raise ctypes.WinError(ctypes.get_last_error())
            record['io'] = {name: getattr(counters, name) for name, _ in counters._fields_}
            record['events'] = [json.loads(line) for line in (folder / (name + '.jsonl')).read_text(encoding='utf-8').splitlines()]
            return record
        finally:
            record.update(elapsed_seconds=time.monotonic() - started,
                          peak_process_bytes=guard.peak, peak_tree_commit_bytes=job.peak)
            (folder / (name + '-process.json')).write_text(json.dumps(record, indent=2), encoding='utf-8')
            job.close()
            if process.poll() is None:
                process.kill()
            process.wait(timeout=10)


try:
    for divisor in (1, 10, 100):
        folder = out / f'scale-{divisor}'
        folder.mkdir()
        source = folder / 'source'
        attempt = dict(divisor=divisor, files=max(100, 2000 // divisor), commits=max(100, 4000 // divisor))
        receipt['attempts'].append(attempt)
        save()
        try:
            attempt['seed'] = run(args.candidate, ['seed', source, attempt['files'], attempt['commits']], folder, 'seed')
            for name, binary in [('baseline', args.baseline), ('candidate', args.candidate)]:
                attempt[name] = run(binary, ['run', source, folder / name], folder, name)
            baseline = next(event for event in attempt['baseline']['events'] if 'object_files' in event)
            candidate = next(event for event in attempt['candidate']['events'] if 'object_files' in event)
            assert candidate['object_files'] < baseline['object_files'] / 10, (baseline, candidate)
            attempt['result'] = 'PASS'
            receipt['result'] = 'PASS'
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

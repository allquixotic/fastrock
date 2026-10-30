#!/usr/bin/env python3
"""Run the feature-complete native fixture matrix serially on one executable."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from benchmark_guard import BoundedJob

# Each child fixture separately watches Fastrock at 2 GB. The outer job also
# contains helpers/servers and guarantees cleanup if a fixture itself stalls.
CASES = [
    ('rally', ['--ci']), ('goose', []), ('vwork', []),
    ('delegation', None), ('schedules', None), ('webhook', None),
    ('provider-setup', []), ('profiles', None), ('effort', None),
    ('capabilities', None), ('recipes', None), ('projects', None), ('commands', None),
    ('palette', None), ('terminal', None), ('checkpoints', None),
    ('sidebar', None), ('organization', None), ('sidebar-drag', None),
    ('sidebar-policy', None), ('titles', None), ('queue', None),
    ('mentions', None), ('review', None), ('shell', None), ('side', None),
    ('window-recovery', None), ('delete', None), ('history', []),
    ('selection', []),
    ('summary', ['--turns', '3', '--navigation', '--windows', '--tray']),
    ('summary-priority', None),
    ('queue-recovery', ['--recovery']), ('process-recovery', ['--process-recovery']),
]

def main():
    assert os.name == 'nt', 'Native acceptance runs on Windows only'
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('binary', type=Path)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--only', nargs='+', choices=[name for name, _ in CASES])
    p.add_argument('--seed-executable', type=Path, help='Matching embedded Goose test executable for history seeding')
    p.add_argument('--keep-going', action='store_true', help='Collect ordinary test failures; memory/time guards still stop the matrix')
    args = p.parse_args()
    binary, out = args.binary.resolve(), args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    dev = Path(__file__).resolve().parent
    report = dict(binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), result='RUNNING', cases=[])
    def save():
        (out/'acceptance.json').write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
    save()
    try:
        for name, flags in CASES:
            if args.only and name not in args.only:
                continue
            script = 'summary' if name in ('queue-recovery', 'process-recovery') else name
            command = [sys.executable, str(dev/f'windows-{script}-smoke.py'), str(binary)]
            command += ['--output', str(out/name)]
            if script == 'history' and args.seed_executable:
                command += ['--seed-executable', str(args.seed_executable.resolve())]
            command += flags or []
            print(f'Native acceptance: {name}', flush=True)
            failure = None
            started = time.monotonic()
            with (out/f'{name}.log').open('w', encoding='utf-8') as log:
                process = subprocess.Popen(command, stdout=log, stderr=log, env=dict(os.environ, PYTHONUTF8='1'))
                job = BoundedJob(process, 4_000_000_000)
                try:
                    while process.poll() is None:
                        job.sample()
                        if time.monotonic()-started > 420:
                            raise TimeoutError(f'{name} exceeded seven minutes')
                        time.sleep(.2)
                    if process.returncode:
                        failure = f'{name} failed ({process.returncode}); see {out/name}.log'
                        if not args.keep_going:
                            raise RuntimeError(failure)
                finally:
                    job.close()
                    process.wait(timeout=10)
            report['cases'].append(dict(name=name, result='FAIL' if failure else 'PASS', error=failure, seconds=round(time.monotonic()-started, 2), peak_tree_bytes=job.peak))
            save()
            print(f'{"FAIL" if failure else "PASS"}: {name}', flush=True)
        assert report['cases'], 'No acceptance cases selected'
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == report['binary_sha256'], 'Executable changed during acceptance'
        failures = [case['name'] for case in report['cases'] if case['result'] == 'FAIL']
        if failures:
            raise RuntimeError('Failed acceptance cases: ' + ', '.join(failures))
        report['result'] = 'PASS'
    except BaseException as error:
        report['result'] = 'FAIL'
        report['error'] = str(error)
        raise
    finally:
        save()

if __name__ == '__main__':
    main()

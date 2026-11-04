#!/usr/bin/env python3
"""Compare native startup on a valid 2.5 GiB fixture store, never user data.

Measures Windows process IO counters (logical bytes, not physical disk traffic).
The filler table isolates database-size cost; real Git repos/workspaces exercise
metadata enumeration. This is not a reproduction of a user's repository mix.
"""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import time
from benchmark_guard import BoundedJob, GuardStopped, HARD_CAP, headroom, guest_headroom
from embedded_provider_fixture import ProcessGuard, ProviderFixture


def bounded(command, env, folder, name, host_guard, record):
    budget = min(HARD_CAP, headroom(host_guard), guest_headroom())
    record.update(cap_bytes=budget)
    class IO(ctypes.Structure):
        _fields_ = [(field, ctypes.c_ulonglong) for field in (
            'read_operations', 'write_operations', 'other_operations', 'read_bytes', 'write_bytes', 'other_bytes')]
    io = IO()
    read_io = ctypes.WinDLL('kernel32', use_last_error=True).GetProcessIoCounters
    read_io.argtypes = [wintypes.HANDLE, ctypes.POINTER(IO)]
    read_io.restype = wintypes.BOOL
    start = time.monotonic()
    with (folder / f'{name}.log').open('w', encoding='utf-8') as log:
        process = subprocess.Popen(command, env=env, cwd=folder, stdout=log, stderr=log)
        job = BoundedJob(process, budget)
        guard = ProcessGuard(process, timeout=300, cap=HARD_CAP)
        def tick():
            headroom(host_guard)
            guest_headroom()
            job.sample()
        try:
            guard.wait(tick)
        finally:
            if read_io(int(process._handle), ctypes.byref(io)):
                record['io'] = {field: getattr(io, field) for field, _ in IO._fields_}
            record.update(elapsed_ms=round((time.monotonic() - start) * 1000, 3),
                          peak_app_bytes=guard.peak, peak_tree_commit_bytes=job.peak,
                          exit_code=process.poll())
            job.close()
            if process.poll() is None:
                process.kill()
            process.wait(timeout=10)


def main():
    assert os.name == 'nt', 'Run on games, never the development Mac'
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('candidate', type=Path)
    parser.add_argument('--baseline', type=Path, help='Optional baseline; omit to measure a candidate after a guarded baseline abort')
    parser.add_argument('--seed-executable', type=Path, required=True)
    parser.add_argument('--host-guard', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    # Fjall stores incompressible fixture values near their logical size.
    parser.add_argument('--mib', type=int, default=2560)
    parser.add_argument('--repositories', type=int, default=24)
    args = parser.parse_args()
    assert 1 <= args.mib <= 4096 and 1 <= args.repositories <= 64
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    receipt = dict(host=platform.node(), hard_cap_bytes=HARD_CAP, attempts=[])
    def save():
        (out / 'summary.json').write_text(json.dumps(receipt, indent=2), encoding='utf-8')
    divisor = 1
    while divisor <= 1000:
        folder = out / f'scale-{divisor}'
        folder.mkdir()
        attempt = dict(mib=max(1, args.mib // divisor), repositories=max(1, args.repositories // divisor), runs={})
        receipt['attempts'].append(attempt)
        try:
            seed = folder / 'seed'
            bounded([str(args.seed_executable.resolve()), '--seed-both' if args.baseline else '--seed', str(seed), str(attempt['mib']), str(attempt['repositories'])],
                    os.environ.copy(), folder, 'seed', args.host_guard, attempt.setdefault('seed', {}))
            database_bytes = sum(p.stat().st_size for p in (seed/'store/workspaces.fjall').rglob('*') if p.is_file())
            attempt['database_bytes'] = database_bytes
            binaries = ([('baseline', args.baseline)] if args.baseline else []) + [('candidate', args.candidate)]
            for name, binary in binaries:
                run = folder / name
                run.mkdir()
                record = dict(binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest())
                attempt['runs'][name] = record
                provider = ProviderFixture()
                env = provider.environment(run)
                shutil.copytree(seed / ('legacy-store' if name=='baseline' else 'store'), run / 'local/vwork')
                env['CODEX_GUI_PERF'] = '1'
                steps = [{'mark': 'startup-begin'}, {'wait_ready': 90000}, {'mark': 'startup-ready'}]
                for index in range(3):
                    steps += [{'mark': f'thread-{index}-begin'}, {'new_thread': str(run / 'project')},
                              {'wait_idle': 120000}, {'mark': f'thread-{index}-ready'},
                              {'send': f'store-startup-{index}'}, {'wait_idle': 60000}]
                if name == 'candidate':
                    steps += [{'runtime_facts': str(run / 'runtime.json')}]
                steps += [{'sidebar': ['dump', str(run / 'sidebar.json')]}, {'wait': 500}, {'quit': True}]
                script = run / 'steps.json'
                script.write_text(json.dumps(steps), encoding='utf-8')
                env['CODEX_GUI_AUTOMATION'] = str(script)
                try:
                    bounded([str(binary.resolve()), '--remote', 'goose'], env, run, 'app', args.host_guard, record)
                    log = (run / 'app.log').read_text(encoding='utf-8')
                    marks = {key: float(value) for key, value in re.findall(r'codex-gui perf: (\S+) ([\d.]+) ms', log)}
                    record['thread_ready_ms'] = [round(marks[f'thread-{i}-ready'] - marks[f'thread-{i}-begin'], 3) for i in range(3)]
                    record['startup_ready_ms'] = round(marks['startup-ready'] - marks['startup-begin'], 3)
                    assert len([r for r in provider.requests if not r['summary']]) == 3, 'Missing foreground responses'
                    if name == 'candidate':
                        record['runtime'] = json.loads((run / 'runtime.json').read_text(encoding='utf-8'))
                        facts = dict(record['runtime']['timings'])
                        assert facts['Workspace store initialization'].startswith('opens 1; clean 1;'), facts
                        assert record['io']['write_bytes'] < database_bytes / 2, 'Clean startup rewrote a large fraction of the store'
                        assert not (run / 'local/vwork/workspaces.redb.recovery.bak').exists(), 'Clean startup took a recovery snapshot'
                    record['result'] = 'PASS'
                except GuardStopped:
                    raise
                except Exception as error:
                    record.update(result='FAILED', reason=str(error))
                    if name == 'candidate':
                        raise
                finally:
                    provider.close()
                    provider.write_evidence(run)
                    save()
            attempt['result'] = 'PASS'
            save()
            print(json.dumps(attempt, indent=2), flush=True)
            return
        except GuardStopped as error:
            attempt.update(result='ABORTED', reason=str(error), next_divisor=divisor * 10)
            divisor *= 10
            save()
        except BaseException as error:
            attempt.update(result='FAILED', reason=str(error))
            save()
            raise
    raise SystemExit('No safe workload completed')


if __name__ == '__main__':
    main()

#!/usr/bin/env python3
"""Read-only host-memory monitoring for remote Windows benchmarks.

Runs on the development Mac; it never loads Fastrock or executes tests locally.
The SSH receiver writes a heartbeat in the guest. Missing/stale observations
stop the benchmark there. No host/VM settings or processes are changed.
"""
import argparse
import json
from pathlib import Path
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--python', required=True, help='Python executable for the games SSH account')
parser.add_argument('--receiver', required=True)
parser.add_argument('--guard', required=True)
parser.add_argument('--stop-file', type=Path, required=True)
args = parser.parse_args()


def windows_argument(value):
    assert not any(char in value for char in '\r\n"%&|<>^'), 'Unsafe Windows command argument'
    return '"' + value + '"'


command = ' '.join(map(windows_argument, [args.python, args.receiver, 'receive', args.guard]))
receiver = subprocess.Popen(['ssh', '-o', 'BatchMode=yes', 'games', command], stdin=subprocess.PIPE, text=True)
try:
    last_report = 0
    while not args.stop_file.exists():
        result = subprocess.run(['ssh', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=3',
                                 'sean@aorus-ubuntu', "awk '/MemAvailable:/ {print $2}' /proc/meminfo"],
                                text=True, capture_output=True, timeout=4, check=True)
        available = int(result.stdout.strip()) * 1024
        receiver.stdin.write(json.dumps(dict(healthy=True, available=available)) + '\n')
        receiver.stdin.flush()
        if time.monotonic() - last_report >= 15:
            print(f'Hypervisor available: {available:,} bytes; heartbeat active', flush=True)
            last_report = time.monotonic()
        time.sleep(1)
finally:
    receiver.stdin.close()  # EOF marks the guest heartbeat unhealthy.
    receiver.wait(timeout=10)

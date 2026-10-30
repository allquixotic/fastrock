"""Windows benchmark containment plus a fail-closed hypervisor heartbeat.

The controller on the Mac reads host memory over SSH. This module never changes
the VM/host, and the job object contains only the fixture-owned process tree.
"""
import ctypes
from ctypes import wintypes
import json
from pathlib import Path
import sys
import time

HARD_CAP = 32_000_000_000
HOST_RESERVE = 4_000_000_000
GUEST_RESERVE = 4_000_000_000


class GuardStopped(RuntimeError):
    pass


def headroom(path):
    try:
        data = json.loads(Path(path).read_text(encoding='utf-8'))
        age = time.time() - data['received_at']
        if not data['healthy'] or not 0 <= age <= 6:
            raise GuardStopped('Hypervisor memory heartbeat unavailable or stale')
        if data['available'] <= HOST_RESERVE:
            raise GuardStopped('Hypervisor memory reserve reached')
        return data['available'] - HOST_RESERVE
    except (OSError, ValueError, KeyError) as error:
        raise GuardStopped('Cannot verify hypervisor memory headroom') from error


def guest_headroom():
    class Status(ctypes.Structure):
        _fields_ = [('length', wintypes.DWORD), ('load', wintypes.DWORD)] + [
            (name, ctypes.c_ulonglong) for name in ('total_physical', 'available_physical',
            'total_pagefile', 'available_pagefile', 'total_virtual', 'available_virtual', 'available_extended')]
    value = Status()
    value.length = ctypes.sizeof(value)
    if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(value)):
        raise GuardStopped('Cannot measure guest memory')
    if value.available_physical <= GUEST_RESERVE:
        raise GuardStopped('Guest memory reserve reached')
    return value.available_physical - GUEST_RESERVE


class BasicLimits(ctypes.Structure):
    _fields_ = [('process_time', ctypes.c_longlong), ('job_time', ctypes.c_longlong),
                ('flags', wintypes.DWORD), ('minimum_working_set', ctypes.c_size_t),
                ('maximum_working_set', ctypes.c_size_t), ('processes', wintypes.DWORD),
                ('affinity', ctypes.c_size_t), ('priority', wintypes.DWORD), ('scheduling', wintypes.DWORD)]


class Limits(ctypes.Structure):
    _fields_ = [('basic', BasicLimits), ('io', ctypes.c_ulonglong * 6),
                ('process_memory', ctypes.c_size_t), ('job_memory', ctypes.c_size_t),
                ('peak_process_memory', ctypes.c_size_t), ('peak_job_memory', ctypes.c_size_t)]


class BoundedJob:
    def __init__(self, process, cap):
        assert 0 < cap <= HARD_CAP
        self.process, self.cap, self.peak = process, cap, 0
        self.api = ctypes.WinDLL('kernel32', use_last_error=True)
        self.api.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
        self.api.CreateJobObjectW.restype = wintypes.HANDLE
        self.api.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
        self.api.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
        self.api.QueryInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p]
        self.api.CloseHandle.argtypes = [wintypes.HANDLE]
        self.handle = self.api.CreateJobObjectW(None, None)
        try:
            if not self.handle:
                raise GuardStopped('Cannot create fixture memory containment')
            limits = Limits()
            limits.basic.flags = 0x100 | 0x200 | 0x2000  # process/job memory, kill owned tree on close
            limits.process_memory = min(cap, HARD_CAP)
            limits.job_memory = cap
            if not self.api.SetInformationJobObject(self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
                raise GuardStopped('Cannot apply fixture memory containment')
            if not self.api.AssignProcessToJobObject(self.handle, int(process._handle)):
                raise GuardStopped('Cannot attach the owned process to its memory limit')
        except BaseException:
            self.close()
            if process.poll() is None:
                process.kill()
            process.wait(timeout=10)
            raise

    def sample(self):
        limits = Limits()
        if not self.api.QueryInformationJobObject(self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits), None):
            raise GuardStopped('Cannot read fixture process-tree memory')
        self.peak = max(self.peak, limits.peak_job_memory)
        # Leave margin to stop cleanly before the OS refuses an allocation.
        if self.peak >= self.cap * .90:
            raise GuardStopped(f'Fixture process tree reached its headroom cap ({self.peak}/{self.cap})')

    def close(self):
        if self.handle:
            self.api.CloseHandle(self.handle)
            self.handle = None


def receive(path):
    """SSH stdin receiver: timestamp in the guest to avoid cross-host clock skew."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    def save(value):
        value['received_at'] = time.time()
        temporary = path.with_suffix('.tmp')
        temporary.write_text(json.dumps(value), encoding='utf-8')
        temporary.replace(path)
    try:
        for line in sys.stdin:
            value = json.loads(line)
            assert isinstance(value['available'], int) and isinstance(value['healthy'], bool)
            save(value)
    finally:
        save(dict(healthy=False, available=0))


if __name__ == '__main__':
    assert sys.argv[1] == 'receive'
    receive(sys.argv[2])

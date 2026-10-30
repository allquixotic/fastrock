"""Verify native fixture failure paths terminate only the owned process."""
import os
import subprocess
import sys
import tempfile
import time
import json
from pathlib import Path
import unittest
from benchmark_guard import BoundedJob, GuardStopped, headroom
from embedded_provider_fixture import ProcessGuard


@unittest.skipUnless(os.name == 'nt', 'Windows process accounting')
class Watchdog(unittest.TestCase):
    def check_stop(self, **limits):
        unrelated = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])
        owned = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])
        try:
            with self.assertRaises(AssertionError) as failure:
                ProcessGuard(owned, **limits).wait()
            self.assertIsNotNone(owned.poll(), 'Owned process survived failed acceptance')
            self.assertIsNone(unrelated.poll(), 'Watchdog terminated an unrelated process')
            return str(failure.exception)
        finally:
            for process in (owned, unrelated):
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=10)

    def test_v45_memory_cap_stops_only_owned_process(self):
        self.assertIn('memory cap', self.check_stop(cap=1))

    def test_v45_deadline_stops_only_owned_process(self):
        self.assertIn('timed out', self.check_stop(timeout=0))

    def test_v45_stale_and_low_host_headroom_fail_closed(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder, 'host.json')
            for value in (dict(healthy=True, available=8_000_000_000, received_at=time.time() - 10),
                          dict(healthy=True, available=3_000_000_000, received_at=time.time()),
                          dict(healthy=False, available=8_000_000_000, received_at=time.time())):
                path.write_text(json.dumps(value), encoding='utf-8')
                with self.assertRaises(GuardStopped):
                    headroom(path)
            path.unlink()
            with self.assertRaises(GuardStopped):
                headroom(path)

    def test_v45_kernel_refuses_oversized_fixture_allocation(self):
        with tempfile.TemporaryDirectory() as folder:
            gate = Path(folder, 'attached')
            script = ("import pathlib,time,sys\n"
                      "while not pathlib.Path(sys.argv[1]).exists(): time.sleep(.01)\n"
                      "try: value=bytearray(256_000_000)\n"
                      "except MemoryError: sys.exit(7)\n"
                      "sys.exit(9)\n")
            process = subprocess.Popen([sys.executable, '-c', script, str(gate)])
            job = BoundedJob(process, 128_000_000)
            try:
                gate.touch()
                self.assertEqual(process.wait(timeout=10), 7, 'Kernel memory cap did not reject allocation')
            finally:
                job.close()
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=10)


if __name__ == '__main__':
    unittest.main()

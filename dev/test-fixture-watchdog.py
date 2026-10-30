"""Verify native fixture failure paths terminate only the owned process."""
import os
import subprocess
import sys
import unittest
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


if __name__ == '__main__':
    unittest.main()

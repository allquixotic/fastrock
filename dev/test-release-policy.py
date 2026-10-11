#!/usr/bin/env python3
"""V25: exercise policy rejection and actual compiler-generated symbol pairs."""
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
from release_config import build_plan, release_tag
from debug_symbols import verify_windows, verify_macos


class ReleasePolicy(unittest.TestCase):
    def test_v25_only_authorized_stable_version_optimizes(self):
        self.assertEqual(build_plan('0.2.0')['profile'], 'official')
        self.assertEqual(build_plan('0.2.0')['lto'], 'off')
        self.assertEqual(build_plan('0.2.1-alpha.1')['profile'], 'dev')
        for version in ('0.2.1', '0.3.0', '0.2.0-arbitrary'):
            with self.assertRaises(AssertionError):
                build_plan(version)

    def test_v25_policy_rejects_lto_and_fast_profile_optimization(self):
        for name, value in [('CARGO_PROFILE_OFFICIAL_LTO', 'false'),
                            ('CARGO_PROFILE_OFFICIAL_DEBUG', '0'),
                            ('CARGO_PROFILE_DEV_OPT_LEVEL', '1'),
                            ('RUSTFLAGS', '-C lto=thin'), ('RUSTFLAGS', '-C lto=false')]:
            result = subprocess.run([sys.executable, str(ROOT / 'dev/check-build-policy.py')],
                                    env={**os.environ, name: value}, capture_output=True)
            self.assertNotEqual(result.returncode, 0, name)

    def test_v25_build_metadata_preserves_stable_application_version(self):
        from unittest.mock import patch
        for tag in ('v0.2.0', 'v0.2.0+build.1'):
            with patch.dict(os.environ, GITHUB_REF_NAME=tag):
                self.assertEqual(release_tag(), tag)
        for tag in ('main', 'v0.2.1', 'v0.2.0-alpha.1', 'v0.2.0+build.0'):
            with patch.dict(os.environ, GITHUB_REF_NAME=tag):
                with self.assertRaises(AssertionError):
                    release_tag()

    @unittest.skipUnless(sys.platform == 'darwin', 'Uses local Apple and LLVM compilers, no GUI')
    def test_v25_real_pe_pdb_and_macho_dsym_match_and_reject_swapped_symbols(self):
        clang = '/opt/homebrew/opt/llvm/bin/clang'
        linker = '/opt/homebrew/opt/lld/bin/lld-link'
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            for n in (1, 2):
                work = root / str(n)
                work.mkdir()
                source = work / 'fixture.c'
                source.write_text(f'volatile int value={n}; int main(void) {{return value;}}\n')
                subprocess.run([clang, '--target=x86_64-pc-windows-msvc', '-g', '-gcodeview', '-O1',
                                '-c', str(source), '-o', str(work / 'fixture.obj')], check=True)
                subprocess.run([linker, '/debug', '/entry:main', '/subsystem:console', '/nodefaultlib',
                                f'/out:{work / "fastrock.exe"}', f'/pdb:{work / "fastrock.pdb"}',
                                str(work / 'fixture.obj')], check=True)
                subprocess.run(['xcrun', 'clang', '-g', '-O1', '-c', str(source), '-o', str(work / 'mac.o')], check=True)
                subprocess.run(['xcrun', 'clang', str(work / 'mac.o'), '-o', str(work / 'fastrock')], check=True)
                subprocess.run(['xcrun', 'dsymutil', str(work / 'fastrock')], check=True)
            first, second = root / '1', root / '2'
            # Cargo retains the hashed link-time filename inside the dSYM.
            dwarf = first / 'fastrock.dSYM/Contents/Resources/DWARF/fastrock'
            dwarf.rename(dwarf.with_name('fastrock-cargohash'))
            self.assertEqual(verify_windows(first / 'fastrock.exe', first / 'fastrock.pdb')['format'], 'PDB')
            self.assertEqual(verify_macos(first / 'fastrock', first / 'fastrock.dSYM')['format'], 'dSYM')
            with self.assertRaisesRegex(AssertionError, 'does not match'):
                verify_windows(first / 'fastrock.exe', second / 'fastrock.pdb')
            with self.assertRaisesRegex(AssertionError, 'does not match'):
                verify_macos(first / 'fastrock', second / 'fastrock.dSYM')
            empty = root / 'empty'
            empty.mkdir()
            subprocess.run(['xcrun', 'clang', str(first / 'fixture.c'), '-o', str(empty / 'fastrock')], check=True)
            subprocess.run(['xcrun', 'dsymutil', str(empty / 'fastrock')], check=True, capture_output=True)
            with self.assertRaisesRegex(AssertionError, 'lacks usable'):
                verify_macos(empty / 'fastrock', empty / 'fastrock.dSYM')


if __name__ == '__main__':
    unittest.main()

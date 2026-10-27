"""Explicit release authorization and effective compiler settings."""
import pathlib
import os
import re
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]


def build_plan(version=None):
    if version is None:
        version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['package']['version']
    prerelease = bool(re.fullmatch(r'\d+\.\d+\.\d+-(alpha|beta|rc)\.\d+', version))
    assert prerelease or version in ('0.2.0', '0.3.0', '0.3.1'), 'Stable release needs explicit release authorization'
    official = version == '0.2.0'
    return dict(version=version, prerelease=prerelease,
                profile='official' if official else 'dev',
                opt_level=1 if official else 0, debug_info=2 if official else 0,
                lto='off' if official else False, codegen_units=256, incremental=True,
                split_debuginfo='packed' if official else 'off', strip='none')


def suffix(plan):
    return '-fast' if plan['profile'] == 'dev' else ''


def release_tag():
    plan = build_plan()
    tag = os.environ['GITHUB_REF_NAME']
    base = 'v' + plan['version']
    pattern = re.escape(base) + (r'(?:\+build\.[1-9]\d*)?' if not plan['prerelease'] else '')
    assert re.fullmatch(pattern, tag), 'Release tag does not match the application version'
    return tag

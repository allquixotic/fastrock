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
    assert prerelease or version == '0.2.0', 'Stable optimization needs explicit release authorization'
    return dict(version=version, prerelease=prerelease,
                profile='dev' if prerelease else 'official',
                opt_level=0 if prerelease else 1, debug_info=0 if prerelease else 2,
                lto=False if prerelease else 'off', codegen_units=256, incremental=True,
                split_debuginfo='off' if prerelease else 'packed', strip='none')


def suffix(plan):
    return '-fast' if plan['prerelease'] else ''


def release_tag():
    plan = build_plan()
    tag = os.environ['GITHUB_REF_NAME']
    base = 'v' + plan['version']
    pattern = re.escape(base) + (r'(?:\+build\.[1-9]\d*)?' if not plan['prerelease'] else '')
    assert re.fullmatch(pattern, tag), 'Release tag does not match the application version'
    return tag

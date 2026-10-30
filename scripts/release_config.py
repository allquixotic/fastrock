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
    assert prerelease or version in ('0.2.0', '0.3.0', '0.3.1', '0.3.2', '0.3.3', '0.3.4', '0.4.0'), 'Stable release needs explicit release authorization'
    official = version == '0.2.0'
    embedded = version.split('-')[0] == '0.4.0'
    return dict(version=version, prerelease=prerelease,
                profile='official' if official else 'dev',
                opt_level=1 if official else 2 if embedded else 0, debug_info=2 if official else 0,
                lto='off', codegen_units=256, incremental=True,
                split_debuginfo='packed' if official else 'off', strip='none',
                rendering_dependencies=tomllib.loads((ROOT / 'Cargo.toml').read_text())['profile']['dev']['package'])


def suffix(plan):
    return '-fast' if plan['profile'] == 'dev' and plan['opt_level'] == 0 and plan['version'] not in ('0.3.2', '0.3.3', '0.3.4') else ''


def assert_publishable(version, spec=None):
    if version.split('-')[0] != '0.4.0':
        return
    assert version == '0.4.0' or re.fullmatch(r'0\.4\.0-rc\.[1-9]\d*', version), 'No more alpha/beta publication; finish the spec before an RC'
    if spec is None:
        spec = (ROOT / 'SPEC.md').read_text()
    tasks = re.findall(r'^(T\d+)\|([^|]+)\|', spec, re.MULTILINE)
    assert len(tasks) == len(dict(tasks)), 'Duplicate spec task identifiers'
    required = {f'T{n}' for n in range(37, 46)} - {'T44'}
    # Any implementation tasks added after the original plan also block an RC.
    required.update(task for task, _ in tasks if int(task[1:]) > 45)
    if version == '0.4.0':
        required.update({'T36', 'T44'})
    incomplete = sorted(task for task in required if dict(tasks).get(task) != 'x')
    assert not incomplete, 'Feature-complete release blocked by spec tasks: ' + ', '.join(incomplete)


def release_tag():
    plan = build_plan()
    tag = os.environ['GITHUB_REF_NAME']
    base = 'v' + plan['version']
    pattern = re.escape(base) + (r'(?:\+build\.[1-9]\d*)?' if not plan['prerelease'] else '')
    assert re.fullmatch(pattern, tag), 'Release tag does not match the application version'
    assert_publishable(plan['version'])
    return tag

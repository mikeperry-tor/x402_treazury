#!/usr/bin/env python3
"""Replay RustSec assessment offline, including explicitly identified local sources."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib

REGISTRY = 'registry+https://github.com/rust-lang/crates.io-index'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def run(command):
    return subprocess.run(command, capture_output=True, text=True, check=False)


def git(path, *args):
    result = run(['git', '-C', str(path), *args])
    if result.returncode:
        raise ValueError(f'git {args[0]} failed: {result.stderr.strip()}')
    return result.stdout.strip()


def source_hashes(path):
    """Hash actual source bytes; do not traverse links or build/VCS artifacts."""
    files = {}
    # Git's tracked inventory avoids ignored wallets, credentials and build artifacts.
    listing = run(['git', '-C', str(path), 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--', '.'])
    if listing.returncode:
        raise ValueError(f'source must have a Git-tracked inventory: {path}')
    entries = [path / name for name in listing.stdout.split('\0') if name]
    if not entries:
        raise ValueError(f'empty source inventory: {path}')
    for entry in sorted(entries):
        relative = entry.relative_to(path)
        if relative == Path('.cargo-ok') or any(part in ('.git', 'target', '__pycache__') for part in relative.parts):
            continue
        if entry.is_symlink():
            files[str(relative)] = {'symlink': str(entry.readlink())}
        elif entry.is_file():
            files[str(relative)] = digest(entry.read_bytes())
    return {'sha256': digest(json.dumps(files, sort_keys=True).encode()), 'files': files}


def normalized_inventory(packages):
    """Synthetic RustSec name/version queries, never a replacement Cargo resolution."""
    rows = ['version = 4', '']
    for name, version in sorted({(p['name'], p['version']) for p in packages}):
        rows.extend(['[[package]]', f'name = {json.dumps(name)}',
                     f'version = {json.dumps(version)}', f'source = {json.dumps(REGISTRY)}', ''])
    return '\n'.join(rows)


def audit(executable, database, lockfile, output):
    command = [executable, 'audit', '--db', str(database), '--no-fetch', '--no-yanked',
               '--stale', '--deny', 'warnings', '--json', '--file', str(lockfile)]
    result = run(command)
    output.with_suffix('.stdout.json').write_text(result.stdout)
    output.with_suffix('.stderr.txt').write_text(result.stderr)
    if result.returncode not in (0, 1):
        raise ValueError(f'audit tool failed for {lockfile}: exit {result.returncode}')
    try:
        report = json.loads(result.stdout)
        vulnerabilities = report['vulnerabilities']
        count = vulnerabilities['count']
        if (type(count) is not int or count < 0
                or vulnerabilities['found'] != (count > 0)
                or len(vulnerabilities['list']) != count):
            raise ValueError('inconsistent vulnerability report')
        settings = report['settings']
        if any(settings.get(key) for key in ('ignore', 'target_arch', 'target_os', 'severity')):
            raise ValueError('audit configuration filtered advisories')
        categories = settings.get('informational_warnings')
        if not isinstance(categories, list) or not {'notice', 'unmaintained', 'unsound'} <= set(categories):
            raise ValueError('audit configuration omitted informational advisories')
        warnings = report['warnings']
        if not isinstance(warnings, dict) or any(not isinstance(rows, list) for rows in warnings.values()):
            raise ValueError('invalid advisory warnings')
        if result.returncode == 0 and (count or any(warnings.values())):
            raise ValueError('successful exit despite advisory findings')
    except (KeyError, TypeError, json.JSONDecodeError) as error:
        raise ValueError(f'invalid audit JSON for {lockfile}: {error}') from error
    return {'command': command, 'exit_code': result.returncode,
            'status': 'findings' if result.returncode else 'clean', 'vulnerabilities': count,
            'warnings': report.get('warnings', {})}


def assess(root, database, revision, sources, output, executable):
    if len(revision) != 40 or any(c not in '0123456789abcdef' for c in revision):
        raise ValueError('--db-revision must be an exact lowercase 40-character Git SHA')
    if git(database, 'rev-parse', 'HEAD') != revision:
        raise ValueError('advisory checkout HEAD does not match --db-revision')
    if git(database, 'status', '--porcelain', '--untracked-files=all'):
        raise ValueError('advisory checkout has local changes')
    binary = shutil.which(executable)
    if binary is None:
        raise ValueError(f'operator-installed audit executable not found: {executable}')
    version = run([binary, '--version'])
    if version.returncode:
        raise ValueError('cannot identify cargo-audit version')
    summary = {'database_revision': revision, 'audit_version': version.stdout.strip(),
               'audit_executable_sha256': digest(Path(binary).read_bytes()), 'lockfiles': [],
               'nonregistry_sources': [], 'reports': []}
    output.mkdir(parents=True, exist_ok=True)
    nonregistry = []
    for name in ('Cargo.lock', 'tools/protoc/Cargo.lock'):
        original = root / name
        data = original.read_bytes()
        packages = tomllib.loads(data.decode())['package']
        snapshot = output / name.replace('/', '_')
        snapshot.write_bytes(data)
        summary['lockfiles'].append({'path': name, 'sha256': digest(data),
                                    'packages': len(packages)})
        nonregistry.extend((name, p) for p in packages
                           if p.get('source') != REGISTRY)
    for lockfile, package in nonregistry:
        identity = package['name']
        path = sources.get(identity)
        if path is None:
            raise ValueError(f'missing --source {identity}=PATH ({lockfile})')
        path = path.resolve()
        manifest = tomllib.loads((path / 'Cargo.toml').read_text())['package']
        if manifest['name'] != identity:
            raise ValueError(f'source package name mismatch for {identity}')
        declared_version = manifest['version']
        if isinstance(declared_version, dict):
            if declared_version != {'workspace': True}:
                raise ValueError(f'unsupported inherited version for {identity}')
            for ancestor in (path, *path.parents):
                candidate = ancestor / 'Cargo.toml'
                if candidate.is_file():
                    workspace = tomllib.loads(candidate.read_text()).get('workspace', {})
                    if 'version' in workspace.get('package', {}):
                        declared_version = workspace['package']['version']
                        break
        if declared_version != package['version']:
            raise ValueError(f'source package version mismatch for {identity}')
        source = package.get('source')
        if source and source.startswith('git+'):
            expected = source.rsplit('#', 1)[-1]
            if git(path, 'rev-parse', 'HEAD') != expected:
                raise ValueError(f'Git revision mismatch for {identity}')
            checkout = Path(git(path, 'rev-parse', '--show-toplevel'))
            if git(checkout, 'status', '--porcelain', '--untracked-files=no'):
                raise ValueError(f'Git source has local changes: {identity}')
            untracked = git(checkout, 'ls-files', '--others', '--exclude-standard').splitlines()
            # Cargo writes this cache marker after checkout; it is not source.
            if any(name != '.cargo-ok' for name in untracked):
                raise ValueError(f'Git source has untracked files: {identity}')
        summary['nonregistry_sources'].append({
            'lockfile': lockfile, 'name': identity, 'version': package['version'],
            'original_source': source, 'path': str(path), 'source': source_hashes(path)})
    inventory = output / 'nonregistry-inventory.lock'
    inventory.write_text(normalized_inventory([p for _, p in nonregistry]))
    summary['normalized_inventory_sha256'] = digest(inventory.read_bytes())
    inputs = [output / name.replace('/', '_') for name in ('Cargo.lock', 'tools/protoc/Cargo.lock')]
    inputs.append(inventory)
    for index, lockfile in enumerate(inputs):
        summary['reports'].append(audit(binary, database, lockfile, output / f'audit-{index}'))
    if (git(database, 'rev-parse', 'HEAD') != revision
            or git(database, 'status', '--porcelain', '--untracked-files=all')):
        raise ValueError('advisory checkout changed during assessment')
    summary['status'] = ('findings' if any(r['exit_code'] for r in summary['reports']) else 'clean')
    (output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    return 1 if summary['status'] == 'findings' else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--db', type=Path, required=True)
    parser.add_argument('--db-revision', required=True)
    parser.add_argument('--source', action='append', default=[], metavar='NAME=PATH',
                        help='actual package source for each path/Git/workspace lock entry')
    parser.add_argument('--output', type=Path, default=Path('target/dependency-audit'))
    parser.add_argument('--cargo-audit', default='cargo-audit')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    try:
        sources = {}
        for item in args.source:
            name, separator, value = item.partition('=')
            if not separator or not name or not value or name in sources:
                raise ValueError(f'invalid or duplicate source mapping: {item}')
            sources[name] = Path(value)
        status = assess(root, args.db.resolve(), args.db_revision, sources,
                        args.output.resolve(), args.cargo_audit)
    except (OSError, ValueError, KeyError, tomllib.TOMLDecodeError) as error:
        args.output.mkdir(parents=True, exist_ok=True)
        (args.output / 'summary.json').write_text(json.dumps(
            {'status': 'tool-failure', 'error': str(error)}, indent=2) + '\n')
        print(f'dependency assessment failed: {error}', file=sys.stderr)
        return 2
    print(f'dependency assessment: {"findings" if status else "clean"}; {args.output}/summary.json',
          file=sys.stderr)
    return status


if __name__ == '__main__':
    sys.exit(main())

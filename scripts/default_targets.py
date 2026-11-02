"""Run the default Cargo targets not already selected by check-compat."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tomllib

LIB_KINDS = {'lib', 'rlib', 'dylib', 'staticlib', 'cdylib', 'proc-macro'}


def remaining_targets(metadata, manifest, manifest_path, compat):
    manifest_path = Path(manifest_path).resolve()
    roots = [package for package in metadata['packages']
             if Path(package['manifest_path']).resolve() == manifest_path]
    if len(roots) != 1:
        raise ValueError('metadata must identify exactly one root package')
    root = roots[0]
    if metadata['workspace_default_members'] != [root['id']]:
        raise ValueError('default workspace selection must contain only the root package')
    nodes = [node for node in metadata['resolve']['nodes'] if node['id'] == root['id']]
    if len(nodes) != 1:
        raise ValueError('metadata must resolve the root feature set')
    features = set(nodes[0]['features'])
    targets = {}
    for target in root['targets']:
        kinds = set(target['kind'])
        if kinds == {'custom-build'}:
            continue
        if kinds and kinds <= LIB_KINDS:
            kind = 'lib'
        elif len(kinds) == 1 and kinds <= {'bin', 'example', 'test', 'bench'}:
            kind = next(iter(kinds))
        else:
            raise ValueError(f'unsupported root target kind: {target["kind"]}')
        key = (kind, target['name'])
        if key in targets:
            raise ValueError(f'duplicate Cargo target: {key}')
        targets[key] = target

    # Cargo metadata omits bench. Read only this missing flag; Cargo still owns
    # inventory, feature resolution, harness settings and target execution.
    bench_flags = {}
    for kind in ('test', 'bench'):
        for row in manifest.get(kind, []):
            matches = [key for key, target in targets.items() if key[0] == kind and
                       (key[1] == row.get('name') or
                        ('path' in row and Path(target['src_path']).resolve() ==
                         (manifest_path.parent / row['path']).resolve()))]
            if len(matches) != 1:
                raise ValueError(f'ambiguous or missing manifest {kind} target')
            key = matches[0]
            target = targets[key]
            if (('name' in row and row['name'] != key[1]) or
                ('path' in row and (manifest_path.parent / row['path']).resolve() !=
                 Path(target['src_path']).resolve()) or key in bench_flags):
                raise ValueError(f'conflicting manifest {kind} target: {key[1]}')
            flag = row.get('bench', kind == 'bench')
            if not isinstance(flag, bool):
                raise ValueError('target bench flag must be boolean')
            bench_flags[key] = flag

    eligible = set()
    for key, target in targets.items():
        if not set(target.get('required-features', [])) <= features:
            continue
        if key[0] not in ('test', 'bench') or target['test'] or bench_flags.get(key, key[0] == 'bench'):
            eligible.add(key)
    if not compat or len(compat) != len(set(compat)):
        raise ValueError('compatibility targets must be nonempty and unique')
    for name in compat:
        if ('test', name) not in eligible:
            raise ValueError(f'missing eligible compatibility integration target: {name}')
    remaining = eligible - {('test', name) for name in compat}
    if not remaining:
        raise ValueError('default remainder contains no eligible targets')
    selectors = ['--lib'] if any(kind == 'lib' for kind, _ in targets) else []
    selectors += ['--bins', '--examples']
    for kind, name in sorted(remaining):
        if kind in ('test', 'bench'):
            selectors += [f'--{kind}', name]
    return selectors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compat', action='append', required=True)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    manifest_path = Path(__file__).resolve().parents[1] / 'Cargo.toml'
    try:
        boundary = command.index('--')
        result = subprocess.run(['cargo', 'metadata', '--locked', '--format-version', '1',
                                 '--manifest-path', str(manifest_path)], stdout=subprocess.PIPE,
                                encoding='utf-8', check=False)
        if result.returncode:
            return result.returncode
        with manifest_path.open('rb') as source:
            manifest = tomllib.load(source)
        selectors = remaining_targets(json.loads(result.stdout), manifest, manifest_path, args.compat)
        command[boundary:boundary] = selectors
        os.execvp(command[0], command)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f'Cannot select default test targets: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())

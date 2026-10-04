#!/usr/bin/env python3
"""Offline verification of dedicated funded-run Tor observations; no network or keys."""
import argparse
import collections
import ipaddress
import json
import os
from pathlib import Path
import shlex


def isolation_evidence(expected, events):
    if not expected:
        raise ValueError('empty identity map')
    labels = {}
    for label, entry in expected.items():
        credential = (entry['user'], entry['password'])
        if credential in labels:
            raise ValueError('distinct identity labels share credentials')
        labels[credential] = label
        if entry['kind'] not in ('evm', 'treasury', 'discovery'):
            raise ValueError('unknown identity kind')
    credentials = {}
    remote_hosts = set()
    circuits = collections.defaultdict(set)
    successes = collections.defaultdict(set)
    unknown = set()
    for line in events:
        fields = shlex.split(line)
        if fields[:2] != ['650', 'STREAM']:
            continue
        if len(fields) < 6:
            raise ValueError('malformed Tor stream event')
        stream, status, circuit, target = fields[2:6]
        attrs = dict(field.split('=', 1) for field in fields[6:] if '=' in field)
        if status == 'NEW' and attrs.get('PURPOSE') == 'USER' and not all(
                name in attrs for name in ('SOCKS_USERNAME', 'SOCKS_PASSWORD')):
            raise ValueError('user stream is missing isolation credentials')
        if 'SOCKS_USERNAME' in attrs or 'SOCKS_PASSWORD' in attrs:
            credential = (attrs.get('SOCKS_USERNAME'), attrs.get('SOCKS_PASSWORD'))
            if stream in credentials and credentials[stream] != credential:
                raise ValueError('stream credentials changed')
            credentials[stream] = credential
            if credential not in labels:
                unknown.add(stream)
        label = labels.get(credentials.get(stream))
        if label is None:
            continue  # Internal unauthenticated Tor streams; authenticated unknowns fail below.
        if status == 'NEW':
            host, separator, port = target.rpartition(':')
            if not separator or not port.isdigit() or not host:
                raise ValueError('invalid remote destination')
            try:
                ipaddress.ip_address(host.strip('[]'))
            except ValueError:
                pass
            else:
                raise ValueError('locally resolved stream destination')
            if expected[label]['kind'] == 'discovery' and target.lower() != expected[label]['target'].lower():
                raise ValueError('discovery identity used for a different origin')
            remote_hosts.add(stream)
        if circuit != '0':
            if not circuit.isdigit():
                raise ValueError('invalid circuit identifier')
            circuits[label].add(circuit)
        if status == 'SUCCEEDED':
            if circuit == '0' or stream not in remote_hosts:
                raise ValueError('successful stream lacks remote-host/circuit evidence')
            successes[label].add(stream)
    if unknown:
        raise ValueError(f'{len(unknown)} authenticated streams have unknown identities; evidence incomplete')
    required = {label for label, entry in expected.items() if entry['kind'] in ('evm', 'treasury')}
    if not required or not any(entry['kind'] == 'evm' for entry in expected.values()):
        raise ValueError('funded qualification requires treasury and wallet identities')
    if not any(entry['kind'] == 'treasury' for entry in expected.values()):
        raise ValueError('missing treasury identity')
    if any(not successes[label] for label in required):
        raise ValueError('missing successful streams for allocated wallet/treasury identities')
    owners = {}
    for label, assigned in circuits.items():
        for circuit in assigned:
            if circuit in owners and owners[circuit] != label:
                raise ValueError('distinct identities shared a Tor circuit')
            owners[circuit] = label
    return {label: {'kind': entry['kind'], 'successful_streams': len(successes[label]),
                    'circuits': len(circuits[label]),
                    'status': 'observed' if successes[label] else 'not_observed'}
            for label, entry in sorted(expected.items())}


def bounded_lines(path):
    with path.open() as stream:
        while line := stream.readline(1024 * 1024 + 1):
            if len(line) > 1024 * 1024:
                raise ValueError('Tor event line exceeds 1 MiB; evidence rejected without truncation')
            yield line


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--identities', type=Path, required=True)
    parser.add_argument('--events', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    with args.identities.open('rb') as stream:
        raw = stream.read(4 * 1024 * 1024 + 1)
    if len(raw) > 4 * 1024 * 1024:
        parser.error('identity map exceeds 4 MiB; rejected without truncation')
    try:
        result = isolation_evidence(json.loads(raw), bounded_lines(args.events))
    except (ValueError, KeyError) as error:
        parser.exit(1, f'isolation evidence rejected: {error}\n')
    with os.fdopen(os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'w') as stream:
        json.dump({'identities': result, 'scope': 'Tor stream isolation only; not proof of payment, settlement or application-level unlinkability'}, stream, indent=2)
        stream.write('\n')
    print('Tor identity separation verified; sanitized counts written to new output file.')


if __name__ == '__main__':
    main()

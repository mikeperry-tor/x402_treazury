#!/usr/bin/env python3
"""Opt-in macOS unsigned startup comparison through an existing Tor SOCKS port.
Does not start, stop or reconfigure Tor. Control observation is read-only unless
SIGNAL NEWNYM is explicitly authorized with --newnym.
No wallet environment.
"""
import argparse
import asyncio
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time
import tomllib
import uuid

ROOT = Path(__file__).resolve().parents[1]


def write_results(path, results):
    # A following sequence may be reading concurrently; publish complete JSON.
    temporary = path.with_suffix('.json.tmp')
    temporary.write_text(json.dumps(results, indent=2)+'\n')
    temporary.replace(path)


def build_times(path, require_ready=True):
    # Read only learning counters; never copy guard state or browser identities.
    fields = {}
    for line in path.read_text().splitlines():
        key, _, value = line.partition(' ')
        if key in ('TotalBuildTimes', 'CircuitBuildAbandonedCount', 'LastWritten'):
            fields[key] = value
    if require_ready and int(fields.get('TotalBuildTimes', '0')) < 100:
        raise ValueError('Tor saved state has fewer than 100 build-time samples; warm-up required')
    return fields


def learning_snapshot(args, require_ready=True):
    fields = build_times(args.tor_state, False)
    control = getattr(args, 'control', None)
    if control is not None:
        # A command round trip proves this observer remains connected to the same daemon.
        control.command('GETINFO version')
        events = [e for e in control.events if 'TOTAL_TIMES=' in e]
        fields['control_event_count'] = len(events)
        if events:
            fields['live_total_build_times'] = int(re.search(r'TOTAL_TIMES=(\d+)', events[-1])[1])
            fields['live_event'] = events[-1]
        count = fields.get('live_total_build_times', 0)
    else:
        count = int(fields.get('TotalBuildTimes', '0'))
    fields['qualified'] = count >= 100
    if require_ready and not fields['qualified']:
        raise ValueError('Tor learning is below 100 or no live event observed; timing paused')
    return fields


def select_providers(args, providers):
    if not getattr(args, 'exclude_issue_tagged', False):
        return providers
    kept, excluded = [], []
    for provider in providers:
        tags = tomllib.loads(provider.read_text()).get('reliability_tags', [])
        if tags:
            excluded.append({'provider': str(provider.relative_to(ROOT)) if provider.is_relative_to(ROOT) else str(provider),
                             'reliability_tags': tags})
        else:
            kept.append(provider)
    write_results(args.output/f'{args.label}-excluded-issues.json', excluded)
    print(json.dumps({'stage': 'provider_issue_filter', 'kept': len(kept), 'excluded': excluded}), flush=True)
    if not kept:
        raise ValueError('All providers excluded by reliability tags; no requests sent')
    return kept


def config(providers, concurrency, port, namespace):
    text = f'''version=1
[startup]
catalog_concurrency={concurrency}
[network]
mode="tor"
socks_endpoint="127.0.0.1:{port}"
isolation_namespace="{namespace}"
socks_auth="tor_extended"
[wallets.unused]
mode="static"
private_key_env="NEVER_READ_BY_STARTUP_PROBE"
'''
    names = []
    for provider in providers:
        name = provider.parent.name if provider.name == 'provider.toml' else provider.stem
        name = name.replace('-', '_')
        names.append(name)
        text += f'[sources.{name}]\nextends={json.dumps(str(provider))}\n'
    text += ('[servers.measure]\nlisten="127.0.0.1:0"\n'
             'bearer_token_env="NEVER_READ_BY_STARTUP_PROBE"\nwallet="unused"\n'
             f'sources={json.dumps(names)}\n')
    return text


async def run(args, label, binary, path, catalog_only=False, require_learning=True):
    state_before = learning_snapshot(args, require_learning)
    prefix = args.output / label
    if prefix.with_suffix('.stdout').exists():
        raise ValueError(f'Measurement {label} already exists; choose a new --label')
    command = ['/usr/bin/time', '-l', '-o', str(prefix.with_suffix('.resources')),
               '/usr/bin/sandbox-exec', '-f', str(args.output / 'socks-only.sb'),
               str(binary), '--meta-config', str(path)]
    if catalog_only:
        command.append('--catalog-only')
    started = time.monotonic()
    with prefix.with_suffix('.stdout').open('w') as stdout, prefix.with_suffix('.stderr').open('w') as stderr:
        child = await asyncio.create_subprocess_exec(
            *command, stdout=stdout, stderr=stderr, start_new_session=True,
            env={'PATH': '/usr/bin:/bin', 'RUST_BACKTRACE': '0'})
        timed_out = False
        try:
            await asyncio.wait_for(child.wait(), args.deadline)
        except asyncio.CancelledError:
            if child.returncode is None:
                os.killpg(child.pid, signal.SIGKILL)
                await child.wait()
            raise
        except asyncio.TimeoutError:
            timed_out = True
            os.killpg(child.pid, signal.SIGTERM)
            try:
                await asyncio.wait_for(child.wait(), 10)
            except asyncio.TimeoutError:
                os.killpg(child.pid, signal.SIGKILL)
                await child.wait()
    try:
        state_after = learning_snapshot(args, False)
    except Exception as error:
        state_after = {'qualified': False, 'error': str(error)}
    result = dict(label=label, returncode=child.returncode, timed_out=timed_out,
                  elapsed_seconds=time.monotonic()-started, tor_state_before=state_before,
                  tor_state_after=state_after)
    result['qualified'] = state_before['qualified'] and result['tor_state_after']['qualified']
    if getattr(args, 'control', None) is not None:
        events = [e for e in args.control.events if 'TOTAL_TIMES=' in e]
        result['qualified'] &= all(int(re.search(r'TOTAL_TIMES=(\d+)', e)[1]) >= 100
                                   for e in events[state_before['control_event_count']:])
    resources = prefix.with_suffix('.resources')
    if resources.exists():
        match = re.search(r'(\d+)\s+maximum resident set size', resources.read_text())
        if match:
            result['peak_rss_bytes'] = int(match[1])  # macOS units, per child process run
    if child.returncode == 0:
        result.update(json.loads(prefix.with_suffix('.stdout').read_text()))
    else:
        result['failure_log'] = str(prefix.with_suffix('.stderr'))
    log = prefix.with_suffix('.stderr').read_text()
    match = re.search(r'request_concurrency=(\d+)', log)
    if match:
        result['pricing_request_limit'] = int(match[1])
    result['pricing_attempts'] = log.count('pricing attempt cached; no automatic retry')
    result['pricing_no_price'] = sum('discovered=false' in line for line in log.splitlines()
                                   if 'pricing attempt cached;' in line)
    prefix.with_suffix('.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result), flush=True)
    return result


async def main(args):
    if not args.control:
        args.output.mkdir(parents=True, mode=0o700, exist_ok=False)
    elif (args.output/'provenance.json').exists():
        raise ValueError('output already contains a run; use --resume')
    namespace = 'startup_' + uuid.uuid4().hex
    provenance = dict(namespace=namespace, socks_port=args.socks_port,
                      saved_tor_state=learning_snapshot(args), binaries={},
                      source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
    for limit, binary in [(16, args.binary16), (32, args.binary32)]:
        provenance['binaries'][limit] = dict(path=str(binary), sha256=hashlib.file_digest(binary.open('rb'), 'sha256').hexdigest())
    (args.output/'provenance.json').write_text(json.dumps(provenance, indent=2)+'\n')
    (args.output/'socks-only.sb').write_text(
        '(version 1)\n(allow default)\n(deny network*)\n'
        f'(allow network-outbound (remote tcp "localhost:{args.socks_port}"))\n')
    providers = select_providers(args, sorted((ROOT/'providers').rglob('*.toml')))
    semaphore = asyncio.Semaphore(16)

    async def preflight(index, provider):
        async with semaphore:
            path = args.output/f'preflight-{index}.toml'
            path.write_text(config([provider], 16, args.socks_port, namespace))
            result = await run(args, f'preflight-{index}', args.binary16, path, True)
            return provider, result

    results = await asyncio.gather(*(preflight(i, p) for i, p in enumerate(providers)))
    cohort = [p for p, result in results if result['returncode'] == 0]
    (args.output/'cohort.json').write_text(json.dumps({str(p.relative_to(ROOT)): r for p, r in results}, indent=2)+'\n')
    if not cohort:
        raise RuntimeError('No catalog succeeded; no timing comparison possible')
    paths = {}
    for limit in [16, 32]:
        paths[limit] = args.output/f'cohort-{limit}.toml'
        paths[limit].write_text(config(cohort, limit, args.socks_port, namespace))
    # Warm the cohort's API origins as well as catalog origins, using normal identities.
    warm = await run(args, 'warmup', args.binary16, paths[16])
    if warm['returncode'] != 0:
        raise RuntimeError('Full warm-up failed; inspect logs before timing')
    await measured_sequence(args, paths)


async def measured_sequence(args, paths):
    # Fresh application/cache per run, same Tor daemon/namespace; alternate order.
    measured = []
    for index, limit in enumerate([16, 32, 32, 16]):
        measured.append(await run(args, f'{args.label}-{index}-{limit}',
                                  args.binary16 if limit == 16 else args.binary32, paths[limit]))
        write_results(args.output/f'{args.label}-results.json', measured)
        if not measured[-1]['qualified']:
            raise RuntimeError('Tor learning dropped below 100 during measurement; result retained but unqualified')


async def catalog_comparison(args):
    """Reuse a completed run's identities/cohort; retry download timeout exclusions."""
    provenance = json.loads((args.output/'provenance.json').read_text())
    previous = json.loads((args.output/'cohort.json').read_text())
    providers = []
    for relative, result in previous.items():
        if result['returncode'] == 0:
            providers.append(ROOT/relative)
        else:
            log = Path(result['failure_log']).read_text()
            if 'timed out' in log and not getattr(args, 'successful_cohort_only', False):
                providers.append(ROOT/relative)
    providers = select_providers(args, providers)
    (args.output/'catalog-comparison-cohort.json').write_text(
        json.dumps([str(p.relative_to(ROOT)) for p in providers], indent=2)+'\n')
    results = []
    for index, limit in enumerate(getattr(args, 'catalog_limits', None) or [3, 16, 32, 3]):
        path = args.output/f'catalog-only-{index}-{limit}.toml'
        path.write_text(config(providers, limit, provenance['socks_port'], provenance['namespace']))
        if args.control is None or not args.newnym:
            raise ValueError('Fresh-circuit comparison requires --control-cookie and explicit --newnym')
        event_start = len(args.control.events)
        args.control.command('SIGNAL NEWNYM')
        print(json.dumps({'stage':'newnym_and_spare_circuit_replenishment', 'seconds':30, 'catalog_concurrency':limit}), flush=True)
        await asyncio.sleep(30)
        if not any('650 SIGNAL NEWNYM' in e for e in args.control.events[event_start:]):
            raise RuntimeError('No observed NEWNYM completion event; comparison not started')
        results.append(await run(args, f'{args.label}-catalog-{index}-{limit}', args.binary16,
                                 path, catalog_only=True))
        write_results(args.output/f'{args.label}-catalog-results.json', results)
        if not results[-1]['qualified']:
            raise RuntimeError('Tor learning changed during catalog comparison; result retained but unqualified')


async def execute(args):
    if args.resume and args.exclude_issue_tagged and not args.catalog_comparison:
        raise ValueError('Issue filtering changes the cohort; start a new run instead of --resume, or use --catalog-comparison')
    if args.after_sequence_count < 1:
        raise ValueError('--after-sequence-count must be positive')
    args.control = None
    try:
        if args.control_cookie:
            from qualify_tor import Control
            args.output.mkdir(parents=True, mode=0o700, exist_ok=True)
            events_path = args.output/f'{args.label}-learning.events'
            if events_path.exists():
                raise ValueError('Learning event log already exists; choose a new --label')
            args.control = Control(('127.0.0.1', args.control_port), args.control_cookie.read_bytes(),
                                   events_path)
            args.control.command('SETEVENTS BUILDTIMEOUT_SET SIGNAL')
            if args.warm_learning:
                path = args.output/f'{args.label}-warm-learning.toml'
                warm_config = (args.output/'cohort-16.toml').read_text().replace(
                    'catalog_concurrency=16', 'catalog_concurrency=3')
                # Normal origin-isolated traffic under a fresh warm-up namespace
                # consumes spare circuits without issuing circuit-control commands.
                # Timed runs retain their original, shared namespace.
                warm_config = re.sub(r'isolation_namespace="[^"]+"',
                    f'isolation_namespace="learning_{uuid.uuid4().hex}"', warm_config)
                path.write_text(warm_config)
                await run(args, f'{args.label}-warm-learning', args.binary16, path,
                          catalog_only=True, require_learning=False)
            deadline = time.monotonic()+900
            report_at = 0
            while not learning_snapshot(args, False)['qualified']:
                if time.monotonic() > deadline:
                    raise RuntimeError('No live learning event with >=100 samples within 900 seconds')
                if time.monotonic() >= report_at:
                    print(json.dumps({'stage':'waiting_for_tor_learning', **learning_snapshot(args, False)}), flush=True)
                    report_at = time.monotonic()+30
                await asyncio.sleep(2)
        if args.after_sequence:
            deadline = time.monotonic()+1800
            while True:
                if args.after_sequence.exists():
                    previous = json.loads(args.after_sequence.read_text())
                    if any(not r.get('qualified', False) for r in previous):
                        raise RuntimeError('Preceding sequence lost learning qualification; no additional requests sent')
                    if len(previous) == args.after_sequence_count:
                        break
                if time.monotonic() > deadline:
                    raise RuntimeError('Preceding sequence did not complete within 1800 seconds')
                await asyncio.sleep(2)
        if args.catalog_comparison:
            await catalog_comparison(args)
        elif args.resume:
            paths = {n: args.output/f'cohort-{n}.toml' for n in [16, 32]}
            await measured_sequence(args, paths)
        else:
            await main(args)
    finally:
        if args.control is not None:
            args.control.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary16', type=Path, required=True)
    parser.add_argument('--binary32', type=Path, required=True)
    parser.add_argument('--tor-state', type=Path, required=True)
    parser.add_argument('--socks-port', type=int, default=9150)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--deadline', type=float, default=900)
    parser.add_argument('--after-sequence', type=Path, help='Observe learning while waiting for a four-run result file before sending any requests')
    parser.add_argument('--after-sequence-count', type=int, default=4,
                        help='Expected samples in --after-sequence (default: 4)')
    parser.add_argument('--newnym', action='store_true', help='Explicitly authorize SIGNAL NEWNYM on the shared Tor daemon between catalog comparison samples')
    parser.add_argument('--resume', action='store_true')
    parser.add_argument('--warm-learning', action='store_true', help='Untimed catalog warm-up at rolling 3 using existing cohort before awaiting live learning')
    parser.add_argument('--label', default='measured')
    parser.add_argument('--control-cookie', type=Path)
    parser.add_argument('--control-port', type=int, default=9151)
    parser.add_argument('--catalog-limits', nargs='+', type=int, choices=range(1, 65),
                        help='Catalog-only sample order (default: 3 16 32 3)')
    parser.add_argument('--exclude-issue-tagged', action='store_true',
                        help='Exclude any provider with authored reliability_tags; record names and tags explicitly')
    parser.add_argument('--successful-cohort-only', action='store_true',
                        help='Keep catalog comparisons on the successful preflight cohort, without retrying timeout exclusions')
    parser.add_argument('--catalog-comparison', action='store_true',
                        help='Reuse existing output cohort/identities; compare catalog-only 3/16/32/3, retrying timed-out catalogs')
    args = parser.parse_args()
    for name in ['binary16', 'binary32', 'tor_state', 'output']:
        setattr(args, name, getattr(args, name).resolve())
    try:
        asyncio.run(execute(args))
    except KeyboardInterrupt:
        parser.exit(130, 'Stopped measurement; owned child processes cancelled, Tor unchanged.\n')
    except (ValueError, RuntimeError, OSError) as error:
        parser.exit(1, f'error: {error}\n')

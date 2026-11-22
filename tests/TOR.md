# Live Tor qualification

The opt-in macOS runner qualifies live Tor circuit assignment and per-process
egress confinement (verified on macOS 26.6.2 with Tor 0.4.9.13; boundaries are
recorded in [COVERAGE.md](COVERAGE.md)). Local `network::http_tests` separately
cover Unix-socket SOCKS, Rustls HTTP/2, trust rejection, remote DNS, and
missing-socket failures without live Tor.

## Scope and prerequisites

Use a dedicated temporary Tor instance with an owner-only data directory, fresh
loopback SOCKS and control ports, cookie-authenticated control access, and
`IsolateSOCKSAuth`. You may invoke Tor Browser's bundled `tor` binary with its
own temporary state without touching the browser's data directory or control
socket. The control connection belongs solely to the test observer.

Public activity is limited to Tor bootstrap and a small number of read-only
HTTPS and lightwalletd calls to `zec.rocks` using dummy EVM addresses,
synthetic treasury IDs, and a fresh isolation namespace (no operator wallet,
`.env`, seed, NEAR quote, x402 payment, or broadcast). See
[qualification status](../docs/testing.md#qualification-status) for funded
scope.

## Running the qualification

Build first, before starting Tor or confining the client. Requires Python
3.11+ (standard library only) and an already-installed Tor binary (this
repository neither bundles nor downloads Tor):

```sh
make test CARGO_ARGS='--offline --test network --no-run --message-format=json' > /tmp/treazury-network-build.jsonl
python3 - <<'PYCODE'
import json
from pathlib import Path
for line in Path('/tmp/treazury-network-build.jsonl').read_text().splitlines():
    item = json.loads(line)
    if (item.get('reason') == 'compiler-artifact'
            and item.get('target', {}).get('name') == 'network'
            and item.get('executable')):
        print(item['executable'])
PYCODE
python3 scripts/qualify_tor.py \
  --tor '/Applications/Tor Browser.app/Contents/MacOS/Tor/tor' \
  --test-binary /absolute/path/printed/above
```

- Pass `--mcp-port 8381 --mcp-port 8382 --mcp-port 8383` to also qualify
  inbound loopback MCP sockets under the same confinement profile via the
  ignored `live_tor_inbound_listeners` test (never permitting outbound
  connections to MCP ports).
- The runner creates a fresh `torrc`, data directory, cookie, and ports, waits
  at most 300 seconds for bootstrap, and runs the three named ignored tests
  under a minimal environment with `TorExtended` credentials (four HTTPS GETs
  and four read-only gRPC calls across two dummy EVM identities, treasury, and
  discovery).
- Evidence writes to `target/tor-qualification/<timestamp>-<id>/` (`result.json`,
  Tor version, source revision, binary hash, commands, logs, control events,
  and sandbox profile), cleaning up temporary Tor state on exit while keeping
  failed evidence intact.

Test the observer logic offline:

```sh
python3 -m unittest discover -s scripts/tests -p 'test_tor_qualification.py'
```

## Execution and acceptance

1. Start dedicated Tor with a 300-second bootstrap limit, authenticate its
   loopback control port, subscribe to stream and circuit events, and verify
   SOCKS isolation settings.
2. Run read-only HTTPS and gRPC requests through `src/network.rs` with normal
   TLS validation and correlate SOCKS credentials with Tor streams and
   circuits: every identity must appear, distinct identities must never share
   a circuit, and repeated identities may reuse or switch circuits.
3. Repeat under an OS confinement policy restricting the client process to the
   loopback SOCKS port (while Tor runs unconfined). On macOS, a temporary
   `sandbox-exec` profile expresses this rule:

   ```scheme
   (version 1)
   (allow default)
   (deny network*)
   (allow network-outbound (remote tcp "localhost:SOCKS_PORT"))
   ```

   Validate enforcement with unconfined positive controls and confined negative
   IPv4/IPv6 TCP and UDP DNS probes (checking resolver IPC behavior too; use
   an isolated Linux network namespace if macOS cannot enforce the required
   boundary).
4. Stop Tor and verify new HTTP and gRPC calls fail within their timeouts
   without direct fallback while negative probes stay denied.
5. Reap every child, remove temporary Tor state and cookies (never copying
   cookies into logs or commits), and retain sanitized evidence under `target/`.

For a standalone read-only smoke check against an existing Tor SOCKS endpoint
(without circuit observation or confinement):

```sh
TOR_SMOKE_SOCKS=127.0.0.1:SOCKS_PORT make test CARGO_ARGS='--offline --test network live_tor_unfunded_smoke' TEST_ARGS='--ignored --exact --nocapture'
```

See Tor's [SOCKS extensions](https://spec.torproject.org/socks-extensions.html)
and [control events](https://spec.torproject.org/control-spec/replies.html).
Circuit separation does not guarantee distinct exit IPs or hide application
payload identifiers.

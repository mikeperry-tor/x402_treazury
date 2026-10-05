# Real Tor qualification

The opt-in macOS runner qualifies real circuit assignment and per-process egress
confinement. It passed on macOS 26.6.2 with Tor 0.4.9.13; the results and their
boundaries are recorded in [COVERAGE.md](COVERAGE.md). Local authenticated SOCKS
and Docker consensus tests remain separate qualifications.

## Scope and prerequisites

Use a dedicated temporary Tor instance, with an owner-only data directory, fresh
loopback SOCKS/control ports, cookie-authenticated control access and
`IsolateSOCKSAuth`. Tor Browser's bundled Tor binary can be used independently;
record its version and do not use the browser's data directory or control socket.
The application continues to use its ordinary network factory and configured SOCKS
credential encoding. The control connection belongs only to the test observer.

Public activity consists of Tor bootstrap/relay traffic and a small number of
read-only HTTPS/lightwalletd calls to `zec.rocks`. Use public dummy EVM addresses,
synthetic treasury IDs and a fresh isolation namespace. No operator wallet,
`.env`, seed, NEAR quote, x402 payment or transaction broadcast is needed. A funded
public swap is a separate qualification; see the reproducible
[qualification scope](../docs/testing.md#qualification-status).

## Running the qualification

Build before starting Tor or applying the process network restriction. The runner
requires Python 3.11+ (standard library only) and an already installed Tor binary;
the repository neither bundles nor downloads Tor. Discover the exact default-build
integration-test executable from Cargo's output:

```sh
scripts/zcash.sh test --offline --test network --no-run --message-format=json > /tmp/treazury-network-build.jsonl
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

For the funded provider deployment, add `--mcp-port 8381 --mcp-port 8382
--mcp-port 8383`. This qualifies incoming loopback connections and responses on
those exact ports under the same profile as the negative egress probes and real
Tor HTTPS/gRPC phases. It does not permit outbound connections to MCP ports.
The additional ignored `live_tor_inbound_listeners` test requires the external
observer; default tests never open these fixed ports. The listener check verifies
socket permissions; the ordinary MCP tests separately cover auth and tool routing.

Run from a terminal permitted to start local listeners and Tor. The runner creates
a fresh regular `torrc`, data directory, cookie and dynamic ports, waits at most
300 seconds for bootstrap, and invokes only the three named ignored tests. It
checks the default TorExtended credential encoding. The client subprocess receives
a minimal environment without wallet keys or `.env` loading. Each smoke phase makes
four HTTPS GETs and four read-only gRPC calls, using two dummy EVM identities plus
treasury and discovery identities. The repeated EVM identity spans HTTPS and gRPC.

Evidence is written to `target/tor-qualification/<timestamp>-<id>/`: Tor version,
platform, source revision/status, executable hash, exact phase commands, client
logs, control events, sandbox profile and `result.json`. Only dummy isolation
credentials appear in events. The runner stops its processes and removes temporary
Tor state on success or failure. A failed assertion exits nonzero and leaves the
failed evidence intact. Ordinary Cargo tests do not select these live checks.

Test the observer offline with:

```sh
python3 -m unittest discover -s scripts/tests -p 'test_tor_qualification.py'
```

Those tests reject missing requests/credentials/success events, locally resolved
stream destinations, changed credentials and circuits shared across identities;
they allow an identity to use multiple circuits and ignore unrelated events.

## Execution and acceptance

1. Start the independent Tor process with a bounded bootstrap deadline. Authenticate
   its loopback control connection and subscribe to stream/circuit events before
   running application requests. Verify the effective SOCKS isolation settings.
2. Perform read-only HTTPS and gRPC requests using the production network factory
   and its normal TLS verification. Correlate expected SOCKS credentials with Tor's
   observed streams and assigned circuits. Every tested identity must be observed;
   streams from distinct isolation identities must not share a circuit. Repeated
   identities may reuse a circuit; do not require Tor always to choose the same one.
   Missing event evidence makes the qualification incomplete.
3. Repeat under an OS policy confining the **client process** to the selected
   loopback SOCKS TCP port. Tor runs outside that restriction. On macOS a temporary
   `sandbox-exec` profile can express the socket restriction:

   ```scheme
   (version 1)
   (allow default)
   (deny network*)
   (allow network-outbound (remote tcp "localhost:SOCKS_PORT"))
   ```

   Substitute the actual numeric port. Validate enforcement with controlled
   loopback IPv4/IPv6 TCP and UDP/DNS negative probes; establish successful
   unrestricted controls first. Check resolver/IPC behavior as well as socket
   denial before claiming DNS confinement. If macOS cannot enforce the intended
   boundary, record the limitation and use an isolated Linux test namespace with
   the same allowlist. Do not silently claim whole-host DNS protection from a
   per-process socket rule. No global host firewall edits are part of this test.
4. Stop the dedicated Tor process and attempt new application HTTP/gRPC connections.
   They must fail within their deadlines without direct fallback. The negative
   probes must remain denied. Record uncertainty rather than treating an unrelated
   public-service failure as evidence of successful isolation.
5. Reap every child and remove the temporary Tor state/control cookie. Retain only
   version, command, assertion and dummy-identity event evidence under ignored
   `target/` output. Do not copy control authentication cookies into logs or commits.

For a read-only smoke check alone against an independently managed Tor endpoint
(without circuit observation or OS confinement):

```sh
TOR_SMOKE_SOCKS=127.0.0.1:SOCKS_PORT scripts/zcash.sh test --offline \
  --test network live_tor_unfunded_smoke -- --ignored --exact --nocapture
```

Tor's [SOCKS extensions](https://spec.torproject.org/socks-extensions.html) define
credential-based stream isolation. Its [control events](https://spec.torproject.org/control-spec/replies.html)
provide stream/circuit observations. Circuit separation does not promise unique
exit IPs or remove application-level identifiers; record the tested boundary.

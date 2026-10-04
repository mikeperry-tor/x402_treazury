#!/usr/bin/env python3
"""Opt-in macOS Tor qualification; public dummy identities and read-only zec.rocks.
Build the Rust network integration test first. No application runtime dependency.
"""
import argparse
import collections
import datetime
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import queue
import shlex
import socket
import subprocess
import sys
import tempfile
import threading
import time
import uuid


def circuit_evidence(log, events):
    expected = {}
    requests = collections.Counter()
    for line in log.splitlines():
        if line.startswith("TOR_IDENTITY "):
            value = json.loads(line.removeprefix("TOR_IDENTITY "))
            label = value["label"]
            if label in expected:
                raise ValueError("duplicate expected identity")
            expected[label] = (value["user"], value["password"])
        if line.startswith("TOR_REQUEST "):
            value = json.loads(line.removeprefix("TOR_REQUEST "))
            requests[value["label"], value["protocol"]] += 1
    required = collections.Counter({("evm_a", "https"): 2, ("evm_b", "https"): 1,
                                    ("discovery", "https"): 1, ("treasury", "grpc"): 1,
                                    ("evm_a", "grpc"): 1})
    if set(expected) != {"evm_a", "evm_b", "treasury", "discovery"} or requests != required:
        raise ValueError("missing identity/request evidence")
    if len(set(expected.values())) != 4:
        raise ValueError("dummy identities unexpectedly share credentials")
    labels = {credential: label for label, credential in expected.items()}
    credentials = {}
    successes = collections.defaultdict(set)
    circuits = collections.defaultdict(set)
    hosts = set()
    for line in events:
        fields = shlex.split(line)
        if fields[:2] != ["650", "STREAM"]:
            continue
        if len(fields) < 6:
            raise ValueError("malformed stream event")
        stream, status, circuit, target = fields[2:6]
        attrs = dict(field.split("=", 1) for field in fields[6:] if "=" in field)
        if "SOCKS_USERNAME" in attrs and "SOCKS_PASSWORD" in attrs:
            credential = (attrs["SOCKS_USERNAME"], attrs["SOCKS_PASSWORD"])
            if stream in credentials and credentials[stream] != credential:
                raise ValueError("stream credentials changed")
            credentials[stream] = credential
        label = labels.get(credentials.get(stream))
        if label is None:
            continue  # Tor's internal streams or an earlier phase's namespace.
        if status == "NEW":
            if target.lower() != "zec.rocks:443":
                raise ValueError("unexpected target for qualification identity")
            hosts.add(label)
        if circuit != "0":
            circuits[label].add(circuit)
        if status == "SUCCEEDED":
            if circuit == "0":
                raise ValueError("successful stream has no circuit")
            successes[label].add(stream)
    if hosts != set(expected) or any(not successes[label] for label in expected):
        raise ValueError("missing successful Tor stream/remote-host evidence")
    if len(successes["evm_a"]) < 2:
        raise ValueError("missing separate HTTPS/gRPC streams for shared EVM identity")
    for label, assigned in circuits.items():
        for other, theirs in circuits.items():
            if label != other and assigned & theirs:
                raise ValueError("distinct identities shared a Tor circuit")
    return {label: {"circuits": sorted(circuits[label]), "streams": sorted(successes[label])}
            for label in sorted(expected)}


class Control:
    def __init__(self, address, cookie, event_file):
        self.sock = socket.create_connection(address, timeout=5)
        self.sock.settimeout(None)
        self.replies = queue.Queue()
        self.events = []
        self.failure = None
        self.file = event_file.open("w")
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()
        try:
            self.command("AUTHENTICATE " + cookie.hex())  # Never log the cookie/command.
        except BaseException:
            self.close()
            raise

    def read(self):
        try:
            with self.sock.makefile("rb") as stream:
                while line := stream.readline(1024 * 1024 + 1):
                    if len(line) > 1024 * 1024:
                        raise ValueError("Tor control line exceeds 1 MiB; qualification aborted, no silent truncation")
                    text = line.decode().rstrip("\r\n")
                    if text.startswith("650 "):
                        self.events.append(text)
                        self.file.write(text + "\n")
                        self.file.flush()
                    else:
                        self.replies.put(text)
        except Exception as error:
            self.failure = str(error)
        finally:
            self.replies.put(None)

    def command(self, command):
        self.sock.sendall((command + "\r\n").encode())
        lines = []
        while True:
            line = self.replies.get(timeout=10)
            if line is None:
                raise RuntimeError("Tor control connection ended")
            lines.append(line)
            if len(line) >= 4 and line[:3].isdigit() and line[3] == " ":
                if not line.startswith("250 "):
                    raise RuntimeError("Tor control command rejected: " + line)
                return lines

    def close(self):
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.sock.close()
        self.reader.join(timeout=5)
        self.file.close()


def terminate(process):
    if process is not None and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=10)


def client_profile(socks_port, mcp_ports=()):
    ports = [socks_port, *mcp_ports]
    if any(type(p) is not int or not 1 <= p <= 65535 for p in ports):
        raise ValueError("SOCKS/MCP ports must be integers in 1..65535")
    if len(set(ports)) != len(ports):
        raise ValueError("SOCKS/MCP ports must be distinct")
    rules = ['(version 1)', '(allow default)', '(deny network*)',
             f'(allow network-outbound (remote tcp "localhost:{socks_port}"))']
    for port in mcp_ports:
        rules.extend([f'(allow network-bind (local tcp "localhost:{port}"))',
                      f'(allow network-inbound (local tcp "localhost:{port}"))'])
    return '\n'.join(rules) + '\n'


def check_inbound(binary, profile, ports, environment, out):
    command = ['/usr/bin/sandbox-exec', '-f', str(profile), str(binary), '--exact',
               'live_tor_inbound_listeners', '--ignored', '--nocapture']
    with (out / 'inbound.log').open('w') as log:
        child = subprocess.Popen(command, env={**environment, 'TOR_MCP_PORTS': ','.join(map(str, ports))},
                                 stdout=log, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + 20
            for port in ports:
                while True:
                    try:
                        connection = socket.create_connection(('127.0.0.1', port), timeout=1)
                        break
                    except OSError:
                        if child.poll() is not None or time.monotonic() > deadline:
                            raise RuntimeError('confined listener did not start; see inbound.log')
                        time.sleep(0.1)
                with connection:
                    connection.settimeout(5)
                    connection.sendall(b'qualification')
                    if connection.recv(64) != b'accepted':
                        raise RuntimeError('confined listener response mismatch')
            if child.wait(timeout=10) != 0:
                raise RuntimeError('confined listener test failed; see inbound.log')
        finally:
            terminate(child)
    return {'status': 'passed', 'ports': ports, 'command': command}
class Probes:
    def __init__(self):
        self.sockets = []
        self.env = {}
        self.counts = collections.Counter()
        self.stop = threading.Event()
        self.threads = []
        for name in ["TCP4", "TCP6", "UDP4", "UDP6"]:
            family = socket.AF_INET6 if name.endswith("6") else socket.AF_INET
            kind = socket.SOCK_STREAM if name.startswith("TCP") else socket.SOCK_DGRAM
            sock = socket.socket(family, kind)
            sock.bind(("::1" if family == socket.AF_INET6 else "127.0.0.1", 0))
            if kind == socket.SOCK_STREAM:
                sock.listen()
            sock.settimeout(0.2)
            host, port = sock.getsockname()[:2]
            self.env["TOR_CONTROL_" + name] = f"[{host}]:{port}" if family == socket.AF_INET6 else f"{host}:{port}"
            self.sockets.append(sock)
            thread = threading.Thread(target=self.serve, args=(name, sock, kind), daemon=True)
            thread.start()
            self.threads.append(thread)

    def serve(self, name, sock, kind):
        while not self.stop.is_set():
            try:
                if kind == socket.SOCK_STREAM:
                    connection, _ = sock.accept()
                    self.counts[name] += 1
                    connection.close()
                else:
                    data, address = sock.recvfrom(1024)
                    self.counts[name] += 1
                    sock.sendto(data, address)
            except socket.timeout:
                pass
            except OSError:
                return

    def close(self):
        self.stop.set()
        for sock in self.sockets:
            sock.close()
        for thread in self.threads:
            thread.join(timeout=2)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tor", type=Path, required=True)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--mcp-port", type=int, action="append", default=[],
                        help="also qualify inbound-only loopback listener allowance (repeatable)")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("this runner implements macOS sandbox-exec; other OS policies require separate qualification")
    binary, tor_path = args.test_binary.resolve(), args.tor.resolve()
    root = Path(__file__).resolve().parents[1]
    out = root / "target" / "tor-qualification" / (datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ-") + uuid.uuid4().hex[:8])
    out.mkdir(parents=True, mode=0o700)
    print(f"Evidence: {out}", flush=True)
    result = {"status": "running", "test_binary_sha256": hashlib.file_digest(binary.open("rb"), "sha256").hexdigest(),
              "tor_version": subprocess.check_output([str(tor_path), "--version"], text=True), "phases": {}}
    result["source_revision"] = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    result["source_status"] = subprocess.check_output(["git", "status", "--short"], cwd=root, text=True)
    result["platform"] = subprocess.check_output(["sw_vers"], text=True)
    process = control = probes = None
    temporary = tempfile.TemporaryDirectory(prefix="treazury-tor-")
    try:
        scratch = Path(temporary.name)
        torrc = scratch / "torrc"
        torrc.write_text("# Dedicated qualification instance; no browser configuration.\n")
        control_file = scratch / "control.port"
        with (out / "tor.log").open("w") as log:
            process = subprocess.Popen([str(tor_path), "-f", str(torrc), "--DataDirectory", str(scratch),
                "--ClientOnly", "1", "--SocksPort", "auto IsolateSOCKSAuth", "--ControlPort", "auto",
                "--ControlPortWriteToFile", str(control_file), "--CookieAuthentication", "1",
                "--AvoidDiskWrites", "1", "--Log", "notice stdout"], stdout=log, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + 300
        while not control_file.exists() or not (scratch / "control_auth_cookie").exists():
            if process.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("Tor did not open control listener; see tor.log")
            time.sleep(0.2)
        host, port = control_file.read_text().strip().removeprefix("PORT=").rsplit(":", 1)
        if not ipaddress.ip_address(host).is_loopback:
            raise RuntimeError("control listener is not loopback")
        control = Control((host, int(port)), (scratch / "control_auth_cookie").read_bytes(), out / "events.log")
        settings = control.command("GETCONF SocksPort")
        if not any("IsolateSOCKSAuth" in line for line in settings):
            raise RuntimeError("missing SOCKS isolation configuration")
        result["socks_settings"] = settings
        listeners = control.command("GETINFO net/listeners/socks")
        listener = next(line.split("=", 1)[1] for line in listeners if "net/listeners/socks=" in line)
        socks = shlex.split(listener)[0]
        if not ipaddress.ip_address(socks.rsplit(":", 1)[0]).is_loopback:
            raise RuntimeError("SOCKS listener is not loopback")
        print("Tor started; waiting for bootstrap", flush=True)
        while True:
            bootstrap = control.command("GETINFO status/bootstrap-phase")
            if any("PROGRESS=100" in line for line in bootstrap):
                break
            if time.monotonic() > deadline:
                raise RuntimeError("Tor bootstrap exceeded 300 seconds; see tor.log")
            time.sleep(1)
        control.command("SETEVENTS STREAM CIRC")
        profile = out / "client.sb"
        profile.write_text(client_profile(int(socks.rsplit(':', 1)[1]), args.mcp_port))
        probes = Probes()
        environment = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "RUST_BACKTRACE": "0", "TOR_SMOKE_SOCKS": socks, **probes.env}
        if args.mcp_port:
            result['phases']['inbound'] = check_inbound(binary, profile, args.mcp_port, environment, out)

        def run(name, test, sandbox, extra=None, timeout=600):
            print(f"Running {name}", flush=True)
            command = (["/usr/bin/sandbox-exec", "-f", str(profile)] if sandbox else []) + [str(binary), "--exact", test, "--ignored", "--nocapture"]
            with (out / (name + ".log")).open("w") as log:
                child = subprocess.Popen(command, env={**environment, **(extra or {})}, stdout=log, stderr=subprocess.STDOUT, cwd=root)
                try:
                    code = child.wait(timeout=timeout)
                finally:
                    terminate(child)
            text = (out / (name + ".log")).read_text()
            if code or "1 passed; 0 failed" not in text:
                raise RuntimeError(f"{name} failed; see {name}.log")
            result["phases"][name] = {"status": "passed", "command": command}
            return text

        run("egress-positive", "live_tor_egress_controls", False, {"TOR_EGRESS_DENIED": "0"}, 30)
        time.sleep(0.3)
        baseline = probes.counts.copy()
        if any(baseline[name] < 1 for name in ["TCP4", "TCP6", "UDP4", "UDP6"]):
            raise RuntimeError("control listeners did not observe all unrestricted probes")
        run("egress-denied", "live_tor_egress_controls", True, {"TOR_EGRESS_DENIED": "1"}, 30)
        time.sleep(0.3)
        if probes.counts != baseline:
            raise RuntimeError("sandboxed probes reached forbidden listeners")
        for phase, sandbox in [("unrestricted", False), ("confined", True)]:
            text = run(phase, "live_tor_unfunded_smoke", sandbox,
                       {"TOR_SMOKE_NAMESPACE": "qualification_" + uuid.uuid4().hex})
            time.sleep(0.5)
            if control.failure:
                raise RuntimeError(control.failure)
            evidence = circuit_evidence(text, control.events.copy())
            result["phases"][phase]["identities"] = evidence
            print(f"{phase}: all four identities observed on separate circuits", flush=True)
        terminate(process)
        print("Dedicated Tor stopped", flush=True)
        run("proxy-stopped", "live_tor_proxy_unavailable", True, timeout=30)
        run("egress-still-denied", "live_tor_egress_controls", True, {"TOR_EGRESS_DENIED": "1"}, 30)
        if probes.counts != baseline:
            raise RuntimeError("forbidden listener traffic after Tor stopped")
        result["probe_counts"] = dict(probes.counts)
        result["status"] = "passed"
    except BaseException as error:
        result["status"] = "failed"
        result["error"] = str(error)
        raise
    finally:
        if control is not None:
            control.close()
        terminate(process)
        if probes is not None:
            probes.close()
        temporary.cleanup()
        (out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        print(f"Qualification {result['status']}; evidence: {out}", flush=True)


if __name__ == "__main__":
    main()

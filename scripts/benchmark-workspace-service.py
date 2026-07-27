#!/usr/bin/env python3
"""Compare current workspace-service paths against a stable AGS binary."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

# A p95 over nine observations degenerates to the single maximum and turns
# scheduler jitter into a release verdict. Twenty-one remains a small fixed
# sample while making p95 independent of one isolated outlier.
SAMPLES = 21


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, math.ceil(len(ordered) * fraction) - 1)]


class Bench:
    def __init__(
        self, binary: Path, source: Path, route_schema: str, label: str
    ) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix=f"ags-bench-{label}-")
        root = Path(self.temp.name)
        self.home = root / "home"
        self.runtime = root / "runtime"
        self.project = root / "project"
        for path in (self.home, self.runtime, self.project):
            path.mkdir()
        subprocess.run(["git", "init", "--quiet"], cwd=self.project, check=True)
        self.binary = binary
        self.route_schema = route_schema
        self.env = os.environ.copy()
        self.env.update(
            {
                "HOME": str(self.home),
                "USERPROFILE": str(self.home),
                "AGS_RUNTIME_HOME": str(self.runtime),
                "AGS_SOURCE_ROOT": str(source),
                "AGS_WORKSPACE_IDLE_MS": "60000",
                "AGS_THIRD_PARTY_MANIFEST_OFFLINE": "1",
            }
        )
        self.run(
            "init",
            "--target",
            str(self.project),
            "--mode",
            "local",
            "--format",
            "json",
        )
        self.run(
            "capability",
            "snapshot",
            "--host",
            "codex",
            "--target",
            str(self.project),
            "--write",
            "--format",
            "json",
        )

    def run(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [self.binary, *args],
            cwd=self.project,
            env=self.env,
            check=True,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

    def timed_cli(self, *args: str) -> float:
        started = time.perf_counter()
        self.run(*args)
        return (time.perf_counter() - started) * 1000

    @staticmethod
    def request(process: subprocess.Popen[str], request: dict) -> dict:
        assert process.stdin is not None
        assert process.stdout is not None
        process.stdin.write(json.dumps(request, separators=(",", ":")) + "\n")
        process.stdin.flush()
        line = process.stdout.readline()
        if not line:
            stderr = process.stderr.read() if process.stderr else ""
            raise RuntimeError(f"MCP adapter closed: {stderr}")
        result = json.loads(line)
        if "error" in result:
            raise RuntimeError(str(result["error"]))
        return result["result"]

    def mcp_sample(self) -> tuple[float, float, int]:
        started = time.perf_counter()
        process = subprocess.Popen(
            [self.binary, "mcp", "serve", "--transport", "stdio"],
            cwd=self.project,
            env=self.env,
            text=True,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            self.request(
                process,
                {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": {},
                        "clientInfo": {"name": "ags-benchmark", "version": "1"},
                    },
                },
            )
            self.request(
                process,
                {
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tools/call",
                    "params": {
                        "name": "ags_preflight",
                        "arguments": {
                            "agent": "codex",
                            "target": str(self.project),
                        },
                    },
                },
            )
            reconnect_ms = (time.perf_counter() - started) * 1000
            route_started = time.perf_counter()
            self.request(
                process,
                {
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "tools/call",
                    "params": {
                        "name": "ags_route_request",
                        "arguments": {
                            "proposal": {
                                "schema_version": self.route_schema,
                                "request_fingerprint": "sha256:benchmark-request",
                                "phase": "execution",
                                "solution_state": "confirmed",
                                "execution_authority": "none",
                                "scope_hash": "sha256:benchmark-scope",
                                "targets": [
                                    {
                                        "kind": "machine_cli",
                                        "capability": "project_verify",
                                        "input": {"kind": "empty"},
                                    }
                                ],
                            }
                        },
                    },
                },
            )
            route_ms = (time.perf_counter() - route_started) * 1000
            daemon_pid = self.workspace_authority_pid(process.pid)
            return (
                reconnect_ms,
                route_ms,
                self.session_rss_kib(process.pid, daemon_pid),
            )
        finally:
            if process.stdin:
                process.stdin.close()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()

    @staticmethod
    def process_rss_kib(pid: int) -> int:
        output = subprocess.run(
            ["ps", "-o", "rss=", "-p", str(pid)],
            check=True,
            text=True,
            stdout=subprocess.PIPE,
        ).stdout
        return int(output.strip())

    def workspace_authority_pid(self, adapter_pid: int) -> int:
        """Return the daemon PID, or the adapter PID for a pre-daemon baseline."""
        key = hashlib.sha256(str(self.project.resolve()).encode()).hexdigest()
        registry_path = self.runtime / "workspace-services" / f"{key}.json"
        if not registry_path.is_file():
            return adapter_pid
        registry = json.loads(registry_path.read_text())
        return int(registry["pid"])

    def session_rss_kib(self, adapter_pid: int, daemon_pid: int) -> int:
        """Measure every AGS process simultaneously serving this MCP session.

        A single-process baseline has identical adapter and service PIDs, counted
        once. A daemon release has a live stdio adapter plus the workspace
        daemon; both are counted. This keeps the baseline and candidate on the
        same total-process footprint rather than comparing one process from
        each architecture.
        """
        return sum(
            self.process_rss_kib(pid) for pid in sorted({adapter_pid, daemon_pid})
        )

    def measure(self) -> dict:
        # Warm all four paths once before collecting fixed samples.
        self.timed_cli(
            "session", "preflight", "--for", "codex", "--target", str(self.project)
        )
        self.timed_cli(
            "capability",
            "snapshot",
            "--host",
            "codex",
            "--target",
            str(self.project),
            "--write",
            "--format",
            "json",
        )
        self.mcp_sample()

        samples = {
            "preflight_ms": [],
            "snapshot_refresh_ms": [],
            "daemon_reconnect_ms": [],
            "route_request_ms": [],
        }
        rss = []
        for _ in range(SAMPLES):
            samples["preflight_ms"].append(
                self.timed_cli(
                    "session",
                    "preflight",
                    "--for",
                    "codex",
                    "--target",
                    str(self.project),
                )
            )
            samples["snapshot_refresh_ms"].append(
                self.timed_cli(
                    "capability",
                    "snapshot",
                    "--host",
                    "codex",
                    "--target",
                    str(self.project),
                    "--write",
                    "--format",
                    "json",
                )
            )
            reconnect, route, authority_rss = self.mcp_sample()
            samples["daemon_reconnect_ms"].append(reconnect)
            samples["route_request_ms"].append(route)
            rss.append(authority_rss)
        return {
            name: {
                "median": statistics.median(values),
                "p95": percentile(values, 0.95),
            }
            for name, values in samples.items()
        } | {"peak_rss_kib": max(rss)}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--baseline-source-root", type=Path, required=True)
    parser.add_argument("--candidate-source-root", type=Path, required=True)
    parser.add_argument("--baseline-route-schema", required=True)
    parser.add_argument("--candidate-route-schema", required=True)
    args = parser.parse_args()
    baseline = Bench(
        args.baseline.resolve(),
        args.baseline_source_root.resolve(),
        args.baseline_route_schema,
        "baseline",
    )
    candidate = Bench(
        args.candidate.resolve(),
        args.candidate_source_root.resolve(),
        args.candidate_route_schema,
        "candidate",
    )
    baseline_result = baseline.measure()
    candidate_result = candidate.measure()

    failures = []
    for name in (
        "preflight_ms",
        "snapshot_refresh_ms",
        "daemon_reconnect_ms",
        "route_request_ms",
    ):
        if candidate_result[name]["median"] > baseline_result[name]["median"] * 1.05:
            failures.append(f"{name} median exceeds 105%")
        if candidate_result[name]["p95"] > baseline_result[name]["p95"] * 1.10:
            failures.append(f"{name} p95 exceeds 110%")
    if candidate_result["peak_rss_kib"] > baseline_result["peak_rss_kib"] * 1.10:
        failures.append("peak RSS exceeds 110%")

    report = {
        "schema_version": "ags-workspace-performance/1",
        "samples": SAMPLES,
        "rss_scope": "live_stdio_adapter_plus_workspace_daemon_unique_pids",
        "thresholds": {"median_ratio": 1.05, "p95_ratio": 1.10, "rss_ratio": 1.10},
        "baseline": baseline_result,
        "candidate": candidate_result,
        "failures": failures,
        "status": "pass" if not failures else "fail",
    }
    print(json.dumps(report, indent=2, sort_keys=True))
    raise SystemExit(0 if not failures else 1)


if __name__ == "__main__":
    main()

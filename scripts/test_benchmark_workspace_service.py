#!/usr/bin/env python3

import importlib.util
import unittest
from pathlib import Path
from unittest import mock


MODULE_PATH = Path(__file__).with_name("benchmark-workspace-service.py")
SPEC = importlib.util.spec_from_file_location("benchmark_workspace_service", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
benchmark = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(benchmark)


class WorkspaceServiceRssTest(unittest.TestCase):
    def test_session_rss_sums_live_adapter_and_daemon(self) -> None:
        bench = object.__new__(benchmark.Bench)
        with mock.patch.object(
            benchmark.Bench,
            "process_rss_kib",
            side_effect=lambda pid: {101: 11_000, 202: 23_000}[pid],
        ):
            self.assertEqual(bench.session_rss_kib(101, 202), 34_000)

    def test_session_rss_does_not_double_count_single_process_baseline(self) -> None:
        bench = object.__new__(benchmark.Bench)
        with mock.patch.object(
            benchmark.Bench, "process_rss_kib", return_value=17_000
        ):
            self.assertEqual(bench.session_rss_kib(101, 101), 17_000)


if __name__ == "__main__":
    unittest.main()

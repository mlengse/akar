"""P53.9 — gap-capture hooks for the KuzuDB compat harness.

Collects every failing test and writes a machine readable
``test_kuzu_compat.gap_report.json`` next to this file so the P53.9 gap
list survives the run even when the suite is big.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

_GAPS: list[dict] = []
_REPORT = Path(__file__).parent / "test_kuzu_compat.gap_report.json"


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_makereport(item: pytest.Item, call):  # noqa: ANN001
    outcome = yield
    rep = outcome.get_result()
    if rep.when == "call" and rep.failed:
        msg = str(rep.longrepr).splitlines()[-1] if rep.longrepr else "?"
        _GAPS.append({"test": item.name, "method": item.name.replace("test_", ""), "error": msg[:400]})


def pytest_sessionfinish(session: pytest.Session, exitstatus: int):  # noqa: ANN001
    if _GAPS:
        _REPORT.write_text(json.dumps(_GAPS, indent=2), encoding="utf-8")
        print(f"\n[P53.9] {len(_GAPS)} gap(s) recorded -> {_REPORT}")

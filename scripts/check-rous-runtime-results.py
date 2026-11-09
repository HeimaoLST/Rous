#!/usr/bin/env python3
"""Require the real Rous client contract fixture to execute once and pass."""
from pathlib import Path
import re
import sys

REQUIRED = "client::tests::real_runtime_contract"


def main(path: str) -> int:
    try:
        log = Path(path).read_text()
    except OSError as error:
        print(f"Rous compatibility checker: {error}", file=sys.stderr)
        return 1
    log = re.sub(r"\x1b\[[0-9;]*m", "", log)
    summary = re.findall(
        r"test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored;", log
    )
    named = re.search(r"test " + re.escape(REQUIRED) + r" \.\.\..*?\bok\b", log, re.S)
    failed = "FAILED" in log or "panicked at" in log
    if not named or summary != [("ok", "1", "0", "0")] or failed:
        print(f"Rous compatibility checker: {REQUIRED} must actually run: 1 PASS, 0 FAIL, 0 ignored", file=sys.stderr)
        return 1
    print(f"Rous compatibility checker: MUST {REQUIRED} = pass")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <cargo-test-log>")
    raise SystemExit(main(sys.argv[1]))

#!/usr/bin/env python3
"""Fetch one explicitly pinned Saki Actions artifact; never choose latest or build source."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import zipfile


TARGET = "aarch64-apple-darwin"


def load_pin(path: Path) -> dict:
    pin = json.loads(path.read_text())
    if not isinstance(pin, dict):
        raise ValueError("Saki runtime pin must be a JSON object")
    if pin.get("status") == "pending":
        raise ValueError("Saki runtime pin is pending: PM must record a completed runtime CI artifact")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", str(pin.get("repository", ""))):
        raise ValueError("pin repository must be owner/name")
    for key in ("run_id", "run_attempt", "artifact_id"):
        if type(pin.get(key)) is not int or pin[key] <= 0:
            raise ValueError(f"pin {key} must be a positive integer")
    for key, length in (("source_sha", 40), ("archive_sha256", 64), ("binary_sha256", 64)):
        if not re.fullmatch(rf"[0-9a-f]{{{length}}}", str(pin.get(key, ""))):
            raise ValueError(f"pin {key} must be a complete lowercase hash")
    if pin.get("target") != TARGET or type(pin.get("protocol_version")) is not int or pin["protocol_version"] != 1:
        raise ValueError("pin must select the macOS arm64 runtime protocol v1 artifact")
    if pin.get("artifact_name") != f"saki-runtime-{TARGET}-{pin['source_sha']}-run-{pin['run_id']}-attempt-{pin['run_attempt']}":
        raise ValueError("pin artifact_name must identify its target and source SHA")
    return pin


def validate_remote(pin: dict, run: dict, artifact: dict) -> None:
    expected_run = {
        "id": pin["run_id"], "run_attempt": pin["run_attempt"],
        "head_sha": pin["source_sha"], "status": "completed", "conclusion": "success",
    }
    if any(run.get(key) != value for key, value in expected_run.items()):
        raise ValueError("pinned Saki run/attempt/source must be completed SUCCESS")
    if run.get("repository", {}).get("full_name") != pin["repository"]:
        raise ValueError("Saki workflow repository differs from pin")
    expected_artifact = {
        "id": pin["artifact_id"], "name": pin["artifact_name"], "expired": False,
        "digest": "sha256:" + pin["archive_sha256"],
    }
    if any(artifact.get(key) != value for key, value in expected_artifact.items()):
        raise ValueError("artifact identity/digest/expiry differs from pin")
    origin = artifact.get("workflow_run", {})
    if origin.get("id") != pin["run_id"] or origin.get("head_sha") != pin["source_sha"]:
        raise ValueError("artifact belongs to a different run or source")


def verify_archive(archive: Path, pin: dict) -> tuple[dict, bytes]:
    with archive.open("rb") as handle:
        digest = hashlib.file_digest(handle, "sha256").hexdigest()
    if digest != pin["archive_sha256"]:
        raise ValueError("downloaded archive SHA256 differs from pin")
    with zipfile.ZipFile(archive) as bundle:
        if len(bundle.namelist()) != 3 or set(bundle.namelist()) != {"Saki", "manifest.json", "SHA256SUMS"}:
            raise ValueError("runtime artifact must contain exactly Saki, manifest.json and SHA256SUMS")
        binary = bundle.read("Saki")
        raw_manifest = bundle.read("manifest.json")
        sums = bundle.read("SHA256SUMS").decode().splitlines()
    manifest = json.loads(raw_manifest)
    if not isinstance(manifest, dict):
        raise ValueError("runtime manifest must be a JSON object")
    expected = {
        "schema": 1, "product": "Saki", "dirty": False,
        **{key: pin[key] for key in ("repository", "source_sha", "run_id", "run_attempt", "target", "protocol_version")},
    }
    if any(manifest.get(key) != value for key, value in expected.items()):
        raise ValueError("runtime manifest source/run/target/protocol differs from pin")
    if not re.fullmatch(r"[0-9][0-9A-Za-z.+-]*", str(manifest.get("version", ""))):
        raise ValueError("runtime manifest lacks a valid product version")
    binary_hash = hashlib.sha256(binary).hexdigest()
    if manifest.get("binary") != {"name": "Saki", "sha256": binary_hash} or binary_hash != pin["binary_sha256"]:
        raise ValueError("runtime binary SHA256 differs from manifest or pin")
    expected_sums = {
        f"{binary_hash}  Saki",
        f"{hashlib.sha256(raw_manifest).hexdigest()}  manifest.json",
    }
    if len(sums) != 2 or set(sums) != expected_sums:
        raise ValueError("runtime SHA256SUMS do not describe the downloaded files")
    return manifest, binary


def fetch(pin: dict, output: Path) -> tuple[Path, dict]:
    base = f"repos/{pin['repository']}/actions"

    def api(path: str) -> dict:
        return json.loads(subprocess.check_output(["gh", "api", path], text=True))

    run = api(f"{base}/runs/{pin['run_id']}/attempts/{pin['run_attempt']}")
    artifact = api(f"{base}/artifacts/{pin['artifact_id']}")
    validate_remote(pin, run, artifact)
    with tempfile.TemporaryDirectory(prefix="saki-artifact-") as temporary:
        archive = Path(temporary) / "artifact.zip"
        with archive.open("wb") as handle:
            subprocess.run(["gh", "api", f"{base}/artifacts/{pin['artifact_id']}/zip"], stdout=handle, check=True)
        manifest, binary = verify_archive(archive, pin)
    output.mkdir(parents=True, exist_ok=False)
    runtime = output / "Saki"
    runtime.write_bytes(binary)
    runtime.chmod(0o755)
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    return runtime.resolve(), manifest


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pin", type=Path, default=Path(".github/saki-runtime.json"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--github-env", type=Path)
    args = parser.parse_args()
    try:
        pin = load_pin(args.pin)
        runtime, manifest = fetch(pin, args.output)
        if args.github_env:
            with args.github_env.open("a") as handle:
                for key, value in {
                    "SAKI_TEST_RUNTIME_BIN": runtime, "SAKI_RUNTIME_BIN": runtime,
                    "SAKI_RUNTIME_SOURCE_SHA": pin["source_sha"], "SAKI_RUNTIME_VERSION": manifest["version"],
                }.items():
                    handle.write(f"{key}={value}\n")
        print(json.dumps({"binary": str(runtime), "pin": pin, "manifest": manifest}, sort_keys=True))
    except (OSError, ValueError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
        print(f"Saki artifact fetch failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

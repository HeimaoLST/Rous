"""Check pinned artifact acceptance, tamper rejection and unavailable-run failures."""
import contextlib
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock
import zipfile


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


artifact = load("fetch-saki-runtime")


class ArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.binary = b"Saki runtime artifact fixture\0"
        self.pin = {
            "repository": "HeimaoLST/Saki", "run_id": 42, "run_attempt": 2,
            "source_sha": "a" * 40, "artifact_id": 84,
            "artifact_name": "saki-runtime-aarch64-apple-darwin-" + "a" * 40 + "-run-42-attempt-2",
            "target": "aarch64-apple-darwin", "protocol_version": 1,
            "binary_sha256": hashlib.sha256(self.binary).hexdigest(),
        }
        self.manifest = {
            "schema": 1, "product": "Saki", "version": "0.1.0", "dirty": False,
            **{key: self.pin[key] for key in ("repository", "run_id", "run_attempt", "source_sha", "target", "protocol_version")},
            "binary": {"name": "Saki", "sha256": self.pin["binary_sha256"]},
        }
        self.archive = self.root / "artifact.zip"
        self.write_archive()
        self.run = {
            "id": 42, "run_attempt": 2, "head_sha": self.pin["source_sha"],
            "status": "completed", "conclusion": "success", "repository": {"full_name": "HeimaoLST/Saki"},
        }
        self.metadata = {
            "id": 84, "name": self.pin["artifact_name"], "expired": False,
            "digest": "sha256:" + self.pin["archive_sha256"],
            "workflow_run": {"id": 42, "head_sha": self.pin["source_sha"]},
        }

    def write_archive(self, binary=None, extra=False):
        raw = json.dumps(self.manifest).encode()
        sums = f"{hashlib.sha256(self.binary).hexdigest()}  Saki\n{hashlib.sha256(raw).hexdigest()}  manifest.json\n"
        with zipfile.ZipFile(self.archive, "w") as bundle:
            bundle.writestr("Saki", self.binary if binary is None else binary)
            bundle.writestr("manifest.json", raw)
            bundle.writestr("SHA256SUMS", sums)
            if extra:
                bundle.writestr("../wrong", b"unexpected")
        self.pin["archive_sha256"] = hashlib.sha256(self.archive.read_bytes()).hexdigest()

    def test_valid_pin_fetches_verified_binary_without_source_checkout(self):
        pin_path = self.root / "pin.json"
        pin_path.write_text(json.dumps(self.pin))
        pin = artifact.load_pin(pin_path)

        def github(argv, **kwargs):
            if "/attempts/" in argv[-1]:
                return json.dumps(self.run)
            return json.dumps(self.metadata)

        def download(_argv, stdout, check):
            stdout.write(self.archive.read_bytes())

        with mock.patch.object(artifact.subprocess, "check_output", side_effect=github), mock.patch.object(artifact.subprocess, "run", side_effect=download):
            binary, manifest = artifact.fetch(pin, self.root / "download")
        self.assertEqual(binary.read_bytes(), self.binary)
        self.assertEqual(manifest, self.manifest)
        self.assertTrue(binary.stat().st_mode & 0o111)

    def test_failed_cancelled_pending_or_wrong_attempt_cannot_supply_runtime(self):
        for changes in ({"conclusion": "failure"}, {"conclusion": "cancelled"}, {"status": "in_progress"}, {"run_attempt": 1}, {"head_sha": "b" * 40}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                artifact.validate_remote(self.pin, {**self.run, **changes}, self.metadata)

    def test_wrong_expired_or_different_run_artifact_is_rejected(self):
        for changes in ({"id": 85}, {"expired": True}, {"digest": "sha256:" + "0" * 64}, {"workflow_run": {"id": 43, "head_sha": self.pin["source_sha"]}}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                artifact.validate_remote(self.pin, self.run, {**self.metadata, **changes})

    def test_archive_corruption_fails_before_binary_is_used(self):
        self.archive.write_bytes(self.archive.read_bytes() + b"changed")
        with self.assertRaisesRegex(ValueError, "archive SHA256"):
            artifact.verify_archive(self.archive, self.pin)

    def test_binary_tampering_fails_even_when_archive_hash_is_updated(self):
        self.write_archive(binary=b"different runtime")
        with self.assertRaisesRegex(ValueError, "binary SHA256"):
            artifact.verify_archive(self.archive, self.pin)

    def test_mismatched_source_protocol_or_dirty_manifest_is_rejected(self):
        for key, value in (("source_sha", "b" * 40), ("protocol_version", 2), ("dirty", True)):
            with self.subTest(key=key):
                original = self.manifest[key]
                self.manifest[key] = value
                self.write_archive()
                with self.assertRaises(ValueError):
                    artifact.verify_archive(self.archive, self.pin)
                self.manifest[key] = original

    def test_unexpected_archive_files_are_not_extracted(self):
        self.write_archive(extra=True)
        with self.assertRaisesRegex(ValueError, "exactly"):
            artifact.verify_archive(self.archive, self.pin)

    def test_pending_or_incomplete_pin_fails_without_downloading(self):
        for value in ({"status": "pending"}, {**self.pin, "source_sha": None}, {**self.pin, "run_id": True}):
            path = self.root / "pin.json"
            path.write_text(json.dumps(value))
            with self.subTest(value=value), self.assertRaises(ValueError):
                artifact.load_pin(path)

    def test_cli_reports_fetch_failure_as_nonzero(self):
        path = self.root / "pin.json"
        path.write_text(json.dumps(self.pin))
        with mock.patch("sys.argv", ["fetch", "--pin", str(path), "--output", str(self.root / "out")]), mock.patch.object(artifact, "fetch", side_effect=subprocess.CalledProcessError(1, "gh")), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(artifact.main(), 1)


class RousResultContractTests(unittest.TestCase):
    def test_zero_ignored_missing_or_prior_failure_cannot_become_pass(self):
        checker = load("check-rous-runtime-results")
        good = "test client::tests::real_runtime_contract ... output\nok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 2 filtered out;\n"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "cargo.log"
            def result(text):
                path.write_text(text)
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    return checker.main(str(path))
            self.assertEqual(result(good), 0)
            self.assertEqual(result("test result: ok. 0 passed; 0 failed; 0 ignored;"), 1)
            self.assertEqual(result(good.replace("1 passed", "0 passed").replace("0 ignored", "1 ignored")), 1)
            self.assertEqual(result(good.replace(checker.REQUIRED, "different_test")), 1)
            self.assertEqual(result("FAILED\n" + good), 1)
            self.assertEqual(result(""), 1)


if __name__ == "__main__":
    unittest.main()

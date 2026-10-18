"""Behavioral calibration for active Linux/WSL x86_64 admission, not gate evidence."""
from __future__ import annotations

import copy
import json
import tomllib
import unittest
from unittest.mock import patch

import bench_worker
import check
import completion_evidence


class PlatformPolicyTests(unittest.TestCase):
    def test_linux_gnu_and_musl_are_admitted(self):
        with patch.object(bench_worker.platform, "system", return_value="Linux"), patch.object(
            bench_worker.platform, "machine", return_value="x86_64"
        ):
            self.assertEqual(bench_worker.native_profile(), "release-musl-stripped")
            self.assertEqual(bench_worker.native_profile("x86_64-unknown-linux-gnu"), "release-gnu-native")
            self.assertEqual(bench_worker.native_profile("x86_64-unknown-linux-musl"), "release-musl-stripped")

    def test_out_of_scope_hosts_refuse_before_measurement(self):
        for system, machine in (("Linux", "aarch64"), ("Darwin", "arm64"),
                                ("Darwin", "x86_64"), ("Windows", "AMD64")):
            with self.subTest(system=system, machine=machine), patch.object(
                bench_worker.platform, "system", return_value=system
            ), patch.object(bench_worker.platform, "machine", return_value=machine):
                with self.assertRaisesRegex(ValueError, "Linux/WSL x86_64"):
                    bench_worker.native_profile()

    def test_out_of_scope_targets_never_fall_back_to_musl(self):
        with patch.object(bench_worker.platform, "system", return_value="Linux"), patch.object(
            bench_worker.platform, "machine", return_value="x86_64"
        ):
            for target in ("aarch64-unknown-linux-musl", "aarch64-unknown-linux-gnu",
                           "x86_64-apple-darwin", "aarch64-apple-darwin",
                           "x86_64-pc-windows-msvc", "unknown-gnu"):
                with self.subTest(target=target), self.assertRaisesRegex(ValueError, "target must"):
                    bench_worker.native_profile(target)


class InventoryPolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.inventory = json.loads(check.INVENTORY.read_text())

    def test_each_retained_gate_is_required_even_without_its_evidence_pointers(self):
        for index, boundary in enumerate(self.inventory["boundaries"]):
            for gate in boundary["gates"]:
                with self.subTest(boundary=boundary["id"], gate=gate):
                    inventory = copy.deepcopy(self.inventory)
                    row = inventory["boundaries"][index]
                    row["gates"].remove(gate)
                    for claim in row["claims"]:
                        claim["evidence"] = [entry for entry in claim["evidence"] if entry["gate"] != gate]
                    errors = []
                    check.validate_migration(inventory, errors)
                    self.assertIn(f"{boundary['id']}: retained gates missing: {gate}", errors)

    def test_inventory_platform_scope_cannot_be_missing_or_expanded(self):
        for policy in (None, completion_evidence.PLATFORM_POLICY | {
            "release_targets": ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"]
        }):
            with self.subTest(policy=policy):
                inventory = copy.deepcopy(self.inventory)
                inventory["platform_policy"] = policy
                errors = check.validate_inventory(inventory)
                self.assertIn("active qualification must require Linux/WSL x86_64 GNU and musl only", errors)


class CompletionAdmissionTests(unittest.TestCase):
    def setUp(self):
        version = tomllib.loads((completion_evidence.ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        # These identities isolate early admission branches; no measurement is
        # produced and no synthetic report can pass full qualification.
        self.source = {"fixture": "source"}
        self.methods = {"fixture": "method"}
        self.report = {
            "schema": 1, "release": "0059", "source": self.source, "methods": self.methods,
            "binary_version": version, "binary_sha256": "a" * 64, "binary_bytes": 1,
            "profile": "release-musl-stripped", "platform": {"machine": "x86_64"},
        }
        self.source_patch = patch.object(completion_evidence, "source_identity", return_value=self.source)
        self.method_patch = patch.object(completion_evidence, "method_hashes", return_value=self.methods)
        self.source_patch.start()
        self.method_patch.start()
        self.addCleanup(self.source_patch.stop)
        self.addCleanup(self.method_patch.stop)

    def test_historical_or_development_profile_cannot_qualify(self):
        for profile in ("development", "release-macos-native", "release-windows-native"):
            with self.subTest(profile=profile), self.assertRaisesRegex(SystemExit, "only Linux x86_64"):
                completion_evidence.validate_report(self.report | {"profile": profile})

    def test_arm_report_cannot_qualify_as_linux_x86_64(self):
        with self.assertRaisesRegex(SystemExit, "Linux/WSL x86_64"):
            completion_evidence.validate_report(self.report | {"platform": {"machine": "aarch64"}})

    def test_changed_source_or_method_cannot_reuse_historical_measurements(self):
        for field in ("source", "methods"):
            with self.subTest(field=field), self.assertRaisesRegex(SystemExit, "current source, methods and artifact"):
                completion_evidence.validate_report(self.report | {field: {"fixture": "historical"}})

    def test_missing_current_report_never_falls_back_to_history(self):
        with patch.object(completion_evidence.Path, "read_text", side_effect=FileNotFoundError):
            with self.assertRaises(FileNotFoundError):
                completion_evidence.validate()


if __name__ == "__main__":
    unittest.main()

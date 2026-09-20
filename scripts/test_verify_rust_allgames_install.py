"""Failure-path checks for the static package audit; no binary execution."""

import contextlib
import io
from pathlib import Path
import unittest
from unittest.mock import patch

import verify_rust_allgames_install as verifier


class AuditTests(unittest.TestCase):
    def run_audit(self, *, arch="arm64", missing_export=None, imported=None,
                  missing_file=None, missing_factory=False, wrong_launcher=False):
        runtime = Path("runtime")
        shared = [runtime / "bin" / f"lib{name}.dylib" for name in (
            "source_abi", "rust_engine_bridge", "launcher", "engine", "filesystem_stdio", "shaderapimetal"
        )]
        abi_exports = {
            "_source_host_run_app_system_group", "_source_host_app_group_startup", "_source_host_app_group_shutdown"
        }
        abi_exports.discard(missing_export)

        def output(*args):
            if args[0] == "lipo":
                return arch + "\n"
            if args[1] == "-u":
                return (imported + "\n") if imported else ""
            if args[2].endswith("libsource_abi.dylib"):
                return "\n".join("000 T " + name for name in abi_exports)
            if missing_factory and args[2].endswith("libclient.dylib"):
                return ""
            return "000 T _CreateInterface\n"

        def read_bytes(path):
            if wrong_launcher and path.name == "cargo-launcher":
                return b"wrong launcher"
            return b"binary fixture"

        with patch.object(Path, "glob", return_value=shared), \
                patch.object(Path, "is_file", lambda path: path.name != missing_file), \
                patch.object(Path, "read_bytes", read_bytes), \
                patch.object(verifier, "output", side_effect=output), \
                contextlib.redirect_stdout(io.StringIO()) as log:
            verifier.audit(runtime, ("hl2",), Path("cargo-launcher"))
            return log.getvalue()

    def test_accepts_resolved_rust_import(self):
        self.assertIn("PASS: 1 client/server pairs", self.run_audit(imported="_source_host_app_group_startup"))

    def test_rejects_missing_game_binary(self):
        with self.assertRaisesRegex(ValueError, "missing installed artifact"):
            self.run_audit(missing_file="libserver.dylib")

    def test_rejects_non_arm64_binary(self):
        with self.assertRaisesRegex(ValueError, "not a native arm64-only"):
            self.run_audit(arch="x86_64")

    def test_rejects_stale_lifecycle_abi(self):
        with self.assertRaisesRegex(ValueError, "missing lifecycle ABI export"):
            self.run_audit(missing_export="_source_host_app_group_shutdown")

    def test_rejects_unresolved_rust_import(self):
        with self.assertRaisesRegex(ValueError, "unprovided Rust imports"):
            self.run_audit(imported="_source_missing_from_install")

    def test_rejects_missing_game_factory(self):
        with self.assertRaisesRegex(ValueError, "missing game interface factory"):
            self.run_audit(missing_factory=True)

    def test_rejects_wrong_launcher(self):
        with self.assertRaisesRegex(ValueError, "installed launcher differs"):
            self.run_audit(wrong_launcher=True)


if __name__ == "__main__":
    unittest.main()

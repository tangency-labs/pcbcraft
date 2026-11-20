"""Exercise the signing-keychain handoff without Apple credentials or a macOS runner."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


IMPORT_CERT = Path(__file__).resolve().parents[1] / "packaging/macos/import-cert.sh"
BASH = shutil.which("bash")
IDENTITY = "0123456789ABCDEF0123456789ABCDEF01234567"


@unittest.skipUnless(os.name == "posix" and BASH, "requires a POSIX Bash environment")
class ImportCertTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory(prefix="pcbcraft-signing-test-")
        self.addCleanup(scratch.cleanup)
        self.directory = Path(scratch.name)
        security = self.directory / "security"
        security.write_text(
            f"""#!/bin/sh
case "$1" in
    create-keychain) : > "$4" ;;
    delete-keychain) rm -f "$2" ;;
    find-identity) printf '  1) %s "Developer ID Application: Test"\\n' '{IDENTITY}' ;;
    set-keychain-settings|unlock-keychain|import|set-key-partition-list|list-keychains) ;;
    *) exit 2 ;;
esac
""",
            encoding="utf-8",
        )
        security.chmod(0o700)

    def run_import(self, github_env):
        return subprocess.run(
            [BASH, str(IMPORT_CERT)],
            env={
                "PATH": os.pathsep.join((str(self.directory), os.defpath)),
                "LC_ALL": "C",
                "RUNNER_TEMP": str(self.directory),
                "DIST": str(self.directory / "dist"),
                "GITHUB_ENV": str(github_env),
                "KEYCHAIN_PASSWORD": "test-password",
                "APPLE_CERTIFICATE": "dGVzdA==",
            },
            capture_output=True,
            text=True,
            timeout=10,
        )

    def assert_failed_handoff_cleans_up(self, github_env, error):
        result = self.run_import(github_env)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(error, result.stderr)
        self.assertFalse(list(self.directory.glob("pcbcraft-signing-*")))

    def test_unopenable_output_removes_keychain(self):
        self.assert_failed_handoff_cleans_up(self.directory, str(self.directory))

    @unittest.skipUnless(Path("/dev/full").exists(), "requires /dev/full")
    def test_failed_write_removes_keychain(self):
        self.assert_failed_handoff_cleans_up(Path("/dev/full"), "write error")

    def test_successful_handoff_retains_keychain(self):
        github_env = self.directory / "github-env"
        github_env.write_text("EXISTING=value\n", encoding="utf-8")
        result = self.run_import(github_env)
        self.assertEqual(result.returncode, 0, result.stderr)
        keychain, = self.directory.glob("pcbcraft-signing-*.keychain-db")
        self.assertEqual(
            github_env.read_text(encoding="utf-8").splitlines(),
            ["EXISTING=value", f"MACOS_SIGN_IDENTITY={IDENTITY}", f"MACOS_KEYCHAIN={keychain}"],
        )
        self.assertFalse(list(self.directory.glob("pcbcraft-signing-*.p12")))

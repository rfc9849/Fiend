import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("rollback.sh")


class RollbackTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="fiend-rollback-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.current = self.root / "fiend-core"
        self.previous = self.root / "fiend-core.prev"
        self.failed = self.root / "fiend-core.failed"
        self.current.write_bytes(b"new binary")
        commands = self.root / "commands"
        commands.mkdir()
        runuser = commands / "runuser"
        runuser.write_text('#!/bin/sh\nshift 3\nexec "$@"\n')
        runuser.chmod(0o755)
        self.environment = dict(os.environ, FIEND_BIN=str(self.root), FIEND_CONFIG="test-node.toml", PATH=str(commands) + os.pathsep + os.environ["PATH"])

    def run_rollback(self):
        return subprocess.run(["sh", str(SCRIPT)], env=self.environment, capture_output=True)

    def set_previous(self, succeeds=True):
        code = '#!/bin/sh\n[ "$1" = check ] && [ "$2" = test-node.toml ] || exit 2\n'
        code += "exit 0\n" if succeeds else "exit 1\n"
        self.previous.write_text(code)
        self.previous.chmod(0o755)
        return self.previous.read_bytes()

    def test_restore_consumes_previous_and_keeps_failed_binary(self):
        previous = self.set_previous()
        result = self.run_rollback()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.current.read_bytes(), previous)
        self.assertEqual(self.failed.read_bytes(), b"new binary")
        self.assertFalse(self.previous.exists())
        self.assertNotEqual(self.run_rollback().returncode, 0)
        self.assertEqual(self.current.read_bytes(), previous)
        self.assertEqual(self.failed.read_bytes(), b"new binary")

    def test_failed_check_keeps_both_binaries(self):
        previous = self.set_previous(succeeds=False)
        self.assertNotEqual(self.run_rollback().returncode, 0)
        self.assertEqual(self.current.read_bytes(), b"new binary")
        self.assertEqual(self.previous.read_bytes(), previous)
        self.assertFalse(self.failed.exists())

    def test_missing_previous_keeps_current_binary(self):
        self.assertNotEqual(self.run_rollback().returncode, 0)
        self.assertEqual(self.current.read_bytes(), b"new binary")
        self.assertFalse(self.failed.exists())


if __name__ == "__main__":
    unittest.main()

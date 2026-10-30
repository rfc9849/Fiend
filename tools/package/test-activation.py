import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ACTIVATE = Path(__file__).with_name("activate-update.sh")
ENTRY = Path(__file__).with_name("node-entry.sh")


class ActivationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="fiend-activation-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.bin = self.root / "bin"
        self.updates = self.root / "updates"
        self.commands = self.root / "commands"
        for path in [self.bin, self.updates, self.commands]:
            path.mkdir()
        self.current = self.bin / "fiend-core"
        self.current.write_bytes(b"old binary")
        self.fake_command("runuser", '#!/bin/sh\nshift 3\nexec "$@"\n')
        self.fake_command("sha256sum", f'#!{sys.executable}\nimport hashlib,sys\nprint(hashlib.sha256(open(sys.argv[1], "rb").read()).hexdigest(), sys.argv[1])\n')
        self.env = dict(os.environ, FIEND_BIN=str(self.bin), FIEND_CONFIG="node.toml", FIEND_STATE="state.json", FIEND_UPDATES=str(self.updates), PATH=str(self.commands) + os.pathsep + os.environ["PATH"])

    def fake_command(self, name, source):
        path = self.commands / name
        path.write_text(source)
        path.chmod(0o755)

    def stage(self, succeeds=True):
        source = b'#!/bin/sh\n[ "$1" = check ] && [ "$2" = node.toml ] && [ "$3" = state.json ] || exit 2\n'
        source += b"exit 0\n" if succeeds else b"exit 1\n"
        (self.updates / "core.pending").write_bytes(source)
        (self.updates / "core.pending.sha256").write_text(hashlib.sha256(source).hexdigest() + "\n")
        return source

    def activate(self):
        return subprocess.run(["sh", str(ACTIVATE)], env=self.env, capture_output=True)

    def test_checked_candidate_replaces_current_and_keeps_previous(self):
        source = self.stage()
        result = self.activate()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.current.read_bytes(), source)
        self.assertEqual((self.bin / "fiend-core.prev").read_bytes(), b"old binary")
        self.assertFalse((self.updates / "core.pending.sha256").exists())
        self.assertEqual(self.activate().returncode, 0)
        self.assertEqual((self.bin / "fiend-core.prev").read_bytes(), b"old binary")

    def test_bad_hash_keeps_current(self):
        self.stage()
        (self.updates / "core.pending").write_bytes(b"damaged")
        self.assertEqual(self.activate().returncode, 0)
        self.assertEqual(self.current.read_bytes(), b"old binary")
        self.assertTrue((self.updates / "core.pending.sha256.rejected").exists())
        self.assertFalse((self.bin / "fiend-core.new").exists())

    def test_failed_self_check_keeps_current(self):
        self.stage(succeeds=False)
        self.assertEqual(self.activate().returncode, 0)
        self.assertEqual(self.current.read_bytes(), b"old binary")
        self.assertFalse((self.bin / "fiend-core.prev").exists())

    def test_entry_supports_managed_and_legacy_binaries(self):
        for managed in [True, False]:
            command = "node" if managed else "server"
            source = f'#!/bin/sh\nif [ "$1" = --help ]; then echo "  {command} <config>"; exit 0; fi\nprintf "%s:%s" "$1" "$2"\n'
            self.current.write_text(source)
            self.current.chmod(0o755)
            result = subprocess.run(["sh", str(ENTRY)], env=self.env, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.decode(), command + ":node.toml")


if __name__ == "__main__":
    unittest.main()

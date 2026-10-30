import argparse
import json
import os
from pathlib import Path
import pwd
import secrets
import select
import shutil
import subprocess
import sys
import time
import unittest


OPTIONS = None


@unittest.skipUnless(sys.platform == "linux" and os.geteuid() == 0, "needs root on Linux with systemd")
class SystemdTests(unittest.TestCase):
    def setUp(self):
        self.suffix = secrets.token_hex(6)
        self.unit = "fiend-verify-" + self.suffix
        self.rollback = self.unit + "-rollback"
        self.base = Path("/var/lib/fiend") / self.unit
        self.bin = Path("/opt/fiend") / self.unit / "bin"
        self.uid = pwd.getpwnam("fiend").pw_uid
        self.gid = pwd.getpwnam("fiend").pw_gid
        self.base.mkdir(mode=0o700)
        self.addCleanup(self.cleanup)
        os.chown(self.base, self.uid, self.gid)
        self.bin.mkdir(parents=True)
        shutil.copy2(OPTIONS.core, self.bin / "fiend-core")
        shutil.copy2(OPTIONS.control, self.base / "node-control")
        self.socket = self.base / "control.sock"
        self.config = self.base / "node.toml"
        self.state = self.base / "state.json"
        self.updates = self.base / "updates"
        self.updates.mkdir(mode=0o700)
        os.chown(self.updates, self.uid, self.gid)
        secret = self.base / "control.secret"
        secret.write_text("93" * 32)
        self.config.write_text(f'listen = "127.0.0.1:0"\nsecret = "' + "91" * 32 + f'"\norigin_host = "127.0.0.1"\norigin_port = 1\n[origin]\nrecord_sizes = [35, 3400, 286, 74]\n[node]\nstate_file = "{self.state}"\ncontrol_secret_file = "{secret}"\ncontrol_socket = "{self.socket}"\nupdates_dir = "{self.updates}"\n')
        for path in [secret, self.config]:
            path.chmod(0o600)
            os.chown(path, self.uid, self.gid)
        environment = f'Environment=FIEND_BIN={self.bin} FIEND_CONFIG={self.config} FIEND_STATE={self.state} FIEND_UPDATES={self.updates}\n'
        service = f'[Unit]\nOnFailure={self.rollback}.service\nStartLimitIntervalSec=60\nStartLimitBurst=3\n[Service]\nUser=fiend\nGroup=fiend\n{environment}ExecStartPre=+/opt/fiend/bin/fiend-activate-update\nExecStartPre={self.bin}/fiend-core check {self.config}\nExecStart=/opt/fiend/bin/fiend-node\nRestart=always\nRestartSec=2\nNoNewPrivileges=yes\nProtectSystem=strict\nProtectHome=yes\nPrivateTmp=yes\nReadWritePaths={self.base}\nRestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX\nUMask=0077\n'
        rollback = f'[Unit]\nConditionPathExists={self.bin}/fiend-core.prev\n[Service]\nType=oneshot\n{environment}ExecStart=/opt/fiend/bin/fiend-rollback\nExecStartPost=/usr/bin/systemctl reset-failed {self.unit}.service\nExecStartPost=/usr/bin/systemctl --no-block start {self.unit}.service\nNoNewPrivileges=yes\nProtectSystem=strict\nProtectHome=yes\nPrivateTmp=yes\nReadWritePaths={self.bin}\nRestrictAddressFamilies=AF_UNIX\n'
        for name, content in [(self.unit, service), (self.rollback, rollback)]:
            Path(f"/run/systemd/system/{name}.service").write_text(content)
        subprocess.run(["systemctl", "daemon-reload"], check=True)
        self.journal = subprocess.Popen(["journalctl", "--follow", "--no-pager", "--output=cat", "--since", "1 minute ago", "--unit", self.unit, "--unit", self.rollback], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.lines = []
        self.buffer = b""

    def cleanup(self):
        subprocess.run(["systemctl", "stop", self.unit, self.rollback], capture_output=True)
        subprocess.run(["systemctl", "reset-failed", self.unit, self.rollback], capture_output=True)
        if hasattr(self, "journal"):
            self.journal.terminate()
            self.journal.wait(timeout=5)
            self.journal.stdout.close()
            self.journal.stderr.close()
        for name in [self.unit, self.rollback]:
            Path(f"/run/systemd/system/{name}.service").unlink(missing_ok=True)
        subprocess.run(["systemctl", "daemon-reload"], check=True)
        shutil.rmtree(self.base, ignore_errors=True)
        shutil.rmtree(self.bin.parent, ignore_errors=True)

    def wait_for_log(self, text):
        deadline = time.monotonic() + 30
        while True:
            while b"\n" in self.buffer:
                line, self.buffer = self.buffer.split(b"\n", 1)
                line = line.decode(errors="replace")
                self.lines.append(line)
                if text in line:
                    return
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([self.journal.stdout], [], [], remaining)[0]:
                self.fail(f"Missing log {text}: " + "\n".join(self.lines))
            data = os.read(self.journal.stdout.fileno(), 65536)
            if not data:
                self.fail("journal stream ended")
            self.buffer += data

    def control(self, command, binary=None):
        args = ["runuser", "-u", "fiend", "--", str(self.base / "node-control"), str(self.socket), command]
        if binary:
            args.append(str(binary))
        result = subprocess.run(args, check=True, capture_output=True, text=True, timeout=30)
        return json.loads(result.stdout)

    def test_update_and_three_start_failure_restore_previous_binary(self):
        subprocess.run(["systemctl", "start", self.unit], check=True)
        self.wait_for_log("node started")
        self.assertEqual(self.control("health")["state_version"], 1)
        self.assertTrue(self.control("update", OPTIONS.core)["restart_required"])
        self.wait_for_log("node started")
        self.assertEqual(self.control("health")["state_version"], 1)
        self.assertTrue((self.bin / "fiend-core.prev").exists())
        self.assertFalse((self.updates / "core.pending.sha256").exists())
        subprocess.run(["systemctl", "reset-failed", self.unit], check=True)
        candidate = self.base / "bad-core"
        marker = self.base / "self-check.uid"
        candidate.write_text(f'#!/bin/sh\nif [ "$1" = check ]; then id -u > {marker}; exit 0; fi\nif [ "$1" = --help ]; then echo "  node <config>"; exit 0; fi\nexit 1\n')
        candidate.chmod(0o700)
        os.chown(candidate, self.uid, self.gid)
        self.assertTrue(self.control("update", candidate)["restart_required"])
        self.wait_for_log("node started")
        self.assertEqual(self.control("health")["state_version"], 1)
        self.assertEqual(marker.read_text().strip(), str(self.uid))
        self.assertFalse((self.bin / "fiend-core.prev").exists())
        self.assertTrue((self.bin / "fiend-core.failed").exists())
        self.assertEqual((self.bin / "fiend-core").read_bytes(), Path(OPTIONS.core).read_bytes())


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--core", required=True)
    parser.add_argument("--control", required=True)
    OPTIONS = parser.parse_args()
    unittest.main(argv=[sys.argv[0]])

#!/usr/bin/env python3
"""Run the Dockerfile's APT commands against controlled package-index failures."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


def apt_instructions():
    commands = []
    lines = iter((ROOT / "Dockerfile").read_text().splitlines())
    for line in lines:
        if not line.startswith("RUN "):
            continue
        command = line[4:]
        while command.endswith("\\"):
            command = command[:-1] + next(lines).lstrip()
        if "apt-get" in command:
            commands.append(command)
    return commands


class DockerAptTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="ivnc-docker-apt-test-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.apt = self.root / "etc/apt"
        (self.apt / "sources.list.d").mkdir(parents=True)
        self.lists = self.root / "var/lib/apt/lists"
        self.lists.mkdir(parents=True)
        self.resolv = self.root / "resolv.conf"
        self.resolv.write_text("nameserver 192.0.2.53\n")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.log = self.root / "commands.jsonl"
        for executable in ("apt-get", "curl"):
            mock = self.bin / executable
            mock.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
command = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with pathlib.Path(os.environ['APT_TEST_LOG']).open('a') as output:
    output.write(json.dumps([command, *args]) + '\\n')
if command == 'curl':
    sys.exit(41)  # Stop before downloads or runtime binary checks.
if 'update' in args and os.environ.get('APT_TEST_INDEX_FAILURE') == '1':
    # APT normally permits missing indexes; strict mode must reject them.
    sys.exit(17 if '--error-on=any' in args else 0)
sys.exit(0)
''')
            mock.chmod(0o755)

    def run_stage(self, stage, fail_indexes=False):
        commands = apt_instructions()
        self.assertEqual(len(commands), 2, "expected builder and runtime APT instructions")
        command = commands[stage].replace("/etc/apt", str(self.apt))
        command = command.replace("/var/lib/apt/lists", str(self.lists))
        command = command.replace("/etc/resolv.conf", str(self.resolv))
        return subprocess.run(
            ["/bin/sh", "-c", command], capture_output=True, text=True, timeout=10,
            env=dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}",
                     APT_TEST_LOG=str(self.log), APT_TEST_INDEX_FAILURE=str(int(fail_indexes))),
        )

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def test_builder_stops_before_install_when_an_index_is_missing(self):
        result = self.run_stage(0, fail_indexes=True)
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertEqual(len(self.calls()), 1)
        self.assertIn("update", self.calls()[0])

    def test_runtime_stops_before_install_and_downloads_when_an_index_is_missing(self):
        result = self.run_stage(1, fail_indexes=True)
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertEqual(len(self.calls()), 1)
        self.assertIn("update", self.calls()[0])

    def test_builder_retries_update_and_install_then_cleans_indexes(self):
        marker = self.lists / "test-index"
        marker.touch()
        result = self.run_stage(0)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls()), 2)
        for call in self.calls():
            self.assertIn("Acquire::Retries=5", call)
        self.assertIn("install", self.calls()[1])
        self.assertFalse(marker.exists())

    def test_noble_deb822_sources_keep_components_keys_and_security_repository(self):
        source = self.apt / "sources.list.d/ubuntu.sources"
        contents = (
            "Types: deb\nURIs: https://archive.ubuntu.com/ubuntu\n"
            "Suites: noble noble-updates noble-backports\n"
            "Components: main restricted universe multiverse\n"
            "Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n\n"
            "Types: deb\nURIs: http://security.ubuntu.com/ubuntu\n"
            "Suites: noble-security\nComponents: main restricted universe multiverse\n"
            "Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n"
        )
        source.write_text(contents)
        result = self.run_stage(1)
        self.assertEqual(result.returncode, 41, result.stderr)
        self.assertEqual(source.read_text(), contents.replace("https://archive.ubuntu.com/ubuntu",
                                                            "https://azure.archive.ubuntu.com/ubuntu"))
        calls = self.calls()
        self.assertEqual(len(calls), 3)
        for call in calls[:2]:
            self.assertIn("Acquire::Retries=5", call)
        self.assertIn("install", calls[1])
        self.assertEqual(calls[2][0], "curl")

    def test_legacy_sources_and_third_party_lists_are_preserved(self):
        source = self.apt / "sources.list"
        contents = ("deb http://archive.ubuntu.com/ubuntu jammy main universe\n"
                    "deb http://security.ubuntu.com/ubuntu jammy-security main universe\n")
        source.write_text(contents)
        third_party = self.apt / "sources.list.d/third party.list"
        third_party.write_text("deb https://packages.example.com/ubuntu jammy main\n")
        result = self.run_stage(1)
        self.assertEqual(result.returncode, 41, result.stderr)
        self.assertEqual(source.read_text(), contents.replace("http://archive.ubuntu.com/ubuntu",
                                                            "http://azure.archive.ubuntu.com/ubuntu"))
        self.assertEqual(third_party.read_text(), "deb https://packages.example.com/ubuntu jammy main\n")


if __name__ == "__main__":
    unittest.main()

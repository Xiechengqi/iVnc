#!/usr/bin/env python3
"""Exercise Docker argument forwarding and shutdown without Docker or host services."""
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import unittest


ROOT = Path(__file__).resolve().parents[1]


class DockerEntrypointTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ivnc-entrypoint-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.env = dict(os.environ, XDG_RUNTIME_DIR=str(self.root / "runtime"),
                        ENTRYPOINT_TEST_DIR=str(self.root), PATH=f"{self.bin}:{os.environ['PATH']}")
        self.process = None
        self.addCleanup(self.stop)
        self.executable("pulseaudio", "#!/bin/sh\nexit 0\n")
        self.ivnc = self.executable("ivnc", """#!/usr/bin/env python3
import json, os, pathlib, signal, sys
root = pathlib.Path(os.environ['ENTRYPOINT_TEST_DIR'])
(root / 'ivnc.json').write_text(json.dumps({'args': sys.argv[1:], 'pid': os.getpid()}))
if 'ENTRYPOINT_TEST_EXIT' in os.environ: sys.exit(int(os.environ['ENTRYPOINT_TEST_EXIT']))
def stop(*_):
    (root / 'ivnc-stopped').touch()
    sys.exit(0)
signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
while True: signal.pause()
""")
        self.foreground = self.executable("foreground", """#!/usr/bin/env python3
import json, os, pathlib, signal, sys
root = pathlib.Path(os.environ['ENTRYPOINT_TEST_DIR'])
(root / 'command.json').write_text(json.dumps({'args': sys.argv[1:], 'pid': os.getpid()}))
if 'ENTRYPOINT_TEST_COMMAND_EXIT' in os.environ: sys.exit(int(os.environ['ENTRYPOINT_TEST_COMMAND_EXIT']))
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
while True: signal.pause()
""")
        # Substitute only the binary location in an isolated copy; never replace
        # /usr/local/bin/ivnc or execute the host's PulseAudio/Docker services.
        source = (ROOT / "docker-entrypoint.sh").read_text()
        self.entrypoint = self.root / "entrypoint.sh"
        self.entrypoint.write_text(source.replace("/usr/local/bin/ivnc", str(self.ivnc)))

    def executable(self, name, content):
        path = self.bin / name
        path.write_text(content)
        path.chmod(0o755)
        return path

    def start(self, args=(), extra_env=None):
        with (self.root / "entrypoint.log").open("w") as output:
            self.process = subprocess.Popen(
                ["bash", str(self.entrypoint), *args], env=dict(self.env, **(extra_env or {})),
                stdout=output, stderr=output, start_new_session=True,
            )
        return self.wait_json("ivnc.json")

    def wait_json(self, name):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            try:
                return json.loads((self.root / name).read_text())
            except (FileNotFoundError, json.JSONDecodeError):
                time.sleep(0.02)
        self.fail(f"{name} was not written: {(self.root / 'entrypoint.log').read_text()}")

    def stop(self):
        if self.process is None:
            return
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=6)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait()
                self.fail("entrypoint failed to stop its children")
        for name in ("ivnc.json", "command.json"):
            if (self.root / name).exists():
                pid = self.wait_json(name)["pid"]
                self.assertFalse(Path(f"/proc/{pid}").exists(), f"child {pid} leaked")

    def test_image_entrypoint_accepts_options_after_image_name(self):
        lines = (ROOT / "Dockerfile").read_text().splitlines()
        entrypoint = next(json.loads(line[11:]) for line in lines if line.startswith("ENTRYPOINT "))
        command = next(json.loads(line[4:]) for line in lines if line.startswith("CMD ["))
        self.assertEqual(entrypoint, ["/usr/local/bin/docker-entrypoint.sh"])
        self.assertEqual(command, ["sleep", "infinity"])

    def test_proxy_ip_reaches_ivnc_and_term_stops_it_without_a_foreground_command(self):
        args = ["--proxy-ip", "203.0.113.10", "--http-port", "9000"]
        self.assertEqual(self.start(args)["args"], args)
        self.stop()
        self.assertEqual(self.process.returncode, 143)
        self.assertTrue((self.root / "ivnc-stopped").exists())

    def test_run_subcommand_and_ipv6_inline_option_reach_ivnc(self):
        args = ["run", "--proxy-ip=2001:db8::10"]
        self.assertEqual(self.start(args)["args"], args)

    def test_no_arguments_uses_ivnc_defaults(self):
        self.assertEqual(self.start()["args"], [])
        self.assertEqual((self.root / "runtime").stat().st_mode & 0o777, 0o700)

    def test_existing_public_runtime_is_rejected_without_chmod(self):
        runtime = self.root / "runtime"
        runtime.mkdir(mode=0o755)
        result = subprocess.run(["bash", str(self.entrypoint)], env=self.env,
                                capture_output=True, text=True, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("mode 0700", result.stderr)
        self.assertEqual(runtime.stat().st_mode & 0o777, 0o755)
        self.assertFalse((self.root / "ivnc.json").exists())

    def test_symlink_runtime_is_rejected_without_changing_its_target(self):
        target = self.root / "target"
        target.mkdir(mode=0o700)
        (self.root / "runtime").symlink_to(target, target_is_directory=True)
        result = subprocess.run(["bash", str(self.entrypoint)], env=self.env,
                                capture_output=True, text=True, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("real directory", result.stderr)
        self.assertEqual(target.stat().st_mode & 0o777, 0o700)
        self.assertFalse((self.root / "ivnc.json").exists())

    def test_legacy_foreground_command_preserves_its_arguments(self):
        args = [str(self.foreground), "argument with spaces", "--proxy-ip", "203.0.113.10"]
        self.assertEqual(self.start(args)["args"], [])
        self.assertEqual(self.wait_json("command.json")["args"], args[1:])

    def test_separator_combines_ivnc_options_and_foreground_command(self):
        args = ["--config", "path with spaces.toml", "--proxy-ip", "203.0.113.10",
                "--proxy-autostart", "false"]
        self.assertEqual(self.start([*args, "--", str(self.foreground), "hello world"])["args"], args)
        self.assertEqual(self.wait_json("command.json")["args"], ["hello world"])

    def test_failed_ivnc_start_does_not_launch_a_foreground_command(self):
        self.start(["--proxy-ip", "invalid", "--", str(self.foreground)],
                   {"ENTRYPOINT_TEST_EXIT": "2"})
        self.assertNotEqual(self.process.wait(timeout=6), 0)
        self.assertFalse((self.root / "command.json").exists())

    def test_foreground_exit_preserves_status_and_stops_ivnc(self):
        self.start([str(self.foreground)], {"ENTRYPOINT_TEST_COMMAND_EXIT": "7"})
        self.assertEqual(self.process.wait(timeout=8), 7)
        self.assertTrue((self.root / "ivnc-stopped").exists())

    def test_docker_run_helper_forwards_arguments_after_the_image(self):
        self.executable("docker", """#!/usr/bin/env python3
import json, os, pathlib, sys
if sys.argv[1:3] == ['container', 'inspect']: sys.exit(1)
pathlib.Path(os.environ['ENTRYPOINT_TEST_DIR'], 'docker.json').write_text(json.dumps(sys.argv[1:]))
""")
        source = (ROOT / "docker-run.sh").read_text()
        self.assertIn("if [[ ! -c /dev/net/tun ]]; then", source)
        helper = self.root / "docker-run.sh"
        helper.write_text(source.replace("if [[ ! -c /dev/net/tun ]]; then", "if false; then"))
        args = ["--proxy-ip", "203.0.113.10", "--proxy-autostart", "true"]
        subprocess.run(["bash", str(helper), *args], env=dict(self.env, IMAGE="example/ivnc:test"),
                       check=True, timeout=5)
        forwarded = json.loads((self.root / "docker.json").read_text())
        self.assertEqual(forwarded[-(len(args) + 1):], ["example/ivnc:test", *args])
        self.assertIn("NET_ADMIN", forwarded)
        self.assertIn("/dev/net/tun", forwarded)


if __name__ == "__main__":
    unittest.main()

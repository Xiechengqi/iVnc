#!/usr/bin/env python3
"""Exercise resolver selection and the workflow's fail-closed build handoff."""
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[1]


class ContainerDnsTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="ivnc-ci-dns-test-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.workspace = self.root / "workspace with spaces"
        (self.workspace / "scripts").mkdir(parents=True)
        self.helper = self.workspace / "scripts/ci_container_dns.sh"
        self.helper.write_text((ROOT / "scripts/ci_container_dns.sh").read_text())
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.calls_file = self.root / "docker.jsonl"
        self.output = self.root / "runner temp/ivnc-build-resolv.conf"
        self.env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}",
                        DNS_TEST_CALLS=str(self.calls_file), RUNNER_TEMP=str(self.output.parent))
        docker = self.bin / "docker"
        docker.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
args = sys.argv[1:]
entry = {'args': args, 'kind': args[0]}
status = int(os.environ.get('DNS_TEST_PULL_STATUS', '0'))
if args[0] == 'run':
    entry['kind'] = 'build' if args[-2:] == ['/bin/bash', '/docker-build.sh'] else 'probe'
    mount = args[args.index('--mount') + 1]
    source = next(part[4:] for part in mount.split(',') if part.startswith('src='))
    entry['config'] = pathlib.Path(source).read_text()
    addresses = [line.split()[1] for line in entry['config'].splitlines() if line.startswith('nameserver ')]
    status = int(json.loads(os.environ.get('DNS_TEST_RESULTS', '{}')).get(addresses[0], os.environ.get('DNS_TEST_DEFAULT_STATUS', '1')))
with pathlib.Path(os.environ['DNS_TEST_CALLS']).open('a') as output:
    output.write(json.dumps(entry) + '\\n')
sys.exit(status)
''')
        docker.chmod(0o755)

    def resolver(self, name, address):
        file = self.root / name
        file.write_text(f"# runner configuration\nnameserver {address}\nsearch private.example\n")
        return file

    def run_helper(self, sources, results=None, **environment):
        result = subprocess.run(
            ["bash", str(self.helper), "ubuntu:22.04", "linux/amd64", str(self.output), *map(str, sources)],
            env=dict(self.env, DNS_TEST_RESULTS=json.dumps(results or {}), **environment),
            capture_output=True, text=True, timeout=10,
        )
        self.assertFalse(list(self.output.parent.glob("ivnc-dns.*")), "probe files leaked")
        return result

    def calls(self, kind):
        return [call for line in self.calls_file.read_text().splitlines()
                if (call := json.loads(line))["kind"] == kind]

    def test_working_runner_stub_is_preserved_and_selected_before_public_dns(self):
        source = self.resolver("runner.conf", "127.0.0.53")
        result = self.run_helper([source], {"127.0.0.53": 0})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls("probe")), 1)
        config = self.output.read_text()
        self.assertIn("nameserver 127.0.0.53\n", config)
        self.assertNotIn("search", config)
        self.assertIn("options timeout:2 attempts:2", config)

    def test_broken_stub_falls_back_to_working_runner_uplink(self):
        sources = [self.resolver("stub.conf", "203.0.113.1"), self.resolver("uplink.conf", "192.0.2.53")]
        result = self.run_helper(sources, {"192.0.2.53": 0})
        self.assertEqual(result.returncode, 0, result.stderr)
        probes = self.calls("probe")
        self.assertEqual(len(probes), 2)
        self.assertEqual(self.output.read_text(), probes[-1]["config"])
        self.assertIn("192.0.2.53", self.output.read_text())

    def test_timed_out_runner_and_blocked_public_dns_try_the_next_resolver(self):
        result = self.run_helper([self.resolver("runner.conf", "203.0.113.1")],
                                 {"203.0.113.1": 124, "8.8.8.8": 0})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls("probe")), 3)
        self.assertIn("8.8.8.8", self.output.read_text())

    def test_unreadable_and_empty_runner_sources_are_skipped(self):
        empty = self.root / "empty.conf"
        empty.write_text("# no resolver\n")
        result = self.run_helper([self.root / "missing.conf", empty], {"1.1.1.1": 0})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls("probe")), 1)
        self.assertIn("1.1.1.1", self.output.read_text())

    def test_all_resolvers_failing_produces_no_output_configuration(self):
        result = self.run_helper([self.resolver("runner.conf", "203.0.113.1")])
        self.assertEqual(result.returncode, 1)
        self.assertFalse(self.output.exists())
        self.assertIn("APT has not been started", result.stderr)

    def test_image_pull_failure_stops_before_any_probe(self):
        result = self.run_helper([], DNS_TEST_PULL_STATUS="17")
        self.assertEqual(result.returncode, 17)
        self.assertEqual(self.calls("probe"), [])
        self.assertFalse(self.output.exists())

    def test_runtime_failure_is_not_misreported_as_a_dns_failure(self):
        result = self.run_helper([self.resolver("runner.conf", "203.0.113.1")], {"203.0.113.1": 125})
        self.assertEqual(result.returncode, 125)
        self.assertEqual(len(self.calls("probe")), 1)
        self.assertIn("container could not run", result.stderr)

    def run_workflow_build(self, status):
        workflow = (ROOT / ".github/workflows/build-release.yml").read_text()
        run = workflow.split("      - name: Build binary in Docker container\n        run: |\n", 1)[1]
        run = textwrap.dedent(run.split("\n      - name:", 1)[0])
        values = {"matrix.docker_platform": "linux/amd64", "matrix.arch": "amd64",
                  "vars.IVNC_MIAO_URL": "", "steps.build-meta.outputs.commit": "test-sha",
                  "steps.build-meta.outputs.message": "DNS regression fixture"}
        run = re.sub(r"\$\{\{\s*(.*?)\s*\}\}", lambda match: values[match[1]], run)
        return subprocess.run(["bash", "-e", "-c", run], cwd=self.workspace,
                              env=dict(self.env, DNS_TEST_DEFAULT_STATUS=str(status)),
                              capture_output=True, text=True, timeout=10)

    def test_workflow_build_uses_the_exact_configuration_that_passed_probes(self):
        result = self.run_workflow_build(0)
        self.assertEqual(result.returncode, 0, result.stderr)
        builds = self.calls("build")
        self.assertEqual(len(builds), 1)
        probe = self.calls("probe")[-1]
        self.assertEqual(builds[0]["config"], probe["config"])
        mount = builds[0]["args"][builds[0]["args"].index("--mount") + 1]
        self.assertEqual(mount, f"type=bind,src={self.output},dst=/etc/resolv.conf,readonly")
        self.assertEqual(builds[0]["args"][builds[0]["args"].index("--network") + 1], "host")

    def test_workflow_never_starts_build_when_container_dns_fails(self):
        result = self.run_workflow_build(1)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.calls("build"), [])
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()

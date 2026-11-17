#!/usr/bin/env python3
"""Local regression: launch the real manager with an inherited controlling PTY.

Run after building: IVNC_BIN=target/debug/ivnc python3 scripts/app_session_smoke.py
Uses an isolated HOME, runtime, profile and ports; never touches a live instance.
"""

import fcntl
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import termios
import time
import urllib.request


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait(label, predicate, timeout=25):
    until = time.monotonic() + timeout
    last = None
    while time.monotonic() < until:
        try:
            value = predicate()
            if value:
                return value
        except (OSError, ValueError) as error:
            last = error
        time.sleep(0.05)
    raise AssertionError(f"Timed out: {label}: {last}")


def proc(pid):
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return dict(zip(("state", "ppid", "pgid", "sid", "tty", "foreground"),
                    (fields[0], *(int(value) for value in fields[1:6]))))


def alive(pid):
    try:
        return proc(pid)["state"] not in ("Z", "X")
    except FileNotFoundError:
        return False


def descendants(pid):
    parents = {}
    for entry in Path("/proc").iterdir():
        if entry.name.isdigit():
            try:
                parents[int(entry.name)] = proc(int(entry.name))["ppid"]
            except OSError:
                pass
    found, pending = set(), [pid]
    while pending:
        parent = pending.pop()
        children = {child for child, ppid in parents.items() if ppid == parent}
        pending.extend(children - found)
        found.update(children)
    return found


def check_detached(pid):
    state = proc(pid)
    assert state["sid"] == pid and state["pgid"] == pid, state
    assert state["tty"] == 0 and state["state"] != "T", state
    assert os.readlink(f"/proc/{pid}/fd/0") == "/dev/null"


def main():
    binary = Path(os.environ.get("IVNC_BIN", "target/debug/ivnc")).resolve()
    assert binary.is_file(), f"Build iVnc first: {binary}"
    assert shutil.which("google-chrome"), "Install google-chrome to test real windows"
    work = Path(tempfile.mkdtemp(prefix="ivnc-app-session-"))
    env = os.environ.copy()
    for key, directory in (("HOME", "home"), ("IVNC_HOME", "ivnc"),
                           ("XDG_RUNTIME_DIR", "run"), ("XDG_CONFIG_HOME", "config"),
                           ("XDG_DATA_HOME", "data"), ("XDG_CACHE_HOME", "cache")):
        path = work / directory
        path.mkdir(mode=0o700)
        env[key] = str(path)
    for key in ("WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DISPLAY", "LD_PRELOAD",
                "DBUS_SESSION_BUS_ADDRESS", "PULSE_SERVER"):
        env.pop(key, None)
    http_port, chrome_port = free_port(), free_port()
    while chrome_port == http_port:
        chrome_port = free_port()
    env["IVNC_CHROME_DEBUG_PORT"] = str(chrome_port)
    home = Path(env["HOME"])
    (home / ".profile").write_text("export PROFILE_FIXTURE=profile\n")
    (home / ".bashrc").write_text(
        "case $- in *i*) ;; *) return;; esac\n"
        "export PROFILE_FIXTURE=guarded-bashrc\nexport PROFILE_FIXTURE_HOME=$HOME\n")
    config = work / "config.toml"
    config.write_text("[audio]\nenabled=false\nsample_rate=48000\nchannels=2\nbitrate=128000\n"
                      "[shell_profile]\napps=true\nterminal=true\n")
    base = f"http://127.0.0.1:{http_port}"

    def api(path, method="GET", body=None):
        request = urllib.request.Request(
            base + path, method=method,
            data=json.dumps(body).encode() if body is not None else None,
            headers={"Content-Type": "application/json", "Origin": base})
        with urllib.request.urlopen(request, timeout=12) as response:
            return json.load(response)

    def controlling_terminal():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        os.tcsetpgrp(0, os.getpgrp())

    master, slave = os.openpty()
    server = None
    apps = set()
    paused = set()
    successful = False
    try:
        with (work / "server.log").open("wb") as log:
            server = subprocess.Popen(
                [str(binary), "--config", str(config), "--http-port", str(http_port),
                 "--basic-auth-enabled", "false", "--proxy-enabled", "false",
                 "--proxy-autostart", "false", "--width", "800", "--height", "600"],
                cwd=work, env=env, stdin=slave, stdout=log, stderr=log,
                preexec_fn=controlling_terminal)
        os.close(slave)
        slave = None
        wait("manager", lambda: api("/api/apps"))
        manager = proc(server.pid)
        assert manager["tty"] != 0 and manager["foreground"] == server.pid, manager
        print("PASS manager owns the foreground controlling PTY", flush=True)

        def details(app_id):
            return api(f"/api/apps/{app_id}")

        def pause_resume(app_id, pid):
            paused.add(pid)
            os.killpg(pid, signal.SIGSTOP)
            wait("suspended process diagnostic", lambda:
                 (details(app_id).get("launch_error") or {}).get("code") == "app_process_stopped")
            assert details(app_id)["pid"] == pid
            os.killpg(pid, signal.SIGCONT)
            wait("suspension diagnostic clears", lambda: details(app_id).get("launch_error") is None)
            paused.discard(pid)

        def stop(app_id):
            pid = details(app_id)["pid"]
            tracked = descendants(pid) | {pid}
            api(f"/api/apps/{app_id}/stop", "POST")
            wait("process tree exits", lambda: all(not alive(child) for child in tracked), timeout=8)
            assert details(app_id)["status"] == "stopped"
            apps.discard(app_id)

        chrome_id = "builtin-chrome"
        apps.add(chrome_id)
        api(f"/api/apps/{chrome_id}/start", "POST")
        chrome = wait("Chrome window", lambda: (value if value.get("window_ready") else None)
                      if (value := details(chrome_id)) else None)
        check_detached(chrome["pid"])
        with urllib.request.urlopen(f"http://127.0.0.1:{chrome_port}/json/version", timeout=5) as response:
            assert "Chrome/" in json.load(response)["Browser"]
        pause_resume(chrome_id, chrome["pid"])
        stop(chrome_id)
        print("PASS real Chrome opens a Wayland window, reports suspension/resume and stops its tree", flush=True)

        apps.add(chrome_id)
        api(f"/api/apps/{chrome_id}/start", "POST")
        wait("Chrome restart window", lambda: details(chrome_id).get("window_ready"))
        assert details(chrome_id)["pid"] != chrome["pid"]
        previous_generation = api(f"/api/apps/{chrome_id}/launch-info")["launch_info"]["generation"]
        stop(chrome_id)
        print("PASS Chrome restarts successfully from the same foreground manager", flush=True)

        report_path = work / "service-report.json"
        fixture = work / "service.py"
        fixture.write_text(
            "import json,os,pathlib,subprocess,signal,sys\n"
            "child=subprocess.Popen(['/bin/sleep','60'])\n"
            "def stop(*_):\n child.terminate();child.wait(timeout=3);sys.exit(0)\n"
            "signal.signal(signal.SIGTERM,stop)\n"
            "pathlib.Path(sys.argv[1]).write_text(json.dumps({"
            "'pid':os.getpid(),'child':child.pid,'home':os.environ['HOME'],"
            "'profile':os.environ.get('PROFILE_FIXTURE'),"
            "'profile_home':os.environ.get('PROFILE_FIXTURE_HOME')}))\n"
            "signal.pause()\n")
        created = api("/api/apps", "POST", {
            "name": "Controlling TTY service fixture", "app_type": "background",
            "launch_command": shlex.join(["exec", sys.executable, str(fixture), str(report_path)])})
        service_id = created["app"]["id"]
        apps.add(service_id)
        api(f"/api/apps/{service_id}/start", "POST")
        report = wait("service report", lambda: json.loads(report_path.read_text()))
        check_detached(report["pid"])
        assert report["profile"] == "guarded-bashrc", report
        assert report["profile_home"] == str(home), report
        assert report["home"] == str(Path(env["IVNC_HOME"]) / "apps" / service_id / "home"), report
        pause_resume(service_id, report["pid"])
        stop(service_id)
        print("PASS background service loads guarded rc, preserves HOME and stops its detached group", flush=True)

        api(f"/api/apps/{chrome_id}/sharing", "PUT", {"enabled": True, "password": "local-session-fixture"})
        apps.add(chrome_id)
        api(f"/api/apps/{chrome_id}/start", "POST")

        def worker_ready():
            report = api(f"/api/apps/{chrome_id}/launch-info")["launch_info"]
            return report if report and report["generation"] != previous_generation and report["window_seen"] else None

        wait("worker Chrome window", worker_ready)
        worker = details(chrome_id)
        assert worker["status"] == "running", worker
        check_detached(worker["pid"])
        pause_resume(chrome_id, worker["pid"])
        stop(chrome_id)
        print("PASS shared Chrome opens a real window and its supervisor still owns and stops the tree", flush=True)
        successful = True
    finally:
        for pid in paused:
            try:
                os.killpg(pid, signal.SIGCONT)
            except ProcessLookupError:
                pass
        for app_id in apps:
            try:
                api(f"/api/apps/{app_id}/stop", "POST")
            except OSError:
                pass
        if server is not None and server.poll() is None:
            server.terminate()
            try:
                server.wait(timeout=8)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait(timeout=3)
        os.close(master)
        if slave is not None:
            os.close(slave)
        if successful and not os.environ.get("KEEP_TMP"):
            shutil.rmtree(work)
        else:
            print(f"Artifacts: {work}", flush=True)


if __name__ == "__main__":
    main()

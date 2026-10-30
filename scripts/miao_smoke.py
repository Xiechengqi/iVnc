#!/usr/bin/env python3
"""Isolated Miao integration regression. Requires root and an iVnc binary with TLS.

No host HOME, TUN, global proxy, existing services or browser profiles are changed.
Only the panel starts; each scenario owns HOME, IVNC_HOME, sockets and ports.
"""
import argparse
import base64
import concurrent.futures
import http.client
import json
import os
from pathlib import Path
import signal
import socket
import ssl
import subprocess
import tempfile
import time


def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def eventually(check, seconds=25):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            value = check()
            if value:
                return value
        except (OSError, http.client.HTTPException, KeyError, ValueError):
            pass
        time.sleep(0.1)
    raise AssertionError("condition did not become true before timeout")


def alive(pid):
    try:
        return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0] != "Z"
    except FileNotFoundError:
        return False


class Instance:
    def __init__(self, binary, root, tls):
        self.binary, self.root, self.tls = binary, root, tls
        self.http_port, self.miao_port = port(), port()
        self.origin = f"{'https' if tls else 'http'}://127.0.0.1:{self.http_port}"
        self.auth = "Basic " + base64.b64encode(b"smoke:isolated-test").decode()
        self.env = dict(os.environ, HOME=str(root / "home"), IVNC_HOME=str(root / "ivnc"),
                        XDG_RUNTIME_DIR=str(root / "runtime"), IVNC_MIAO_PORT=str(self.miao_port),
                        IVNC_CHROME_DEBUG_PORT=str(port()))
        for key in ("HOME", "IVNC_HOME", "XDG_RUNTIME_DIR"):
            Path(self.env[key]).mkdir(mode=0o700, parents=True)
        self.config = Path(self.env["HOME"]) / ".miao" / "config.yaml"
        self.config.parent.mkdir()
        self.saved_config = f"port: {port()}\nsocks_listen: 127.0.0.1\nsocks_port: {port()}\nmode: pool\n"
        self.config.write_text(self.saved_config)
        self.process = None
        self.log = None
        self.miao_pids = set()

    def connection(self):
        if self.tls:
            return http.client.HTTPSConnection("127.0.0.1", self.http_port, timeout=20,
                                               context=ssl._create_unverified_context())
        return http.client.HTTPConnection("127.0.0.1", self.http_port, timeout=20)

    def request(self, path, method="GET", body=None, headers=None, status=200):
        conn = self.connection()
        request_headers = {"Authorization": self.auth, "Origin": self.origin}
        if body is not None:
            request_headers["Content-Type"] = "application/json"
        request_headers.update(headers or {})
        try:
            conn.request(method, path, None if body is None else json.dumps(body), request_headers)
            response = conn.getresponse()
            data = response.read()
            assert response.status == status, (path, response.status, data[:300])
            try:
                value = json.loads(data)
            except ValueError:
                value = data.decode()
            return value, {key.lower(): value for key, value in response.getheaders()}
        finally:
            conn.close()

    def app(self):
        return self.request("/api/apps/builtin-miao")[0]

    def ready(self):
        app = self.app()
        if app.get("ready") and app.get("pid"):
            self.miao_pids.add(app["pid"])
            return app
        return False

    def boot(self, enabled=True):
        self.log = (self.root / "ivnc.log").open("a")
        args = [str(self.binary), "--foreground", "--http-port", str(self.http_port),
                "--basic-auth-enabled", "true", "--basic-auth-user", "smoke",
                "--basic-auth-password", "isolated-test", "--proxy-enabled", str(enabled).lower()]
        if self.tls:
            args.append("--tls")
        self.process = subprocess.Popen(args, cwd=self.root, env=self.env, stdout=self.log, stderr=self.log)
        eventually(lambda: self.process.poll() is None and self.request("/health")[0])
        eventually(lambda: self.app())

    def stop(self):
        if self.process and self.process.poll() is None:
            self.process.send_signal(signal.SIGINT)
            try:
                self.process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
                raise AssertionError("iVnc did not shut down gracefully")
        if self.log:
            self.log.close()
            self.log = None

    def websocket(self, cookie=None, origin=None, status=101):
        conn = self.connection()
        headers = {"Authorization": self.auth, "Origin": origin or self.origin,
                   "Connection": "Upgrade", "Upgrade": "websocket", "Sec-WebSocket-Version": "13",
                   "Sec-WebSocket-Key": base64.b64encode(os.urandom(16)).decode()}
        if cookie:
            headers["Cookie"] = cookie
        try:
            conn.request("GET", "/proxy-ws/api/clash/traffic", headers=headers)
            response = conn.getresponse()
            assert response.status == status, ("websocket", response.status)
        finally:
            conn.close()


def scenario(binary, root, tls, browser=False):
    instance = Instance(binary, root, tls)
    app_path = "/api/apps/builtin-miao"
    try:
        instance.boot()
        app = eventually(instance.ready)
        assert app["autostart"] and app["app_type"] == "background" and app["builtin"]
        assert not app["can_delete"] and not app["can_clear_data"]
        legacy = instance.request("/api/proxy/status")[0]
        assert app["pid"] == legacy["pid"]
        assert instance.config.read_text() == instance.saved_config
        status = instance.request("/proxy/api/auth/status")[0]["data"]
        assert status["service_pid"] == app["pid"] and status["embedded"] and not status["proxy_running"]
        panel, headers = instance.request("/proxy/")
        assert "__ivncProxyPanelRewrite" in panel and headers["x-frame-options"] == "SAMEORIGIN"
        instance.request("/proxy/api/status", status=401)
        password = "isolated-panel-password"
        result, headers = instance.request("/proxy/api/auth/setup", "POST",
                                           {"password": password, "confirm_password": password})
        assert result["success"]
        cookie, csrf = headers["set-cookie"].split(";", 1)[0], result["data"]["csrf"]
        assert ("; Secure" in headers["set-cookie"]) == tls
        session = {"Cookie": cookie, "X-Miao-CSRF": csrf}
        if browser:
            browser_env = dict(instance.env, MIAO_SMOKE_AUTH=instance.auth, MIAO_SMOKE_COOKIE=cookie)
            subprocess.run(["node", str(Path(__file__).with_name("miao_console_smoke.mjs")),
                            instance.origin, str(root)], env=browser_env, check=True, timeout=60)
        instance.request("/proxy/api/status", headers=session)
        instance.request("/proxy/api/auth/logout", "POST", {}, {"Cookie": cookie}, status=403)
        instance.request("/proxy/api/auth/logout", "POST", {}, dict(session, Origin="https://foreign.example"), status=403)
        instance.websocket(status=401)
        instance.websocket(cookie, origin="https://foreign.example", status=403)
        instance.websocket(cookie)
        # Preserve an old panel request path without intercepting iVnc management calls.
        instance.request("/api/auth/status", headers={"Referer": instance.origin + "/proxy/"})
        result = instance.request("/proxy/api/upgrade", "POST", {}, session)[0]
        assert not result["success"] and "managed by iVnc" in result["message"]
        instance.request(app_path, "DELETE", status=403)
        instance.request(app_path + "/clear-data", "POST", status=403)
        instance.request(app_path, "PUT", {"launch_command": "false"}, status=403)
        instance.request(app_path, "PUT", {"autostart": False})
        assert not instance.app()["autostart"]
        instance.request(app_path + "/stop", "POST")
        instance.request("/proxy/", status=503)
        instance.websocket(cookie, status=503)
        time.sleep(3.5)
        assert instance.app()["status"] == "stopped"
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
            replies = list(executor.map(lambda _: instance.request("/api/proxy/start", "POST")[0], range(4)))
        assert len({reply["pid"] for reply in replies}) == 1
        app = eventually(instance.ready)
        os.kill(app["pid"], signal.SIGKILL)
        eventually(lambda: (new := instance.ready()) and new["pid"] != app["pid"])
        # Simulate a host crash while Miao is manually running with autostart off.
        instance.process.kill()
        instance.process.wait()
        instance.stop()
        # Old state files must never override the explicit Miao autostart preference.
        (Path(instance.env["IVNC_HOME"]) / "app_running_state.json").write_text(
            json.dumps({"app_ids": ["builtin-miao"], "timestamp": int(time.time())}))
        instance.boot()
        time.sleep(3.5)
        assert instance.app()["status"] == "stopped" and not instance.app()["autostart"]
        eventually(lambda: all(not alive(pid) for pid in instance.miao_pids))
        instance.request("/proxy/", status=503)
        instance.request("/api/proxy/start", "POST")
        eventually(instance.ready)
        # Authentication and the original YAML survive process and iVnc restarts.
        result, _ = instance.request("/proxy/api/auth/login", "POST", {"password": password})
        assert result["success"]
        assert instance.config.read_text() == instance.saved_config
        instance.request(app_path, "PUT", {"autostart": True})
        instance.stop()
        instance.boot(enabled=False)
        time.sleep(3.5)
        assert instance.app()["enabled"] is False and instance.app()["status"] == "stopped"
        instance.request(app_path + "/start", "POST", status=403)
        instance.request(app_path + "/restart", "POST", status=403)
        instance.request("/api/proxy/start", "POST", status=404)
        instance.request("/proxy/", status=404)
        print(f"PASS {'HTTPS' if tls else 'HTTP'}: lifecycle, preference, auth, CSRF, WS, configuration and master switch", flush=True)
    except BaseException:
        print((root / "ivnc.log").read_text()[-6000:], flush=True)
        miao_log = Path(instance.env["IVNC_HOME"]) / "miao/miao.log"
        if miao_log.exists():
            print(miao_log.read_text()[-4000:], flush=True)
        raise
    finally:
        instance.stop()
        assert all(not alive(pid) for pid in instance.miao_pids), "owned Miao process leaked"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--http-only", action="store_true")
    parser.add_argument("--browser", action="store_true", help="also verify the shipped UI in an isolated headless Chrome (Node 20+)")
    args = parser.parse_args()
    if os.geteuid() != 0:
        parser.error("Miao requires root; run this isolated test as root")
    with tempfile.TemporaryDirectory(prefix="ivnc-miao-smoke-") as directory:
        for tls in ([False] if args.http_only else [False, True]):
            root = Path(directory) / ("https" if tls else "http")
            root.mkdir()
            scenario(args.binary.resolve(), root, tls, args.browser)


if __name__ == "__main__":
    main()

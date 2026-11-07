#!/usr/bin/env python3
"""Exercise two real desktop sandboxes, X11, private audio and service allowlists.

Requires the dependencies from remote_app_smoke.sh plus Xwayland, openbox,
pulseaudio/pactl and Python 3. Set IVNC_BIN to the freshly built binary.
"""
import json
import os
from pathlib import Path
import shlex
import socket
import subprocess
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
BIN = os.environ.get('IVNC_BIN', str(ROOT / 'target/release/ivnc'))


def port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def wait_for(label, fn, timeout=25):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            if fn():
                print('PASS', label, flush=True)
                return
        except (OSError, ValueError):
            pass
        time.sleep(.2)
    raise AssertionError('Timed out: ' + label)


with tempfile.TemporaryDirectory(prefix='ivnc-isolation-smoke.') as tmp:
    tmp = Path(tmp)
    home = tmp / 'home'
    run = tmp / 'run'
    run.mkdir(mode=0o700)
    home.mkdir()
    http_port, service_port = port(), port()
    base = f'http://127.0.0.1:{http_port}'
    config = (ROOT / 'config.example.toml').read_text().replace('port = 8008', f'port = {http_port}')
    config = config.replace('basic_auth_enabled = true', 'basic_auth_enabled = false')
    config = config.replace('[audio]\n# Enable audio streaming\nenabled = false', '[audio]\nenabled = true')
    (tmp / 'config.toml').write_text(config)
    env = {**os.environ, 'HOME': str(tmp / 'user'), 'IVNC_HOME': str(home), 'XDG_RUNTIME_DIR': str(run)}
    env.pop('PULSE_SERVER', None)
    env.pop('PULSE_SOURCE', None)
    log = (tmp / 'server.log').open('w')
    process = subprocess.Popen([BIN, '-c', str(tmp / 'config.toml'), '--proxy-enabled', 'false'], env=env, stdout=log, stderr=log)
    apps = []

    def api(path, method='GET', data=None):
        request = urllib.request.Request(base + path, method=method,
            data=json.dumps(data).encode() if data is not None else None,
            headers={'Content-Type': 'application/json'})
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)

    def app(data):
        result = api('/api/apps', 'POST', data)['app']['id']
        apps.append(result)
        return result

    def start(app_id):
        assert api(f'/api/apps/{app_id}/start', 'POST')['ok']

    try:
        wait_for('manager ready', lambda: api('/health'))
        service = app({'name': 'allowed-service', 'app_type': 'background',
            'url': f'http://127.0.0.1:{service_port}/',
            'launch_command': f'python3 -m http.server {service_port} --bind 127.0.0.1'})
        start(service)
        first = app({'name': 'Wayland-private', 'app_type': 'desktop', 'exec_command': 'exec gtk3-demo'})
        first_home = home / 'apps' / first / 'home'
        first_home.mkdir(parents=True, exist_ok=True)
        (first_home / 'other-app-secret').write_text('private to the first application')
        start(first)
        second = app({'name': 'X11-private', 'app_type': 'desktop',
            'exec_command': 'sh "$HOME/probe.sh" && exec gtk3-demo'})
        second_home = home / 'apps' / second / 'home'
        second_home.mkdir(parents=True, exist_ok=True)
        api(f'/api/apps/{second}/sharing', 'PUT', {'enabled': False,
            'desktop': {'x11': True, 'services': [service]}})
        # These probes run *inside* the application namespace, with its privileges.
        probe = f'''set -eu
[ ! -e {shlex.quote(str(first_home / 'other-app-secret'))} ]
[ ! -e {shlex.quote(str(home / 'apps.db'))} ]
[ ! -e {shlex.quote(str(home / 'remote.key'))} ]
[ "$(pactl get-default-sink)" = app ]
[ "$(pactl list short sinks | wc -l)" -eq 1 ]
[ -S /tmp/.X11-unix/X0 ]
curl --noproxy '*' -fsS -m 5 http://127.0.0.1:{service_port}/ >/dev/null
if curl --noproxy '*' -fsS -m 2 http://127.0.0.1:{http_port}/health >/dev/null 2>&1; then exit 1; fi
if curl --noproxy '*' -fsS -m 2 http://10.0.2.2:{http_port}/health >/dev/null 2>&1; then exit 1; fi
if nft flush ruleset >/dev/null 2>&1; then exit 1; fi
printf 'PASS files network audio X11 capabilities\\n' > "$HOME/probe-result"
'''
        (second_home / 'probe.sh').write_text(probe)
        start(second)
        wait_for('private files, allowed service, blocked manager, private audio and X11',
            lambda: (second_home / 'probe-result').exists())
        assert api(f'/api/apps/{first}')['status'] == 'running'
        assert api(f'/api/apps/{second}')['status'] == 'running'
        e1 = json.loads((home / 'runtimes' / first / 'endpoint.json').read_text())
        e2 = json.loads((home / 'runtimes' / second / 'endpoint.json').read_text())
        assert e1['generation'] != e2['generation'] and e1['port'] != e2['port']
        print('PASS independent compositor generations and endpoints', flush=True)
        view = api(f'/api/apps/{second}/view', 'POST')['url']
        browser_dir = tmp / 'browser-test'
        browser_dir.mkdir()
        subprocess.run(['node', str(ROOT / 'scripts/remote_view_smoke.mjs'),
            base + view + '?app=' + second, str(browser_dir), '--audio'], check=True, timeout=130)
        print('PASS X11 application renders and accepts input in a real browser', flush=True)
        api(f'/api/apps/{second}/stop', 'POST')
        assert api(f'/api/apps/{first}')['status'] == 'running'
        assert (first_home / 'other-app-secret').read_text() == 'private to the first application'
        print('PASS stopping one application leaves the other running with its data', flush=True)
    except BaseException:
        for app_id in apps:
            app_log = home / 'apps' / app_id / 'app.log'
            if app_log.exists():
                print('\nApplication log:', app_id, '\n', app_log.read_text(errors='replace')[-6000:])
        print('\nServer log:\n', (tmp / 'server.log').read_text(errors='replace')[-4000:])
        raise
    finally:
        for app_id in reversed(apps):
            try:
                api(f'/api/apps/{app_id}/stop', 'POST')
            except OSError:
                pass
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        log.close()

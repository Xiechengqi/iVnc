ARG BASE_IMAGE=ghcr.io/xiechengqi/ivnc:latest
FROM ${BASE_IMAGE}

# Example:
#   docker build -f Dockerfile.App --build-arg BASE_IMAGE=ghcr.io/your-org/ivnc:latest -t ivnc-apps:latest .
#
# This file appends preset apps into /root/.config/ivnc/apps.db
# without replacing the base image's existing apps. The built-in Terminal
# app is seeded by iVnc at runtime and is not touched here.

RUN mkdir -p /root/.config/ivnc \
    && sqlite3 /root/.config/ivnc/apps.db "\
CREATE TABLE IF NOT EXISTS apps (\
    id TEXT PRIMARY KEY,\
    name TEXT NOT NULL UNIQUE,\
    url TEXT,\
    mode TEXT,\
    dark_mode INTEGER DEFAULT 0,\
    autostart INTEGER DEFAULT 0,\
    show_nav INTEGER DEFAULT 0,\
    created_at TEXT NOT NULL,\
    app_type TEXT DEFAULT 'webapp',\
    exec_command TEXT,\
    env_vars TEXT,\
    remote_debugging_port INTEGER,\
    proxy_server TEXT\
);\
INSERT OR IGNORE INTO apps \
    (id, name, app_type, url, mode, autostart, show_nav, exec_command, env_vars, created_at, remote_debugging_port, proxy_server) \
VALUES \
    ('preset-cc-switch', 'cc-switch', 'desktop', NULL, NULL, 1, 0, 'cc-switch', '', '2026-04-23T00:00:00Z', NULL, NULL),\
    ('preset-google', 'Google', 'webapp', 'https://google.com', 'native', 0, 1, '', '', '2026-04-23T00:00:01Z', NULL, NULL);\
"

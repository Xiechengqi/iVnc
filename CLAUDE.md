# Project Rules

## Build & Deploy

- Local build: `bash build.sh` (builds frontend + backend; final `cp` may fail with "Text file busy" if ivnc is running — stop it first)
- Build & restart sequence: stop tmux → cp binary → start tmux:
  ```
  tmux send-keys -t ivnc C-c C-c
  sleep 2
  cp -f target/release/ivnc ivnc
  tmux send-keys -t ivnc 'ivnc' Enter
  ```
- Verify: `curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8008/` should return 401

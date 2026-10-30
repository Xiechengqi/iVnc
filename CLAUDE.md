# Project Rules

## Build & Deploy

- Local build: `bash build.sh` (builds the frontend, paired Miao from `../miao` or `MIAO_SOURCE_DIR`, and backend; `IVNC_MIAO_BIN` can supply a compatible artifact). The final binary replacement is atomic and does not restart running services.
- Build & restart sequence: stop tmux → cp binary → start tmux:
  ```
  tmux send-keys -t ivnc C-c C-c
  sleep 2
  cp -f target/release/ivnc ivnc
  tmux send-keys -t ivnc 'ivnc' Enter
  ```
- Verify: `curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8008/` should return 401

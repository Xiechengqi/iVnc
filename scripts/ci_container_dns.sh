#!/usr/bin/env bash
# Select DNS inside the same image/network used by the release build. Host
# networking alone does not replace Docker's generated /etc/resolv.conf.
set -euo pipefail

if [[ $# -lt 3 ]]; then
    echo "Usage: $0 IMAGE PLATFORM OUTPUT_RESOLV_CONF [RUNNER_RESOLV_CONF ...]" >&2
    exit 2
fi
build_image="$1"
build_platform="$2"
output_file="$3"
shift 3
if [[ $# == 0 ]]; then
    set -- /etc/resolv.conf /run/systemd/resolve/resolv.conf
fi
mkdir -p -- "$(dirname -- "$output_file")"
output_dir="$(cd -- "$(dirname -- "$output_file")" && pwd)"
output_file="$output_dir/$(basename -- "$output_file")"
candidate_dir="$(mktemp -d "$output_dir/ivnc-dns.XXXXXX")"
trap 'rm -rf -- "$candidate_dir"' EXIT

# Separate an image-pull failure from DNS failures in an already running image.
docker pull --platform "$build_platform" "$build_image"

try_dns() {
    local label="$1" candidate="$2" status=0
    echo "Checking build-container DNS: $label"
    docker run --rm --network host --platform "$build_platform" \
        --mount "type=bind,src=$candidate,dst=/etc/resolv.conf,readonly" \
        "$build_image" timeout --kill-after=2s 20s /bin/bash -ec '
            cat /etc/resolv.conf
            for hostname in azure.archive.ubuntu.com security.ubuntu.com \
                deb.nodesource.com registry.npmjs.org sh.rustup.rs \
                static.rust-lang.org index.crates.io github.com; do
                if ! getent ahostsv4 "$hostname" >/dev/null; then
                    echo "DNS lookup failed: $hostname" >&2
                    exit 1
                fi
            done
        ' || status=$?
    if [[ "$status" == 0 ]]; then
        cp -- "$candidate" "$output_file" || exit 1
        echo "Selected build-container DNS: $label"
        return 0
    fi
    case "$status" in
        125|126|127)
            echo "::error::DNS probe container could not run (exit $status)." >&2
            exit "$status"
            ;;
    esac
    echo "DNS configuration failed its container probe (exit $status): $label" >&2
    return 1
}

index=0
for runner_resolv in "$@"; do
    [[ -r "$runner_resolv" ]] || continue
    index=$((index + 1))
    candidate="$candidate_dir/runner-$index.conf"
    # Build dependencies use FQDNs; preserve the runner's resolver addresses
    # (including a local stub reachable over host networking), with bounded waits.
    awk '$1 == "nameserver" && NF >= 2 { print $1, $2 }' "$runner_resolv" > "$candidate"
    [[ -s "$candidate" ]] || continue
    printf 'options timeout:2 attempts:2\n' >> "$candidate"
    if try_dns "$runner_resolv" "$candidate"; then exit 0; fi
done

# Public DNS is a tested fallback, never an assumption about runner egress.
for resolver in 1.1.1.1 8.8.8.8; do
    candidate="$candidate_dir/public-$resolver.conf"
    printf 'nameserver %s\noptions timeout:2 attempts:2\n' "$resolver" > "$candidate"
    if try_dns "$resolver" "$candidate"; then exit 0; fi
done

echo "::error::No DNS configuration could resolve the build dependencies inside $build_image. Check runner DNS and DNS egress; APT has not been started." >&2
exit 1

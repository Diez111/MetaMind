#!/usr/bin/env bash
# One lane for every build in this workspace.
#
# Why this exists: `build-capped` puts the target in /dev/shm, which is 14 G on this
# machine and shared with everything else. This workspace does not fit: the linker dies
# with `ld terminated with signal 7 [Bus error]` while writing a test binary, and because
# the failing command used to end in a pipe, the shell still reported exit 0. A build that
# lies about failing is worse than a slow build.
#
# So: the target goes to disk (372 G free), the command runs inside the capped slice
# (`pi-build.slice`: CPU capped, no disk hammering), and the exit code of the whole
# pipeline is what the caller sees (`set -o pipefail`).
#
# Usage:
#   scripts/dev.sh test [args...]     cargo test
#   scripts/dev.sh check [args...]    cargo check
#   scripts/dev.sh clippy [args...]   cargo clippy (extra args are passed through)
#   scripts/dev.sh build [args...]    cargo build
#   scripts/dev.sh fmt [args...]      cargo fmt
#   scripts/dev.sh run [args...]      cargo run
#   scripts/dev.sh bench [args...]    cargo bench
#   scripts/dev.sh clean              remove the build artifacts (the target on disk)
#
# Environment:
#   CARGO_TARGET_DIR   honoured when already set (use it to share a target between runs)
#   MM_NO_CAP=1        run the command without the build slice (useful inside CI containers)
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"

capped() {
    if [ "${MM_NO_CAP:-0}" = "1" ] || ! command -v systemd-run >/dev/null 2>&1; then
        "$@"
        return
    fi
    local tool="$HOME/.local/bin/build-capped"
    if [ -x "$tool" ]; then
        "$tool" "$@"
    else
        "$@"
    fi
}

usage() {
    sed -n '13,22p' "$0" | sed 's/^# \{0,1\}//'
}

cmd="${1:-}"
if [ -z "$cmd" ] || [ "$cmd" = "-h" ] || [ "$cmd" = "--help" ]; then
    usage
    exit 0
fi
shift

case "$cmd" in
    fmt)    capped cargo fmt "$@" ;;
    test)   capped cargo test "$@" ;;
    check)  capped cargo check "$@" ;;
    clippy) capped cargo clippy "$@" ;;
    build)  capped cargo build "$@" ;;
    run)    capped cargo run "$@" ;;
    bench)  capped cargo bench "$@" ;;
    clean)
        # `cargo clean` follows CARGO_TARGET_DIR: without this the slice's tmpfs target would be
        # the one cleaned and the 180 GB of stale fingerprints in `target/` would stay.
        capped cargo clean "$@"
        du -sh "$CARGO_TARGET_DIR" 2>/dev/null || true
        ;;
    *)
        echo "dev.sh: unknown command '$cmd'" >&2
        usage >&2
        exit 64
        ;;
esac

#!/bin/sh
here="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
sys="$(uname -s)"
case "$sys" in
MINGW* | MSYS* | CYGWIN*) rel="x86_64-pc-windows-msvc/gray-ssh.exe" ;;
Darwin*)
    case "$(uname -m)" in
    arm64) rel="aarch64-apple-darwin/gray-ssh" ;;
    *) rel="x86_64-apple-darwin/gray-ssh" ;;
    esac ;;
*)
    case "$(uname -m)" in
    aarch64 | arm64) rel="aarch64-unknown-linux-musl/gray-ssh" ;;
    *) rel="x86_64-unknown-linux-musl/gray-ssh" ;;
    esac ;;
esac
if [ -x "$here/$rel" ]; then exec "$here/$rel" "$@"; fi
for cand in "$here/target/release/gray-ssh.exe" "$here/target/release/gray-ssh" "$here/target/debug/gray-ssh.exe" "$here/target/debug/gray-ssh"; do
    if [ -x "$cand" ]; then exec "$cand" "$@"; fi
done
echo "gray-ssh binary not found in $here" >&2
exit 1

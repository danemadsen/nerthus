#!/bin/sh
# Builds the web version of the editor into a folder ready to serve
# (target/web, or the folder given): the editor compiled to WebAssembly,
# wasm-bindgen's JavaScript for it, and the page that starts it.
#
# Needs the wasm32-unknown-unknown target (rustup target add
# wasm32-unknown-unknown) and the wasm-bindgen command of the version in
# Cargo.lock (cargo install wasm-bindgen-cli --version <that version>).
# Extra arguments go to cargo build.
set -eu

root=$(cd "$(dirname "$0")/../../.." && pwd)
out=${1:-"$root/target/web"}
if [ "$#" -gt 0 ]; then
    shift
fi

# The wasm-bindgen command must match the library the editor links.
locked=$(sed -n '/^name = "wasm-bindgen"$/{n;s/^version = "\(.*\)"$/\1/p;}' "$root/Cargo.lock")
installed=$(wasm-bindgen --version | cut -d ' ' -f 2)
if [ "$locked" != "$installed" ]; then
    echo "wasm-bindgen $installed is installed, but Cargo.lock has $locked:" >&2
    echo "    cargo install wasm-bindgen-cli --version $locked --locked" >&2
    exit 1
fi

cargo build --manifest-path "$root/Cargo.toml" --release --target wasm32-unknown-unknown -p nerthus-app "$@"

rm -rf "$out"
mkdir -p "$out"
wasm-bindgen --target web --no-typescript --out-dir "$out" "$root/target/wasm32-unknown-unknown/release/nerthus.wasm"
cp "$root/crates/nerthus-app/web/index.html" "$out/"
cp "$root/crates/nerthus-app/assets/nerthus.svg" "$out/"
echo "Built the web version in $out"

#!/bin/bash
# Packages an existing release build as Nerthus.app and a distributable zip.
# Requires macOS, Python 3.11+, and rsvg-convert (brew install librsvg).
set -euo pipefail

if [ "$#" -gt 2 ]; then
    echo "Usage: $0 [binary-dir] [output-dir]" >&2
    exit 1
fi

if [ "$(uname -s)" != "Darwin" ]; then
    echo "The macOS package must be assembled on macOS." >&2
    exit 1
fi

repo_root=$(cd "$(dirname "$0")/.." && pwd)
binary_dir=${1:-"$repo_root/target/release"}
output_dir=${2:-"$repo_root/target/packages"}

for tool in python3 rsvg-convert iconutil codesign ditto; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "Required packaging tool is missing: $tool" >&2
        exit 1
    fi
done

for binary in nerthus nerthus-build; do
    if [ ! -x "$binary_dir/$binary" ]; then
        echo "Missing executable: $binary_dir/$binary (run cargo build --release --locked first)." >&2
        exit 1
    fi
done

mkdir -p "$output_dir"
output_dir=$(cd "$output_dir" && pwd)
staging_dir=$(mktemp -d "$output_dir/macos-package.XXXXXX")
trap 'rm -rf "$staging_dir"' EXIT

bundle="$staging_dir/Nerthus.app"
macos_dir="$bundle/Contents/MacOS"
resources_dir="$bundle/Contents/Resources"
mkdir -p "$macos_dir" "$resources_dir/licenses"

# The editor finds the command-line builder beside its own executable.
cp "$binary_dir/nerthus" "$binary_dir/nerthus-build" "$macos_dir/"
chmod 755 "$macos_dir/nerthus" "$macos_dir/nerthus-build"
cp "$repo_root/README.md" "$repo_root/LICENSE" "$resources_dir/"
cp "$repo_root/crates/nerthus-app/assets/fonts/roboto-license.txt" "$resources_dir/licenses/"
cp "$repo_root/crates/nerthus-app/assets/fonts/roboto-mono-license.txt" "$resources_dir/licenses/"
cp "$repo_root/crates/nerthus-app/assets/icons/tabler-license.txt" "$resources_dir/licenses/"
cp "$repo_root/crates/nerthus-app/assets/docs/LICENSE.txt" "$resources_dir/licenses/docs-license.txt"

python3 - "$repo_root/Cargo.toml" "$bundle/Contents/Info.plist" <<'PY'
import pathlib
import plistlib
import sys
import tomllib

manifest_path = pathlib.Path(sys.argv[1])
with manifest_path.open("rb") as manifest_file:
    manifest = tomllib.load(manifest_file)
version = manifest["workspace"]["package"]["version"]

bundle_info = {
    "CFBundleDevelopmentRegion": "en",
    "CFBundleDisplayName": "Nerthus",
    "CFBundleExecutable": "nerthus",
    "CFBundleIconFile": "nerthus.icns",
    "CFBundleIdentifier": "app.nerthus.Nerthus",
    "CFBundleInfoDictionaryVersion": "6.0",
    "CFBundleName": "Nerthus",
    "CFBundlePackageType": "APPL",
    "CFBundleShortVersionString": version,
    "CFBundleVersion": version,
    "NSHighResolutionCapable": True,
    "NSPrincipalClass": "NSApplication",
    "NSSupportsAutomaticGraphicsSwitching": True,
}
plist_path = pathlib.Path(sys.argv[2])
with plist_path.open("wb") as plist_file:
    plistlib.dump(bundle_info, plist_file)
PY

# Draw the macOS icon (the logo's rune on a yellow tile in the rounded-square
# icon shape) as the Dock icon does, with the same 100/1024 inset.
# iconutil requires each point size at both 1x and 2x scale.
iconset_dir="$staging_dir/nerthus.iconset"
mkdir -p "$iconset_dir"
for point_size in 16 32 128 256 512; do
    for scale in 1 2; do
        pixel_size=$((point_size * scale))
        inset=$((pixel_size * 100 / 1024))
        artwork_size=$((pixel_size - inset * 2))
        suffix=""
        if [ "$scale" -eq 2 ]; then
            suffix="@2x"
        fi
        icon_path="$iconset_dir/icon_${point_size}x${point_size}${suffix}.png"
        rsvg-convert --keep-aspect-ratio \
            --width "$artwork_size" --height "$artwork_size" \
            --page-width "$pixel_size" --page-height "$pixel_size" \
            --left "$inset" --top "$inset" \
            --output "$icon_path" "$repo_root/crates/nerthus-app/assets/nerthus-macos.svg"
    done
done
iconutil --convert icns --output "$resources_dir/nerthus.icns" "$iconset_dir"
plutil -lint "$bundle/Contents/Info.plist"

# Ad-hoc signing seals the finished bundle and its helper without requiring
# an Apple developer certificate. This does not notarize the application.
codesign --force --sign - "$macos_dir/nerthus-build"
codesign --force --sign - "$bundle"
codesign --verify --deep --strict "$bundle"

archive="$output_dir/nerthus-macos-$(uname -m).zip"
rm -f "$archive"
ditto -c -k --sequesterRsrc --keepParent "$bundle" "$archive"

rm -rf "$output_dir/Nerthus.app"
mv "$bundle" "$output_dir/Nerthus.app"
echo "Packaged $output_dir/Nerthus.app"
echo "Archive: $archive"

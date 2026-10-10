#!/usr/bin/env bash
# Wraps a built fastrock binary into Fastrock.app.
#
# Usage: bundle-app.sh <fastrock binary> <output dir> <version>
#                      [--bundle-id ID]
#
# Codex remains installed externally. This bundle is unsigned.
set -euo pipefail

binary="$1"
out_dir="$2"
prerelease_version="$3"
version="${prerelease_version%%-*}"
shift 3
bundle_id="com.allquixotic.fastrock"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --bundle-id)
            bundle_id="$2"
            shift 2
            ;;
        *)
            bundle_id="$1"
            shift
            ;;
    esac
done

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
icon_png="${here}/../../ui/assets/icon.png"
app="${out_dir}/Fastrock.app"

rm -rf "$app"
mkdir -p "${app}/Contents/MacOS" "${app}/Contents/Resources"
ditto "$binary" "${app}/Contents/MacOS/fastrock"
sed -e "s/@VERSION@/${version}/g" -e "s/@BUNDLE_ID@/${bundle_id}/g" \
    -e "s/@PRERELEASE_VERSION@/${prerelease_version}/g" \
    "${here}/Info.plist.in" > "${app}/Contents/Info.plist"

iconset="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$icon_png" --out "${iconset}/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" "$icon_png" --out "${iconset}/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "${app}/Contents/Resources/AppIcon.icns"
rm -rf "$(dirname "$iconset")"

plutil -lint "${app}/Contents/Info.plist" >/dev/null
echo "$app"

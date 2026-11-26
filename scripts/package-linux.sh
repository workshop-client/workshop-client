#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --locked --profile dist --no-default-features
package="target/package/workshop-client-linux-x64"
mkdir -p "$package"
install -m755 target/dist/workshop-client "$package/workshop-client"
install -m755 target/dist/libworkshop_client.so "$package/libworkshop_client.so"
cat > "$package/README.txt" <<'TEXT'
Workshop Client, Linux x64 preview

Keep workshop-client and libworkshop_client.so together.
Launch with: ./workshop-client launch /path/to/Discord.AppImage
Or set WORKSHOP_DISCORD_APPIMAGE and run ./workshop-client.

Clear URLs and Global Badges are embedded WASM plugins. Toggle them in
Workshop Settings > Plugins List. Developer CLI commands are omitted.

Native integration currently supports only the inspected Phoenix build:
0.1.0-dev.20261123093910.gb71ec80, Linux x64.

Discord itself is not included. Global Badges image preparation currently
requires curl, ImageMagick, FFmpeg, and timeout installed on the system.
This is a development preview; animation/profile compatibility is still
being verified. Do not treat it as a tested public stable release.
TEXT
size=$(du -cb "$package/workshop-client" "$package/libworkshop_client.so" "$package/README.txt" | tail -1 | cut -f1)
if (( size > 5000000 )); then
  echo "Release exceeds 5,000,000-byte package budget: $size bytes" >&2
  exit 1
fi
tar -czf target/package/workshop-client-linux-x64.tar.gz -C target/package workshop-client-linux-x64
printf 'Uncompressed package: %s bytes\n' "$size"
ls -lh "$package/workshop-client" "$package/libworkshop_client.so" target/package/workshop-client-linux-x64.tar.gz

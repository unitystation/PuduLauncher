#!/bin/bash
set -euo pipefail

SRC=/repo
WORK=/work/pudu

# Keep the cargo target dir for incremental builds
if [ -d "$WORK/src-tauri/target" ]; then
    mv "$WORK/src-tauri/target" /work/target-cache
fi
rm -rf "$WORK"
mkdir -p "$WORK"
(cd "$SRC" && tar -cf - \
    --exclude=./node_modules --exclude=./dist --exclude=./dist-steam \
    --exclude=./src-tauri/target --exclude=./src-tauri/binaries \
    --exclude=./.git .) | tar -xf - -C "$WORK"
if [ -d /work/target-cache ]; then
    mkdir -p "$WORK/src-tauri"
    mv /work/target-cache "$WORK/src-tauri/target"
fi

cd "$WORK"
# Windows checkouts can leave CRLF in sh files
find tools -name '*.sh' -exec sed -i 's/\r$//' {} +

npm ci
npx tauri build --no-bundle --features no-updater \
    --config '{"bundle":{"createUpdaterArtifacts":false}}'

OUT="$WORK/out/pudu-launcher-linux-x64"
rm -rf "$WORK/out"
mkdir -p "$OUT/lib" "$OUT/libexec"

cp src-tauri/target/release/pudu-launcher "$OUT/"
cp src-tauri/binaries/pudu-launcher-sidecar-x86_64-unknown-linux-gnu "$OUT/pudu-launcher-sidecar"
cp -a /opt/pudu-deps/lib/*.so* "$OUT/lib/"
# The platform runtime has no woff2
cp -a /usr/lib/x86_64-linux-gnu/libwoff2dec.so.* /usr/lib/x86_64-linux-gnu/libwoff2common.so.* "$OUT/lib/"
# libffi.so.7 is in sniper but not on modern hosts
cp -a /usr/lib/x86_64-linux-gnu/libffi.so.7* "$OUT/lib/"
cp -a /opt/pudu-deps/lib/webkit2gtk-4.1 "$OUT/lib/"
cp -a /opt/pudu-deps/libexec/webkit2gtk-4.1 "$OUT/libexec/"
rm -f "$OUT/libexec/webkit2gtk-4.1/MiniBrowser"
cp tools/steam-linux/run.sh "$OUT/run.sh"
chmod +x "$OUT/run.sh" "$OUT/pudu-launcher" "$OUT/pudu-launcher-sidecar"

strip --strip-unneeded "$OUT"/lib/*.so* 2>/dev/null || true
strip "$OUT/pudu-launcher" "$OUT"/libexec/webkit2gtk-4.1/WebKit* 2>/dev/null || true

# The publish pipeline goes through Windows, which breaks symlinks. Ship one real file per library.
find "$OUT/lib" -maxdepth 1 -name '*.so' -type l -delete
while IFS= read -r link; do
    target=$(readlink -f "$link")
    rm "$link"
    if [ -f "$target" ]; then
        mv "$target" "$link"
    fi
done < <(find "$OUT/lib" -maxdepth 1 -type l)
leftover=$(find "$OUT" -type l)
if [ -n "$leftover" ]; then
    echo "ERROR: symlinks remain in artifact:" >&2
    echo "$leftover" >&2
    exit 1
fi

# Relative rpaths so no env vars are needed
patchelf --set-rpath '$ORIGIN/lib' "$OUT/pudu-launcher"
for f in "$OUT"/libexec/webkit2gtk-4.1/WebKit*; do
    patchelf --set-rpath '$ORIGIN/../../lib' "$f"
done
for so in "$OUT"/lib/*.so*; do
    patchelf --set-rpath '$ORIGIN' "$so"
done
patchelf --set-rpath '$ORIGIN/../..' "$OUT/lib/webkit2gtk-4.1/injected-bundle/libwebkit2gtkinjectedbundle.so"

tar -C "$WORK/out" -czf "$WORK/out/pudu-launcher-linux-x64.tar.gz" pudu-launcher-linux-x64

mkdir -p "$SRC/dist-steam"
rm -rf "$SRC/dist-steam/pudu-launcher-linux-x64"
cp -a "$WORK/out/pudu-launcher-linux-x64" "$SRC/dist-steam/"
cp "$WORK/out/pudu-launcher-linux-x64.tar.gz" "$SRC/dist-steam/"
# The container runs as root
if [ -n "${HOST_UID:-}" ]; then
    chown -R "$HOST_UID:${HOST_GID:-$HOST_UID}" "$SRC/dist-steam"
fi
echo "ARTIFACT READY: dist-steam/pudu-launcher-linux-x64"

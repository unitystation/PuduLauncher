#!/bin/bash
set -u
cd "${ARTIFACT:-/artifact}/pudu-launcher-linux-x64"
# No LD_LIBRARY_PATH on purpose, Steam may not pass env vars to child processes
fail=0
check() {
    local miss
    miss=$(ldd "$1" 2>&1 | grep "not found")
    if [ -n "$miss" ]; then
        echo "MISSING deps for $1:"
        echo "$miss"
        fail=1
    fi
}
check pudu-launcher
check pudu-launcher-sidecar
check libexec/webkit2gtk-4.1/WebKitWebProcess
check libexec/webkit2gtk-4.1/WebKitNetworkProcess
while IFS= read -r so; do check "$so"; done < <(find lib -name '*.so*' -type f)
if [ "$fail" -eq 0 ]; then echo "ALL DEPS RESOLVED"; fi
exit "$fail"

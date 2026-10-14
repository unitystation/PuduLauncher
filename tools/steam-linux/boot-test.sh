#!/bin/bash
set -u
cd "${ARTIFACT:-/artifact}/pudu-launcher-linux-x64"
# Only use what ships in the artifact: drop the image's LD_LIBRARY_PATH and hide
# /opt/pudu-deps (WebKit falls back to it, which would hide missing files).
unset LD_LIBRARY_PATH WEBKIT_EXEC_PATH WEBKIT_INJECTED_BUNDLE_PATH
mv /opt/pudu-deps /opt/pudu-deps.hidden 2>/dev/null || true
xvfb-run -a -s "-screen 0 1366x768x24" ./run.sh > /tmp/app.log 2>&1 &
sleep 25
ok=1
pgrep -x pudu-launcher > /dev/null && echo "main process: alive" || { echo "main process: DEAD"; ok=0; }
pgrep -f WebKitWebProcess > /dev/null && echo "WebKitWebProcess: alive" || { echo "WebKitWebProcess: DEAD"; ok=0; }
pgrep -f pudu-launcher-sidecar > /dev/null && echo "sidecar: alive" || { echo "sidecar: DEAD"; ok=0; }
echo "--- app log tail ---"
tail -n 40 /tmp/app.log
pkill -x pudu-launcher 2>/dev/null
[ "$ok" -eq 1 ] && echo "BOOT TEST PASSED"
exit $((1 - ok))

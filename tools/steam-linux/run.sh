#!/bin/sh
# Steam launch option: run.sh
HERE="$(cd "$(dirname "$0")" && pwd)"
export LD_LIBRARY_PATH="$HERE/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export WEBKIT_EXEC_PATH="$HERE/libexec/webkit2gtk-4.1"
export WEBKIT_INJECTED_BUNDLE_PATH="$HERE/lib/webkit2gtk-4.1/injected-bundle"
# TLS/proxy GIO modules from the runtime
export GIO_EXTRA_MODULES="/usr/lib/x86_64-linux-gnu/gio/modules${GIO_EXTRA_MODULES:+:$GIO_EXTRA_MODULES}"
# Uncomment if the window renders black:
# export WEBKIT_DISABLE_DMABUF_RENDERER=1
# export WEBKIT_DISABLE_COMPOSITING_MODE=1
exec "$HERE/pudu-launcher" "$@"

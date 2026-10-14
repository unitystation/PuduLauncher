#!/bin/bash
# Builds the Steam Linux depot into dist-steam/.
#   --build-deps  build the deps image locally instead of pulling it (~1 h)
#   --test        then run verify-ldd.sh and boot-test.sh
set -euo pipefail

DEPS_IMAGE="${DEPS_IMAGE:-ghcr.io/unitystation/pudu-steamrt-deps:latest}"
PLATFORM_IMAGE="${PLATFORM_IMAGE:-registry.gitlab.steamos.cloud/steamrt/sniper/platform:latest}"
APP_IMAGE=pudu-steamrt-app
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TOOLS="$REPO/tools/steam-linux"

build_deps=0
run_tests=0
for arg in "$@"; do
    case "$arg" in
        --build-deps) build_deps=1 ;;
        --test) run_tests=1 ;;
        *) echo "unknown argument: $arg" >&2; exit 2 ;;
    esac
done

if [ "$build_deps" -eq 1 ]; then
    docker build -t "$DEPS_IMAGE" -f "$TOOLS/Dockerfile.deps" "$TOOLS"
elif ! docker image inspect "$DEPS_IMAGE" > /dev/null 2>&1; then
    if ! docker pull "$DEPS_IMAGE"; then
        echo "Pull failed, building deps image locally (compiles WebKit, ~1h)"
        docker build -t "$DEPS_IMAGE" -f "$TOOLS/Dockerfile.deps" "$TOOLS"
    fi
fi

docker build --build-arg "DEPS_IMAGE=$DEPS_IMAGE" -t "$APP_IMAGE" -f "$TOOLS/Dockerfile" "$TOOLS"

docker run --rm \
    -v "$REPO:/repo" \
    -v pudu-steam-work:/work \
    -v pudu-steam-cargo-registry:/root/.cargo/registry \
    -v pudu-steam-npm:/root/.npm \
    -v pudu-steam-nuget:/root/.nuget \
    -e HOST_UID="$(id -u)" \
    -e HOST_GID="$(id -g)" \
    "$APP_IMAGE" \
    bash -c "sed 's/\r$//' /repo/tools/steam-linux/container-build.sh > /tmp/cb.sh && bash /tmp/cb.sh"

if [ "$run_tests" -eq 1 ]; then
    echo "--- verify-ldd (sniper platform image) ---"
    docker run --rm \
        -v "$REPO/dist-steam:/artifact:ro" \
        -v "$TOOLS:/scripts:ro" \
        "$PLATFORM_IMAGE" \
        bash -c "sed 's/\r$//' /scripts/verify-ldd.sh > /tmp/v.sh && bash /tmp/v.sh"

    echo "--- boot test (xvfb) ---"
    docker run --rm \
        -v "$REPO/dist-steam:/artifact:ro" \
        -v "$TOOLS:/scripts:ro" \
        "$DEPS_IMAGE" \
        bash -c "sed 's/\r$//' /scripts/boot-test.sh > /tmp/b.sh && bash /tmp/b.sh"
fi

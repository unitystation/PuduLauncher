param(
    [switch]$BuildDeps,
    [string]$DepsImage = "ghcr.io/unitystation/pudu-steamrt-deps:latest"
)
$ErrorActionPreference = "Stop"
$repo = (Resolve-Path "$PSScriptRoot\..\..").Path

if ($BuildDeps) {
    docker build -t $DepsImage -f "$repo\tools\steam-linux\Dockerfile.deps" "$repo\tools\steam-linux"
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} else {
    docker image inspect $DepsImage *> $null
    if ($LASTEXITCODE -ne 0) {
        docker pull $DepsImage
        if ($LASTEXITCODE -ne 0) {
            Write-Host "Pull failed, building deps image locally (compiles WebKit, ~1h)"
            docker build -t $DepsImage -f "$repo\tools\steam-linux\Dockerfile.deps" "$repo\tools\steam-linux"
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        }
    }
}

docker build --build-arg "DEPS_IMAGE=$DepsImage" -t pudu-steamrt-app -f "$repo\tools\steam-linux\Dockerfile" "$repo\tools\steam-linux"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

docker run --rm `
    -v "${repo}:/repo" `
    -v pudu-steam-work:/work `
    -v pudu-steam-cargo-registry:/root/.cargo/registry `
    -v pudu-steam-npm:/root/.npm `
    -v pudu-steam-nuget:/root/.nuget `
    pudu-steamrt-app `
    bash -c "sed 's/\r$//' /repo/tools/steam-linux/container-build.sh > /tmp/cb.sh && bash /tmp/cb.sh"
exit $LASTEXITCODE

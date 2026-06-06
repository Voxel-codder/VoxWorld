$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$crateRoot = Resolve-Path (Join-Path $scriptDir "..")
$repoRoot = Resolve-Path (Join-Path $crateRoot "..")
$targetDir = Join-Path $repoRoot "target\wasm32-unknown-unknown"
$profile = if ($env:VOXWORLD_WEB_DEBUG -eq "1") { "debug" } else { "release" }
$wasmBindgenVersion = if ($env:WASM_BINDGEN_VERSION) { $env:WASM_BINDGEN_VERSION } else { "0.2.106" }
$outDir = Join-Path $crateRoot "web\pkg"

Push-Location $repoRoot
try {
    rustup target add wasm32-unknown-unknown | Out-Null

    $installed = (& wasm-bindgen --version 2>$null)
    if ($installed -ne "wasm-bindgen $wasmBindgenVersion") {
        cargo install wasm-bindgen-cli --version $wasmBindgenVersion --locked --force
    }

    if ($profile -eq "release") {
        cargo build --locked -p voxworld-voxygen-web --target wasm32-unknown-unknown --release
    } else {
        cargo build --locked -p voxworld-voxygen-web --target wasm32-unknown-unknown
    }

    if (Test-Path $outDir) {
        Remove-Item -Recurse -Force $outDir
    }

    wasm-bindgen `
        --target web `
        --out-dir $outDir `
        (Join-Path $targetDir "$profile\voxworld_voxygen_web.wasm")

    $js = Join-Path $outDir "voxworld_voxygen_web.js"
    $wasm = Join-Path $outDir "voxworld_voxygen_web_bg.wasm"
    if (-not (Test-Path $js)) { throw "missing generated JS: $js" }
    if (-not (Test-Path $wasm)) { throw "missing generated WASM: $wasm" }

    Write-Host "Built Voxygen web bootstrap into $outDir"
}
finally {
    Pop-Location
}

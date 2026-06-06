param(
    [string]$Profile = "dev"
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$targetDir = Join-Path $repoRoot "target\wasm32-unknown-unknown"
$outDir = Join-Path $repoRoot "web-client\web\pkg"

Push-Location $repoRoot
try {
    if ($Profile -eq "release") {
        cargo build -p voxworld-web-client --target wasm32-unknown-unknown --release
        $wasmPath = Join-Path $targetDir "release\voxworld_web_client.wasm"
    } else {
        cargo build -p voxworld-web-client --target wasm32-unknown-unknown
        $wasmPath = Join-Path $targetDir "debug\voxworld_web_client.wasm"
    }

    wasm-bindgen --target web --out-dir $outDir $wasmPath
    Write-Host "Built web client into $outDir"
} finally {
    Pop-Location
}

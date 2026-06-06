param(
    [string]$Profile = "dev",
    [string]$WasmBindgenVersion = "0.2.106"
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$targetDir = Join-Path $repoRoot "target\wasm32-unknown-unknown"
$outDir = Join-Path $repoRoot "web-client\web\pkg"
$installedWasmBindgenVersion = $null

try {
    $installedWasmBindgenVersion = (& wasm-bindgen --version 2>$null)
} catch {
    $installedWasmBindgenVersion = $null
}

if ($installedWasmBindgenVersion -ne "wasm-bindgen $WasmBindgenVersion") {
    cargo install wasm-bindgen-cli --version $WasmBindgenVersion --locked --force
}

Push-Location $repoRoot
try {
    if ($Profile -eq "release") {
        cargo build -p voxworld-web-client --target wasm32-unknown-unknown --release
        $wasmPath = Join-Path $targetDir "release\voxworld_web_client.wasm"
    } else {
        cargo build -p voxworld-web-client --target wasm32-unknown-unknown
        $wasmPath = Join-Path $targetDir "debug\voxworld_web_client.wasm"
    }

    if (Test-Path $outDir) {
        Remove-Item -Recurse -Force $outDir
    }

    wasm-bindgen --target web --out-dir $outDir $wasmPath
    $jsPath = Join-Path $outDir "voxworld_web_client.js"
    $wasmOutPath = Join-Path $outDir "voxworld_web_client_bg.wasm"
    if (!(Test-Path $jsPath) -or !(Test-Path $wasmOutPath)) {
        throw "wasm-bindgen did not produce the expected web-client package files"
    }
    Write-Host "Built web client into $outDir"
} finally {
    Pop-Location
}

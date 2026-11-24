#Requires -Version 7.0
$ErrorActionPreference = 'Stop'

Push-Location -LiteralPath (Join-Path $PSScriptRoot '..')
try {
    $output = 'target/dist/THIRD_PARTY_LICENSES.html'
    New-Item -ItemType Directory -Force (Split-Path $output) | Out-Null
    cargo +stable about generate --locked --fail --output-file $output packaging/about.hbs
    if ($LASTEXITCODE -ne 0) {
        throw "cargo-about failed with exit code $LASTEXITCODE"
    }
    Write-Output (Resolve-Path -LiteralPath $output).Path
} finally {
    Pop-Location
}

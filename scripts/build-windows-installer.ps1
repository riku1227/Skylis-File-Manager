#Requires -Version 7.0
$ErrorActionPreference = 'Stop'

if (!$IsWindows) {
    throw 'Windows is required to build the Skylis installer.'
}
if (!(Get-Command cargo -CommandType Application -ErrorAction SilentlyContinue)) {
    throw 'Install Rust stable with rustup (https://rustup.rs) and add Cargo to PATH.'
}

$iscc = (Get-Command ISCC.exe -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1).Source
if (!$iscc) {
    foreach ($base in @(${env:ProgramFiles(x86)}, $env:ProgramFiles, (Join-Path $env:LOCALAPPDATA 'Programs'))) {
        if ($base) {
            $candidate = Join-Path $base 'Inno Setup 6/ISCC.exe'
            if (Test-Path -LiteralPath $candidate -PathType Leaf) {
                $iscc = $candidate
                break
            }
        }
    }
}
if (!$iscc) {
    throw 'ISCC.exe was not found. Install Inno Setup 6.3 or later (winget install JRSoftware.InnoSetup).'
}

Push-Location -LiteralPath (Join-Path $PSScriptRoot '..')
try {
    cargo +stable about --version | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw 'Install cargo-about with: cargo +stable install --locked cargo-about --version 0.9.3 --features cli'
    }

    $metadataJson = cargo metadata --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed with exit code $LASTEXITCODE."
    }
    $metadata = $metadataJson | ConvertFrom-Json
    # explorer inherits its version from [workspace.package] in the root Cargo.toml.
    $version = ($metadata.packages | Where-Object name -eq 'explorer').version
    if ($version -cnotmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$') {
        throw 'Could not read the Skylis workspace version.'
    }
    $numericVersion = ($version -split '[-+]', 2)[0]

    $rustTarget = 'x86_64-pc-windows-msvc'
    cargo build --locked --release -p explorer --target $rustTarget
    if ($LASTEXITCODE -ne 0) {
        throw "Skylis release build failed with exit code $LASTEXITCODE."
    }
    $releaseExe = Join-Path $metadata.target_directory "$rustTarget/release/skylis.exe"
    if (!(Test-Path -LiteralPath $releaseExe -PathType Leaf)) {
        throw "The Skylis release executable was not found at $releaseExe."
    }

    $licenseNotices = & (Join-Path $PSScriptRoot 'generate-third-party-licenses.ps1')
    if (!(Test-Path -LiteralPath $licenseNotices -PathType Leaf)) {
        throw 'The third-party license notices could not be located.'
    }

    & $iscc "/DAppVersion=$version" "/DNumericVersion=$numericVersion" "/DReleaseExe=$releaseExe" "/DLicenseNotices=$licenseNotices" 'packaging/windows/skylis.iss'
    if ($LASTEXITCODE -ne 0) {
        throw "Inno Setup compilation failed with exit code $LASTEXITCODE. Inno Setup 6.3 or later is required."
    }

    $installer = Join-Path $PWD "target/installer/Skylis-$version-x64-setup.exe"
    Write-Output (Resolve-Path -LiteralPath $installer).Path
} finally {
    Pop-Location
}

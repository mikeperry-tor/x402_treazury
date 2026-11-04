# Native Windows equivalents of the named Make validation commands. This is
# ordinary unfunded validation, not release reproducibility or live qualification.
param([Parameter(Mandatory = $true)][ValidateSet('Release', 'Checks')][string]$Stage)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
if (-not $IsWindows) { throw 'This entrypoint requires native Windows PowerShell 7' }
Set-Location (Split-Path -Parent $PSScriptRoot)

function Invoke-Checked([string]$Command, [string[]]$Arguments) {
    Write-Host ('> ' + $Command + ' ' + ($Arguments -join ' '))
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Command failed with exit code $LASTEXITCODE" }
}

$pin = (Select-String -Path rust-toolchain.toml -Pattern '^channel = "([^"]+)"$').Matches.Groups[1].Value
if (-not $pin) { throw 'Missing pinned Rust toolchain' }
$env:RUSTUP_TOOLCHAIN = $pin
foreach ($tool in @('rustc', 'cargo')) {
    $version = & $tool --version
    if ($LASTEXITCODE -ne 0 -or $version -notlike "$tool $pin *") {
        throw "Toolchain mismatch for $tool; expected $pin, found $version"
    }
    Write-Host $version
}
$details = & rustc -vV
if ($LASTEXITCODE -ne 0 -or $details -notcontains 'host: x86_64-pc-windows-msvc') {
    throw 'Native x86_64-pc-windows-msvc Rust toolchain required'
}
if ($env:CARGO_BUILD_TARGET) {
    throw 'Use the native host default; unset CARGO_BUILD_TARGET for this entrypoint'
}
Write-Host ($details -join "`n")
Invoke-Checked 'git' @('rev-parse', 'HEAD')
foreach ($path in @('Cargo.lock', 'rust-toolchain.toml', 'tools/protoc/Cargo.lock')) {
    Write-Host ($path + ' SHA256 ' + (Get-FileHash -Algorithm SHA256 $path).Hash)
}
Write-Host ('Windows image ' + $env:ImageOS + ' ' + $env:ImageVersion)
Invoke-Checked 'python' @('--version')

# Bundled protoc is independently locked and built for the host, never replaced
# by a PATH/system protoc. Compiler warnings remain visible on stderr.
$protoc = & cargo run --quiet --locked --manifest-path tools/protoc/Cargo.toml
if ($LASTEXITCODE -ne 0) { throw 'Locked protoc helper failed' }
$env:PROTOC = ($protoc | Out-String).Trim()
if (-not (Test-Path -LiteralPath $env:PROTOC -PathType Leaf)) {
    throw 'Locked protoc helper did not return an existing host executable'
}
Invoke-Checked $env:PROTOC @('--version')

if ($Stage -eq 'Release') {
    $env:CARGO_INCREMENTAL = '0'
    Invoke-Checked 'cargo' @('build', '--locked', '--manifest-path', 'Cargo.toml', '--profile', 'release')
    $metadata = & cargo metadata --locked --manifest-path Cargo.toml --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Cannot determine Cargo artifact directory' }
    $targetDirectory = ($metadata | ConvertFrom-Json).target_directory
    if (-not $targetDirectory) { throw 'Cargo metadata omitted its target directory' }
    $binary = Join-Path $targetDirectory 'release/x402_treazury.exe'
    Invoke-Checked $binary @('--help')
    Invoke-Checked $binary @('wallet', '--help')
    Invoke-Checked $binary @('catalog', '--help')
    Write-Host ('Native executable SHA256 ' + (Get-FileHash -Algorithm SHA256 $binary).Hash)
    return
}

# Git for Windows already includes GNU patch, needed by the existing byte-exact
# vendor reconstruction checks. Fail explicitly if that prerequisite is absent.
$gitRoot = Split-Path -Parent (Split-Path -Parent (Get-Command git).Source)
$gitTools = Join-Path $gitRoot 'usr/bin'
if (-not (Get-Command patch -ErrorAction SilentlyContinue)) {
    $env:PATH = $gitTools + [IO.Path]::PathSeparator + $env:PATH
}
Invoke-Checked 'patch' @('--version')
Invoke-Checked 'cargo' @('fmt', '--check')
Invoke-Checked 'python' @('vendor/verify.py')
Invoke-Checked 'python' @('vendor/verify_pepper.py')
# Keep the same production and full all-target warning owners as Make. No
# dependency warnings, features or failing native tests are silently filtered.
Invoke-Checked 'python' @('scripts/check_warnings.py', 'cargo', 'check', '--locked', '--manifest-path', 'Cargo.toml', '--lib', '--bins', '--message-format=json')
Invoke-Checked 'python' @('scripts/check_warnings.py', 'cargo', 'clippy', '--locked', '--manifest-path', 'Cargo.toml', '--all-targets', '--features', 'zcash-regtest', '--message-format=json', '--', '-D', 'warnings')
Invoke-Checked 'cargo' @('test', '--locked', '--manifest-path', 'Cargo.toml', '--test', 'sdk_compat', '--test', 'payments', '--test', 'server', '--test', 'crypto_vectors', '--', '--test-threads=8')
Invoke-Checked 'python' @('scripts/default_targets.py', '--compat', 'sdk_compat', '--compat', 'payments', '--compat', 'server', '--compat', 'crypto_vectors', '--', 'cargo', 'test', '--locked', '--manifest-path', 'Cargo.toml', '--', '--test-threads=8')
Invoke-Checked 'cargo' @('test', '--locked', '--manifest-path', 'Cargo.toml', '--features', 'zcash-testutils', '--lib', 'treasury::send::proving_tests', '--', '--test-threads=1')
Invoke-Checked 'cargo' @('test', '--locked', '--manifest-path', 'Cargo.toml', '--features', 'zcash-testutils', '--lib', 'treasury::lifecycle_tests', '--', '--test-threads=8')
Invoke-Checked 'cargo' @('test', '--locked', '--manifest-path', 'Cargo.toml', '--features', 'zcash-testutils', '--lib', 'synthetic_address_codec_is_scoped_to_its_test_client', '--', '--test-threads=1')

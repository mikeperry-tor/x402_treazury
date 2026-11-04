# Unfunded native compatibility evidence. No installation or power-loss claim.
param([switch]$Child)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
Set-StrictMode -Version Latest
if (-not $IsWindows -or [IntPtr]::Size -ne 8) { throw 'Native x64 Windows PowerShell 7 is required' }
$repository = Split-Path -Parent $PSScriptRoot
function Field($Value, [string]$Name, $Fallback) {
    if ($null -ne $Value -and $Value.ContainsKey($Name)) { return $Value[$Name] }
    return $Fallback
}
function Emit-Notice($Summary) {
    $text = $Summary | ConvertTo-Json -Depth 12 -Compress
    $text = $text.Replace('%', '%25').Replace("`r", '%0D').Replace("`n", '%0A')
    if ($env:GITHUB_ACTIONS -eq 'true') {
        [Console]::Out.WriteLine('::notice title=Native namespace probe (compatibility only)::' + $text)
    } else {
        [Console]::Error.WriteLine('Native namespace probe summary: ' + $text)
    }
}
if (-not $Child) {
    # Checked reversion failure can terminate only this isolated helper process.
    $pwsh = (Get-Process -Id $PID).Path
    $output = & $pwsh -NoProfile -NonInteractive -File $PSCommandPath -Child
    $result = $LASTEXITCODE
    $json = $output -join "`n"
    [Console]::Out.WriteLine($json)
    try {
        $report = $json | ConvertFrom-Json -AsHashtable
        $candidates = @(foreach ($candidate in $report.candidates) {
            $original = Field $candidate 'original_identity' @{}
            $final = Field $candidate 'final_identity' @{}
            $filesystem = Field $candidate 'filesystem' @{}
            @{
                candidate = $candidate.candidate
                complete = $candidate.complete
                failed_stage = Field $candidate 'failed_stage' 'none'
                error_code = Field $candidate 'error_code' 0
                fixed_local_ntfs = ((Field $filesystem 'filesystem' '') -eq 'NTFS' -and (Field $filesystem 'drive_type' 0) -eq 3)
                original_volume_id = Field $original 'volume_id' 0
                final_volume_id = Field $final 'volume_id' 0
                original_file_id = Field $original 'file_id' ''
                final_file_id = Field $final 'file_id' ''
                exact_bytes = Field $candidate 'exact_public_bytes' $false
                collision_preserved = Field $candidate 'collision_preserved_both_originals' $false
                stages = $candidate.stages
            }
        })
        Emit-Notice @{
            schema = 1
            complete = $report.complete
            power_loss_qualified = $false
            child_exit = $result
            git_sha = Field $report 'git_sha' 'unavailable'
            windows_version = Field $report 'windows_version' 'unavailable'
            dotnet_version = Field $report 'dotnet_version' 'unavailable'
            powershell_version = Field $report 'powershell' 'unavailable'
            hosted_image_version = Field $report 'hosted_image_version' 'unavailable'
            script_sha256 = Field (Field $report 'source_sha256' @{}) 'scripts/windows-namespace-probe.ps1' 'unavailable'
            native_source_sha256 = Field (Field $report 'source_sha256' @{}) 'scripts/windows-namespace-probe.cs' 'unavailable'
            workflow_sha256 = Field (Field $report 'source_sha256' @{}) '.github/workflows/windows-namespace-probe.yml' 'unavailable'
            ordinary_rights_verified = Field $report 'ordinary_rights_verified' $false
            owner_control_read = Field $report 'restricted_owner_control_read' $false
            original_admin_control_read = Field $report 'original_admin_control_read' $false
            restricted_admin_control_error = Field $report 'restricted_admin_control_error' 0
            admin_contrast = if ((Field $report 'original_admin_control_read' $false) -and
                (Field $report 'restricted_admin_control_error' 0) -eq 5) {
                'required_and_exercised'
            } else { Field $report 'admin_contrast' 'not_established' }
            owned_fixture_cleanup = Field $report 'owned_fixture_cleanup' $false
            failed_stage = Field $report 'failed_stage' 'none'
            error_code = Field $report 'error_code' 0
            candidates = $candidates
        }
        if ($result -ne 0) { exit $result }
        if ($report.schema -ne 1 -or -not $report.complete -or $report.candidates.Count -ne 2 -or
            -not $report.ordinary_rights_verified -or -not $report.restricted_owner_control_read -or
            -not ($report.candidates | Where-Object { $_.complete })) {
            throw 'Native namespace helper did not return a complete verified report'
        }
    } catch {
        Emit-Notice @{ schema = 1; complete = $false; child_exit = $result; failed_stage = 'probe_compilation_or_report_failed'; power_loss_qualified = $false }
        exit 1
    }
    exit 0
}

# Add-Type uses the .NET compiler bundled with the existing PowerShell runtime.
# No NuGet/npm/Cargo package resolution or downloaded helper executable.
Add-Type -Path (Join-Path $PSScriptRoot 'windows-namespace-probe.cs')
$report = [WindowsNamespaceProbe]::Run([IO.Path]::GetTempPath())
$report['powershell'] = $PSVersionTable.PSVersion.ToString()
$report['dotnet'] = [Runtime.InteropServices.RuntimeInformation]::FrameworkDescription
$report['os'] = [Runtime.InteropServices.RuntimeInformation]::OSDescription
$report['windows_version'] = [Environment]::OSVersion.Version.ToString()
$report['dotnet_version'] = [Environment]::Version.ToString()
$report['hosted_image_version'] = if ([string]$env:ImageVersion -match '^[0-9.]+$') { [string]$env:ImageVersion } else { 'unavailable' }
$report['process_architecture'] = [Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
$report['hosted_image'] = [string]$env:ImageOS + ' ' + [string]$env:ImageVersion
$head = & git -C $repository rev-parse HEAD
if ($LASTEXITCODE -ne 0) { throw 'Cannot identify exact probe Git source' }
$report['git_sha'] = ($head | Out-String).Trim()
$report['source_sha256'] = @{}
foreach ($name in @('scripts/windows-namespace-probe.ps1', 'scripts/windows-namespace-probe.cs', '.github/workflows/windows-namespace-probe.yml')) {
    $report.source_sha256[$name] = (Get-FileHash -Algorithm SHA256 (Join-Path $repository $name)).Hash
}
$report | ConvertTo-Json -Depth 12 -Compress
if (-not $report.complete) { exit 1 }

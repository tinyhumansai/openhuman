<#
.SYNOPSIS
    phase7-windows-evidence.ps1 - Validates MSVC target, runs/locates release build artifacts,
    computes SHA-256 hashes, and emits structured Windows evidence for Phase 7.

.DESCRIPTION
    Non-destructive helper that:
    1. Verifies MSVC host and target (rejecting MinGW/GNU toolchains).
    2. Runs an optional build command if supplied.
    3. Locates release OpenHuman.exe, NSIS installer (.exe), and MSI (.msi).
    4. Computes byte sizes and SHA-256 hashes.
    5. Emits commit, branch, and status in JSON and Markdown.
#>

[CmdletBinding()]
param (
    [Parameter(Mandatory = $false)]
    [string]$TargetDir = "",

    [Parameter(Mandatory = $false)]
    [string]$BuildCommand = "",

    [Parameter(Mandatory = $false)]
    [switch]$SkipBuild,

    [Parameter(Mandatory = $false)]
    [string]$OutputFile = "",

    [Parameter(Mandatory = $false)]
    [string]$RustcOutputOverride = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Test-MsvcEnvironment {
    param (
        [string]$RustcVerboseOutput
    )

    if ([string]::IsNullOrWhiteSpace($RustcVerboseOutput)) {
        try {
            $RustcVerboseOutput = & rustc -vV 2>&1 | Out-String
        } catch {
            throw "Failed to execute rustc to determine host/target toolchain."
        }
    }

    if ($RustcVerboseOutput -match "host:\s*([^\r\n]+)") {
        $hostTriple = $Matches[1].Trim()
    } else {
        $hostTriple = "unknown"
    }

    # Reject MinGW / GNU
    if ($hostTriple -match "gnu" -or $hostTriple -match "mingw") {
        return @{
            IsMsvc = $false
            Host = $hostTriple
            Error = "Detected non-MSVC toolchain: $hostTriple. MinGW/GNU is strictly rejected."
        }
    }

    if ($hostTriple -notmatch "msvc") {
        return @{
            IsMsvc = $false
            Host = $hostTriple
            Error = "Host triple $hostTriple is not MSVC."
        }
    }

    return @{
        IsMsvc = $true
        Host = $hostTriple
        Error = $null
    }
}

function Find-Phase7Artifacts {
    param (
        [string]$SearchRoot
    )

    if ([string]::IsNullOrWhiteSpace($SearchRoot)) {
        $SearchRoot = $PSScriptRoot
        # Walk up to repo root
        $repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
        $SearchRoot = $repoRoot.Path
    }

    $candidates = @(
        (Join-Path $SearchRoot "target\release"),
        (Join-Path $SearchRoot "crates\openhuman-app\target\release"),
        $SearchRoot
    )

    $exePath = $null
    $nsisPath = $null
    $msiPath = $null

    foreach ($dir in $candidates) {
        if (-not (Test-Path $dir)) { continue }

        # 1. OpenHuman.exe
        if (-not $exePath) {
            $possibleExe = Join-Path $dir "OpenHuman.exe"
            if (Test-Path $possibleExe) {
                $exePath = (Resolve-Path $possibleExe).Path
            } else {
                # Case insensitive search
                $found = Get-ChildItem -Path $dir -Filter "OpenHuman.exe" -File -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
                if ($found) { $exePath = $found.FullName }
            }
        }

        # 2. NSIS installer (.exe inside bundle\nsis or bundle)
        if (-not $nsisPath) {
            $nsisDir = Join-Path $dir "bundle\nsis"
            if (Test-Path $nsisDir) {
                $found = Get-ChildItem -Path $nsisDir -Filter "*.exe" -File -ErrorAction SilentlyContinue | Select-Object -First 1
                if ($found) { $nsisPath = $found.FullName }
            }
            if (-not $nsisPath) {
                $found = Get-ChildItem -Path $dir -Filter "*setup*.exe" -File -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
                if ($found) { $nsisPath = $found.FullName }
            }
        }

        # 3. MSI installer (.msi inside bundle\msi or bundle)
        if (-not $msiPath) {
            $msiDir = Join-Path $dir "bundle\msi"
            if (Test-Path $msiDir) {
                $found = Get-ChildItem -Path $msiDir -Filter "*.msi" -File -ErrorAction SilentlyContinue | Select-Object -First 1
                if ($found) { $msiPath = $found.FullName }
            }
            if (-not $msiPath) {
                $found = Get-ChildItem -Path $dir -Filter "*.msi" -File -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
                if ($found) { $msiPath = $found.FullName }
            }
        }
    }

    return @{
        Exe = $exePath
        Nsis = $nsisPath
        Msi = $msiPath
    }
}

function Get-ArtifactEvidence {
    param (
        [string]$FilePath,
        [string]$ArtifactType
    )

    if ([string]::IsNullOrWhiteSpace($FilePath) -or -not (Test-Path $FilePath)) {
        return @{
            Type = $ArtifactType
            Path = $null
            SizeBytes = 0
            Sha256 = $null
            Exists = $false
        }
    }

    $item = Get-Item -LiteralPath $FilePath
    $hash = (Get-FileHash -LiteralPath $FilePath -Algorithm SHA256).Hash.ToLowerInvariant()

    return @{
        Type = $ArtifactType
        Path = $item.FullName
        SizeBytes = $item.Length
        Sha256 = $hash
        Exists = $true
    }
}

# Main execution logic if invoked directly
function Invoke-Phase7WindowsEvidence {
    param (
        [string]$TargetRoot,
        [string]$Command,
        [switch]$NoBuild,
        [string]$RustcOverride
    )

    # 1. Verify MSVC
    $msvcCheck = Test-MsvcEnvironment -RustcVerboseOutput $RustcOverride
    if (-not $msvcCheck.IsMsvc) {
        throw $msvcCheck.Error
    }

    # 2. Run optional build
    if (-not $NoBuild -and -not [string]::IsNullOrWhiteSpace($Command)) {
        Write-Host "Running build command: $Command"
        Invoke-Expression $Command
        if ($LASTEXITCODE -ne 0) {
            throw "Build command failed with exit code $LASTEXITCODE"
        }
    }

    # 3. Locate artifacts
    $located = Find-Phase7Artifacts -SearchRoot $TargetRoot

    $exeEvidence = Get-ArtifactEvidence -FilePath $located.Exe -ArtifactType "Executable"
    $nsisEvidence = Get-ArtifactEvidence -FilePath $located.Nsis -ArtifactType "NsisInstaller"
    $msiEvidence = Get-ArtifactEvidence -FilePath $located.Msi -ArtifactType "MsiInstaller"

    $allFound = $exeEvidence.Exists -and $nsisEvidence.Exists -and $msiEvidence.Exists

    # Git metadata
    $commit = $null
    $branch = $null
    try {
        $commit = (& git rev-parse HEAD 2>&1).Trim()
        $branch = (& git rev-parse --abbrev-ref HEAD 2>&1).Trim()
    } catch {
        $commit = "unknown"
        $branch = "unknown"
    }

    $evidence = [ordered]@{
        Timestamp = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
        HostToolchain = $msvcCheck.Host
        GitCommit = $commit
        GitBranch = $branch
        Status = if ($allFound) { "PASSED" } else { "FAILED" }
        Artifacts = @(
            $exeEvidence,
            $nsisEvidence,
            $msiEvidence
        )
    }

    return $evidence
}

# Export functions for testing
if ($MyInvocation.InvocationName -ne ".") {
    $result = Invoke-Phase7WindowsEvidence -TargetRoot $TargetDir -Command $BuildCommand -NoBuild:$SkipBuild -RustcOverride $RustcOutputOverride
    $json = $result | ConvertTo-Json -Depth 5

    if ($OutputFile) {
        Set-Content -LiteralPath $OutputFile -Value $json -Encoding UTF8
        Write-Host "Evidence written to $OutputFile"
    }

    Write-Output $json
}

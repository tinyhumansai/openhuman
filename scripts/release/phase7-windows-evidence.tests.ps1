# scripts/release/phase7-windows-evidence.tests.ps1
# Unit tests for phase7-windows-evidence.ps1 using temporary directories and fake files.
# Does not build or contact a network.

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptPath = Join-Path $PSScriptRoot "phase7-windows-evidence.ps1"
if (-not (Test-Path $scriptPath)) {
    throw "Script not found: $scriptPath"
}

# Dot-source the functions
. $scriptPath

$passedTests = 0
$failedTests = 0

function Assert-True {
    param ([bool]$Condition, [string]$Message)
    if (-not $Condition) {
        Write-Error "ASSERTION FAILED: $Message"
        $script:failedTests++
    } else {
        Write-Host "  PASS: $Message" -ForegroundColor Green
        $script:passedTests++
    }
}

function Assert-Equal {
    param ($Actual, $Expected, [string]$Message)
    if ($Actual -ne $Expected) {
        Write-Error "ASSERTION FAILED: $Message. Expected: '$Expected', Actual: '$Actual'"
        $script:failedTests++
    } else {
        Write-Host "  PASS: $Message" -ForegroundColor Green
        $script:passedTests++
    }
}

Write-Host "Running phase7-windows-evidence tests..." -ForegroundColor Cyan

# Test 1: Rejects MinGW/GNU toolchain
$gnuOutput = @"
rustc 1.85.0 (4d91de4e4 2025-02-17)
binary: rustc
commit-hash: 4d91de4e48198da2e33413efdcd9cd2cc0c46688
commit-date: 2025-02-17
host: x86_64-pc-windows-gnu
release: 1.85.0
LLVM version: 19.1.7
"@

$gnuResult = Test-MsvcEnvironment -RustcVerboseOutput $gnuOutput
Assert-Equal $gnuResult.IsMsvc $false "Rejects x86_64-pc-windows-gnu host"
Assert-True ($gnuResult.Error -match "MinGW/GNU is strictly rejected") "Contains MinGW rejection message"

# Test 2: Accepts MSVC toolchain
$msvcOutput = @"
rustc 1.85.0 (4d91de4e4 2025-02-17)
binary: rustc
commit-hash: 4d91de4e48198da2e33413efdcd9cd2cc0c46688
commit-date: 2025-02-17
host: x86_64-pc-windows-msvc
release: 1.85.0
LLVM version: 19.1.7
"@

$msvcResult = Test-MsvcEnvironment -RustcVerboseOutput $msvcOutput
Assert-Equal $msvcResult.IsMsvc $true "Accepts x86_64-pc-windows-msvc host"
Assert-Equal $msvcResult.Host "x86_64-pc-windows-msvc" "Records MSVC host"

# Test 3: Artifact discovery and hashing with fake files
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("phase7_evidence_test_" + [System.Guid]::NewGuid().ToString("N"))
try {
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
    $releaseDir = Join-Path $tempDir "target\release"
    $nsisDir = Join-Path $releaseDir "bundle\nsis"
    $msiDir = Join-Path $releaseDir "bundle\msi"

    New-Item -ItemType Directory -Path $nsisDir -Force | Out-Null
    New-Item -ItemType Directory -Path $msiDir -Force | Out-Null

    # Create fake artifacts with known content
    $exeFile = Join-Path $releaseDir "OpenHuman.exe"
    $nsisFile = Join-Path $nsisDir "OpenHuman_0.63.25_x64-setup.exe"
    $msiFile = Join-Path $msiDir "OpenHuman_0.63.25_x64_en-US.msi"

    Set-Content -Path $exeFile -Value "fake-binary-openhuman-exe-content" -Encoding UTF8
    Set-Content -Path $nsisFile -Value "fake-nsis-installer-content" -Encoding UTF8
    Set-Content -Path $msiFile -Value "fake-msi-installer-content" -Encoding UTF8

    $located = Find-Phase7Artifacts -SearchRoot $tempDir
    Assert-Equal (Test-Path $located.Exe) $true "Discovers fake OpenHuman.exe"
    Assert-Equal (Test-Path $located.Nsis) $true "Discovers fake NSIS installer"
    Assert-Equal (Test-Path $located.Msi) $true "Discovers fake MSI installer"

    $exeEvidence = Get-ArtifactEvidence -FilePath $located.Exe -ArtifactType "Executable"
    Assert-Equal $exeEvidence.Exists $true "Executable evidence Exists is true"
    Assert-True ($exeEvidence.SizeBytes -gt 0) "Executable size is greater than 0"
    Assert-True (-not [string]::IsNullOrEmpty($exeEvidence.Sha256)) "Executable SHA-256 is computed"

    # Test 4: End-to-end Invoke-Phase7WindowsEvidence on fake directory
    $evidence = Invoke-Phase7WindowsEvidence -TargetRoot $tempDir -NoBuild -RustcOverride $msvcOutput
    Assert-Equal $evidence.Status "PASSED" "Status is PASSED when all 3 artifacts exist"
    Assert-Equal $evidence.Artifacts.Count 3 "Contains exactly 3 artifact entries"
    Assert-Equal $evidence.HostToolchain "x86_64-pc-windows-msvc" "Emits MSVC host toolchain"

    # Test 5: Missing artifact reports FAILED
    Remove-Item -Path $exeFile -Force
    $evidenceMissing = Invoke-Phase7WindowsEvidence -TargetRoot $tempDir -NoBuild -RustcOverride $msvcOutput
    Assert-Equal $evidenceMissing.Status "FAILED" "Status is FAILED when OpenHuman.exe is missing"
} finally {
    if (Test-Path $tempDir) {
        Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Write-Host "`nTest Summary: $passedTests passed, $failedTests failed."
if ($failedTests -gt 0) {
    exit 1
} else {
    exit 0
}

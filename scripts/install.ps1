#!/usr/bin/env pwsh
<#
.SYNOPSIS
  OpenHuman installer for Windows.

.DESCRIPTION
  Intended for:
  irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
#>

param(
  [switch]$Help,
  [switch]$Version,
  [string]$Channel = "stable",
  [switch]$DryRun
)

$ErrorActionPreference = "Stop"

$InstallerVersion = "1.0.0"
$Repo = "tinyhumansai/openhuman"
$LatestJsonUrl = "https://github.com/$Repo/releases/latest/download/latest.json"
$LatestReleaseApiUrl = "https://api.github.com/repos/$Repo/releases/latest"

function Write-Info([string]$Message) { Write-Host "-> $Message" -ForegroundColor Cyan }
function Write-Ok([string]$Message) { Write-Host "OK $Message" -ForegroundColor Green }
function Write-WarnMsg([string]$Message) { Write-Host "!  $Message" -ForegroundColor Yellow }
function Write-Err([string]$Message) { Write-Host "x  $Message" -ForegroundColor Red }

function Show-Usage {
  @"
OpenHuman Installer (Windows)

Usage:
  install.ps1 [-Channel stable] [-DryRun] [-Help] [-Version]

Examples:
  irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
  .\scripts\install.ps1 -DryRun
"@
}

if ($Help) {
  Show-Usage
  exit 0
}

if ($Version) {
  Write-Output "openhuman-installer $InstallerVersion"
  exit 0
}

if ($Channel -ne "stable") {
  Write-Err "Only -Channel stable is currently supported."
  exit 1
}

if ($env:OS -ne "Windows_NT") {
  Write-Err "This installer is for Windows only."
  exit 1
}

$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
if ($arch -notin @("x64", "amd64")) {
  Write-Err "Unsupported architecture: $arch (Windows x64 required)."
  exit 1
}

Write-Ok "Detected platform: windows/x64"

$release = $null
$releaseTag = ""
$assetName = ""
$assetUrl = ""
$assetDigest = ""

function Select-WindowsAssetFromRelease([object]$Rel) {
  $assets = @($Rel.assets)
  if (-not $assets -or $assets.Count -eq 0) {
    return $null
  }

  $msi = $assets | Where-Object { $_.name -match 'OpenHuman_.*x64.*\.msi$' } | Select-Object -First 1
  if ($msi) { return $msi }

  $exe = $assets | Where-Object { $_.name -match 'OpenHuman_.*x64.*\.exe$' } | Select-Object -First 1
  if ($exe) { return $exe }

  return $null
}

try {
  $release = Invoke-RestMethod -Uri $LatestReleaseApiUrl
  $releaseTag = ($release.tag_name -replace '^v', '')
  $selected = Select-WindowsAssetFromRelease -Rel $release
  if ($selected) {
    $assetName = $selected.name
    $assetUrl = $selected.browser_download_url
    if ($selected.digest) {
      $assetDigest = ($selected.digest -replace '^sha256:', '')
    }
  }
} catch {
  Write-WarnMsg "Could not query release API: $($_.Exception.Message)"
}

if (-not $assetUrl) {
  Write-Err "No Windows x64 installer artifact found in latest release."
  Write-Err "Ensure release workflow publishes Windows MSI/EXE assets."
  exit 1
}

Write-Ok "Resolved latest release ($releaseTag): $assetName"

$tmpFile = Join-Path $env:TEMP $assetName
if ($DryRun) {
  Write-Output "DRY RUN: download $assetUrl -> $tmpFile"
} else {
  Write-Info "Downloading $assetName"
  Invoke-WebRequest -Uri $assetUrl -OutFile $tmpFile
}

if ($assetDigest) {
  if ($DryRun) {
    Write-Output "DRY RUN: verify SHA256 $assetDigest"
  } else {
    $fileHash = (Get-FileHash -Path $tmpFile -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($fileHash -ne $assetDigest.ToLowerInvariant()) {
      Write-Err "SHA256 mismatch for $assetName"
      Write-Err "Expected: $assetDigest"
      Write-Err "Actual:   $fileHash"
      exit 1
    }
    Write-Ok "Integrity verified (sha256)"
  }
} else {
  Write-WarnMsg "No SHA256 digest available for $assetName; skipping integrity verification."
}

if ($DryRun) {
  if ($assetName -like "*.msi") {
    Write-Output "DRY RUN: msiexec /i `"$tmpFile`" MSIINSTALLPERUSER=1 ALLUSERS=2 /qn /norestart"
  } else {
    Write-Output "DRY RUN: Start-Process `"$tmpFile`" -Wait"
  }
  exit 0
}

Write-Info "Installing OpenHuman"
if ($assetName -like "*.msi") {
  $args = "/i `"$tmpFile`" MSIINSTALLPERUSER=1 ALLUSERS=2 /qn /norestart"
  $proc = Start-Process -FilePath "msiexec.exe" -ArgumentList $args -Wait -PassThru
  if ($proc.ExitCode -ne 0) {
    Write-Err "MSI install failed with exit code $($proc.ExitCode)."
    exit 1
  }
} elseif ($assetName -like "*.exe") {
  $proc = Start-Process -FilePath $tmpFile -Wait -PassThru
  if ($proc.ExitCode -ne 0) {
    Write-Err "Installer exited with code $($proc.ExitCode)."
    exit 1
  }
} else {
  Write-Err "Unsupported Windows installer type: $assetName"
  exit 1
}

$expectedPaths = @(
  "$env:LOCALAPPDATA\Programs\OpenHuman\OpenHuman.exe",
  "$env:ProgramFiles\OpenHuman\OpenHuman.exe"
)
$launchPath = $expectedPaths | Where-Object { Test-Path $_ } | Select-Object -First 1

Write-Output ""
Write-Output "OpenHuman is ready."
if ($launchPath) {
  Write-Output "Launch: `"$launchPath`""
  Write-Output "Uninstall: Settings -> Apps -> Installed apps -> OpenHuman"
} else {
  Write-WarnMsg "Could not locate installed executable automatically."
  Write-Output "Try launching OpenHuman from Start Menu."
  Write-Output "Uninstall: Settings -> Apps -> Installed apps -> OpenHuman"
}

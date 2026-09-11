#!/usr/bin/env pwsh
<#
.SYNOPSIS
  OpenHuman installer for Windows.

.DESCRIPTION
  Intended for:
  irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex

  Also works when saved and run directly:
  .\scripts\install.ps1 -DryRun

  MSI installs use the Tauri WiX package (InstallScope perMachine). Per-user
  public properties (MSIINSTALLPERUSER / ALLUSERS=2) conflict with that layout
  and commonly fail with exit 1603 — see tinyhumansai/openhuman#913.

  When the current session is not elevated, msiexec is started with -Verb RunAs
  so Windows shows UAC once (machine install to Program Files).
#>

# --- Script-scoped helpers (unit-tested; safe to dot-source this file) ---

function Get-OpenHumanMsiexecInstallArgumentList {
  <#
  .SYNOPSIS
    Argument list for Start-Process msiexec.exe (no per-user MSI overrides).
  #>
  param(
    [Parameter(Mandatory = $true)]
    [string]$MsiPath
  )
  # Start-Process joins ArgumentList entries into one command line, so preserve spaces by
  # quoting the MSI path explicitly before it is passed to msiexec.
  return @('/i', ('"{0}"' -f $MsiPath), '/qn', '/norestart')
}

function Test-OpenHumanWindowsProcessElevated {
  <#
  .SYNOPSIS
    True when the current process is running with an administrator token (Windows only).
  #>
  if ($env:OS -ne 'Windows_NT') {
    return $false
  }
  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  $principal = [Security.Principal.WindowsPrincipal]::new($identity)
  return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Test-OpenHumanInstallerExitCodeSucceeded {
  param(
    [Parameter(Mandatory = $true)]
    [int]$ExitCode,

    [Parameter(Mandatory = $true)]
    [ValidateSet("MSI", "EXE")]
    [string]$InstallerType
  )

  return $ExitCode -eq 0 -or ($InstallerType -eq "MSI" -and $ExitCode -in @(1641, 3010))
}

function Assert-OpenHumanInstallerProcessSucceeded {
  <#
  .SYNOPSIS
    Throw when a Windows installer process reports an unsuccessful exit code.
  #>
  param(
    [Parameter(Mandatory = $true)]
    [int]$ExitCode,

    [Parameter(Mandatory = $true)]
    [ValidateSet("MSI", "EXE")]
    [string]$InstallerType
  )

  if (Test-OpenHumanInstallerExitCodeSucceeded -ExitCode $ExitCode -InstallerType $InstallerType) {
    return
  }

  if ($InstallerType -eq "MSI") {
    throw "MSI install failed with exit code $ExitCode."
  }

  throw "Installer exited with code $ExitCode."
}

function Select-OpenHumanWindowsAssetFromRelease {
  <#
  .SYNOPSIS
    Pick the Windows x64 MSI from a GitHub release object, else NSIS exe.
  #>
  param(
    [Parameter(Mandatory = $true)]
    [object]$Release
  )
  $assets = @($Release.assets)
  if (-not $assets -or $assets.Count -eq 0) {
    return $null
  }

  $msi = $assets | Where-Object { $_.name -match 'OpenHuman_.*x64.*\.msi$' } | Select-Object -First 1
  if ($msi) {
    return $msi
  }

  $exe = $assets | Where-Object { $_.name -match 'OpenHuman_.*x64.*\.exe$' } | Select-Object -First 1
  if ($exe) {
    return $exe
  }

  return $null
}

# Wrap in a function so `param()` works when piped via `irm | iex`.
# When piped, PowerShell cannot bind param() at the top-level scope.
function Install-OpenHuman {
  param(
    [switch]$Help,
    [switch]$Version,
    [string]$Channel = "stable",
    [switch]$DryRun
  )

  $ErrorActionPreference = "Stop"

  $InstallerVersion = "1.1.0"
  $Repo = "tinyhumansai/openhuman"
  $LatestReleaseApiUrl = "https://api.github.com/repos/$Repo/releases/latest"

  function Write-Info([string]$Message) { Write-Host "-> $Message" -ForegroundColor Cyan }
  function Write-Ok([string]$Message) { Write-Host "OK $Message" -ForegroundColor Green }
  function Write-WarnMsg([string]$Message) { Write-Host "!  $Message" -ForegroundColor Yellow }

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
    return
  }

  if ($Version) {
    Write-Output "openhuman-installer $InstallerVersion"
    return
  }

  if ($Channel -ne "stable") {
    throw "Only -Channel stable is currently supported."
  }

  if ($env:OS -ne "Windows_NT") {
    throw "This installer is for Windows only."
  }

  # Detect architecture — use environment variable as primary (always available),
  # fall back to .NET RuntimeInformation for newer PowerShell versions.
  $arch = $env:PROCESSOR_ARCHITECTURE
  if (-not $arch) {
    try {
      $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    } catch {
      $arch = ""
    }
  }
  $arch = "$arch".ToLowerInvariant()

  if ($arch -notin @("x64", "amd64")) {
    throw "Unsupported architecture: $arch (Windows x64 required)."
  }

  Write-Ok "Detected platform: windows/x64"

  $release = $null
  $releaseTag = ""
  $assetName = ""
  $assetUrl = ""
  $assetDigest = ""

  try {
    $release = Invoke-RestMethod -Uri $LatestReleaseApiUrl -UseBasicParsing
    $releaseTag = ($release.tag_name -replace '^v', '')
    $selected = Select-OpenHumanWindowsAssetFromRelease -Release $release
    if ($selected) {
      $assetName = $selected.name
      $assetUrl = $selected.browser_download_url
      if ($selected.digest) {
        $assetDigest = ($selected.digest -replace '^sha256:', '')
      }
    }
  } catch {
    throw
  }

  if (-not $assetUrl) {
    throw "No Windows x64 installer artifact found in latest release. Ensure release workflow publishes Windows MSI/EXE assets."
  }

  Write-Ok "Resolved latest release ($releaseTag): $assetName"

  $tmpFile = Join-Path $env:TEMP $assetName
  if ($DryRun) {
    Write-Output "DRY RUN: download $assetUrl -> $tmpFile"
  } else {
    Write-Info "Downloading $assetName"
    Invoke-WebRequest -Uri $assetUrl -OutFile $tmpFile -UseBasicParsing
  }

  if ($assetDigest) {
    if ($DryRun) {
      Write-Output "DRY RUN: verify SHA256 $assetDigest"
    } else {
      $fileHash = (Get-FileHash -Path $tmpFile -Algorithm SHA256).Hash.ToLowerInvariant()
      if ($fileHash -ne $assetDigest.ToLowerInvariant()) {
        throw "SHA256 mismatch for $assetName. Expected: $assetDigest. Actual: $fileHash."
      }
      Write-Ok "Integrity verified (sha256)"
    }
  } else {
    Write-WarnMsg "No SHA256 digest available for $assetName; skipping integrity verification."
  }

  if ($DryRun) {
    if ($assetName -like "*.msi") {
      $dryMsiArgs = Get-OpenHumanMsiexecInstallArgumentList -MsiPath $tmpFile
      Write-Output "DRY RUN: msiexec ArgumentList = $($dryMsiArgs | ConvertTo-Json -Compress)"
      if (Test-OpenHumanWindowsProcessElevated) {
        Write-Output "DRY RUN: (already elevated) Start-Process msiexec -Wait -ArgumentList <above>"
      } else {
        Write-Output "DRY RUN: (non-admin) Start-Process msiexec -Verb RunAs -Wait -ArgumentList <above>"
      }
    } else {
      Write-Output "DRY RUN: Start-Process `"$tmpFile`" -Wait"
    }
    return
  }

  Write-Info "Installing OpenHuman"
  if ($assetName -like "*.msi") {
    $msiArgs = Get-OpenHumanMsiexecInstallArgumentList -MsiPath $tmpFile
    $elevated = Test-OpenHumanWindowsProcessElevated
    if ($elevated) {
      $proc = Start-Process -FilePath "msiexec.exe" -ArgumentList $msiArgs -Wait -PassThru
    } else {
      Write-Info "Requesting administrator approval for machine-wide install (UAC)…"
      $proc = Start-Process -FilePath "msiexec.exe" -ArgumentList $msiArgs -Verb RunAs -Wait -PassThru
    }
    if (-not (Test-OpenHumanInstallerExitCodeSucceeded -ExitCode $proc.ExitCode -InstallerType "MSI")) {
      Write-WarnMsg "If this persists, capture a log: msiexec /i `"$tmpFile`" /l*v `"$env:TEMP\OpenHuman-msi.log`""
    }
    Assert-OpenHumanInstallerProcessSucceeded -ExitCode $proc.ExitCode -InstallerType "MSI"
  } elseif ($assetName -like "*.exe") {
    $proc = Start-Process -FilePath $tmpFile -Wait -PassThru
    Assert-OpenHumanInstallerProcessSucceeded -ExitCode $proc.ExitCode -InstallerType "EXE"
  } else {
    throw "Unsupported Windows installer type: $assetName"
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
}

# Run when executed as a script; skip when dot-sourced (e.g. unit tests).
if ($MyInvocation.InvocationName -ne '.') {
  Install-OpenHuman @args
}

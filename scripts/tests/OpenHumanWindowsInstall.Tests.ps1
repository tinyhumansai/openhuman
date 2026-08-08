#!/usr/bin/env pwsh
<#
.SYNOPSIS
  Unit tests for scripts/install.ps1 helpers (#913 MSI argument contract).

.DESCRIPTION
  Dot-sources install.ps1 (does not run Install-OpenHuman) and validates
  Get-OpenHumanMsiexecInstallArgumentList, Select-OpenHumanWindowsAssetFromRelease,
  Test-OpenHumanWindowsProcessElevated, and installer failure propagation.

  Run from repo root:
    pwsh -NoProfile -File scripts/tests/OpenHumanWindowsInstall.Tests.ps1
#>
$ErrorActionPreference = 'Stop'

$installScript = (Resolve-Path (Join-Path (Join-Path $PSScriptRoot '..') 'install.ps1')).Path
. $installScript

$testCount = 0
$failCount = 0

function Assert-Equal {
  param(
    [string]$Expected,
    [string]$Actual,
    [string]$Message
  )
  $script:testCount++
  if ($Expected -ne $Actual) {
    $script:failCount++
    Write-Host "FAIL: $Message" -ForegroundColor Red
    Write-Host "  expected: $Expected" -ForegroundColor Red
    Write-Host "  actual:   $Actual" -ForegroundColor Red
  } else {
    Write-Host "ok $Message" -ForegroundColor Green
  }
}

function Assert-True {
  param([bool]$Condition, [string]$Message)
  $script:testCount++
  if (-not $Condition) {
    $script:failCount++
    Write-Host "FAIL: $Message" -ForegroundColor Red
  } else {
    Write-Host "ok $Message" -ForegroundColor Green
  }
}

function Assert-DoesNotThrow {
  param([scriptblock]$Action, [string]$Message)
  $script:testCount++
  try {
    & $Action | Out-Null
    Write-Host "ok $Message" -ForegroundColor Green
  } catch {
    $script:failCount++
    Write-Host "FAIL: $Message" -ForegroundColor Red
    Write-Host "  unexpected error: $($_.Exception.Message)" -ForegroundColor Red
  }
}

function Assert-Throws {
  param([scriptblock]$Action, [string]$ExpectedMessage, [string]$Message)
  $script:testCount++
  try {
    & $Action | Out-Null
    $script:failCount++
    Write-Host "FAIL: $Message" -ForegroundColor Red
    Write-Host "  expected error: $ExpectedMessage" -ForegroundColor Red
    Write-Host "  actual: no terminating error" -ForegroundColor Red
  } catch {
    $actualMessage = $_.Exception.Message
    if ($ExpectedMessage -ne $actualMessage) {
      $script:failCount++
      Write-Host "FAIL: $Message" -ForegroundColor Red
      Write-Host "  expected error: $ExpectedMessage" -ForegroundColor Red
      Write-Host "  actual error:   $actualMessage" -ForegroundColor Red
    } else {
      Write-Host "ok $Message" -ForegroundColor Green
    }
  }
}

Write-Host "`n== Get-OpenHumanMsiexecInstallArgumentList (#913) ==" -ForegroundColor Cyan
$p = 'C:\Temp\OpenHuman_0.0.0_x64_en-US.msi'
$args = Get-OpenHumanMsiexecInstallArgumentList -MsiPath $p
Assert-True ($args.Count -eq 4) 'returns exactly 4 argument tokens'
Assert-Equal '/i' $args[0] 'first token is /i'
Assert-Equal $p $args[1] 'second token is MSI path'
$pSpaces = 'C:\Temp\Test User\OpenHuman_0.0.0_x64_en-US.msi'
$argsSpaces = Get-OpenHumanMsiexecInstallArgumentList -MsiPath $pSpaces
Assert-Equal $pSpaces $argsSpaces[1] 'path with spaces remains one second argv token (no split)'
Assert-Equal '/qn' $args[2] 'third token is /qn'
Assert-Equal '/norestart' $args[3] 'fourth token is /norestart'
Assert-True ($args -notcontains 'MSIINSTALLPERUSER') 'must not set MSIINSTALLPERUSER (perMachine MSI)'
Assert-True ($args -notcontains 'ALLUSERS=2') 'must not set ALLUSERS=2'
Assert-True ($args -notcontains 'ALLUSERS=1') 'must not set ALLUSERS=1 (use package default)'
$joined = $args -join ' '
Assert-True ($joined -notmatch 'MSIINSTALLPERUSER') 'joined args omit MSIINSTALLPERUSER'
Assert-True ($joined -notmatch 'ALLUSERS') 'joined args omit ALLUSERS'

Write-Host "`n== Select-OpenHumanWindowsAssetFromRelease ==" -ForegroundColor Cyan
$release = [pscustomobject]@{
  assets = @(
    [pscustomobject]@{ name = 'OpenHuman_1.0.0_x64_en-US.msi'; browser_download_url = 'https://example/msi' }
    [pscustomobject]@{ name = 'other.zip'; browser_download_url = 'https://example/z' }
  )
}
$sel = Select-OpenHumanWindowsAssetFromRelease -Release $release
Assert-Equal 'OpenHuman_1.0.0_x64_en-US.msi' $sel.name 'prefers MSI over other assets'

$releaseExe = [pscustomobject]@{
  assets = @(
    [pscustomobject]@{ name = 'OpenHuman_1.0.0_x64-setup.exe'; browser_download_url = 'https://example/exe' }
  )
}
$sel2 = Select-OpenHumanWindowsAssetFromRelease -Release $releaseExe
Assert-True ($null -ne $sel2) 'selects exe when no msi'
Assert-Equal 'OpenHuman_1.0.0_x64-setup.exe' $sel2.name 'exe name matches pattern'

$releaseEmpty = [pscustomobject]@{ assets = @() }
$sel3 = Select-OpenHumanWindowsAssetFromRelease -Release $releaseEmpty
Assert-True ($null -eq $sel3) 'null when no assets'

Write-Host "`n== Test-OpenHumanWindowsProcessElevated ==" -ForegroundColor Cyan
$t = Test-OpenHumanWindowsProcessElevated
Assert-True ($t -is [bool]) 'returns a boolean'

Write-Host "`n== Installer failure propagation ==" -ForegroundColor Cyan
Assert-DoesNotThrow { Assert-OpenHumanInstallerProcessSucceeded -ExitCode 0 -InstallerType 'MSI' } 'accepts a successful child process'
Assert-DoesNotThrow { Assert-OpenHumanInstallerProcessSucceeded -ExitCode 1641 -InstallerType 'MSI' } 'accepts an MSI success that initiated a reboot'
Assert-DoesNotThrow { Assert-OpenHumanInstallerProcessSucceeded -ExitCode 3010 -InstallerType 'MSI' } 'accepts an MSI success that requires a reboot'
Assert-Throws { Assert-OpenHumanInstallerProcessSucceeded -ExitCode 1603 -InstallerType 'MSI' } 'MSI install failed with exit code 1603.' 'turns an MSI failure into a terminating error'
Assert-Throws { Assert-OpenHumanInstallerProcessSucceeded -ExitCode 5 -InstallerType 'EXE' } 'Installer exited with code 5.' 'turns an EXE failure into a terminating error'
Assert-Throws { Assert-OpenHumanInstallerProcessSucceeded -ExitCode 3010 -InstallerType 'EXE' } 'Installer exited with code 3010.' 'does not treat MSI reboot codes as EXE success'

$versionOutput = (Install-OpenHuman -Version | Out-String).Trim()
Assert-Equal 'openhuman-installer 1.1.0' $versionOutput '-Version remains successful'
Assert-Throws { Install-OpenHuman -Channel 'preview' } 'Only -Channel stable is currently supported.' 'invalid arguments terminate instead of returning success'

$originalArch = $env:PROCESSOR_ARCHITECTURE
$originalOs = $env:OS
try {
  $env:OS = 'Windows_NT'
  $env:PROCESSOR_ARCHITECTURE = 'AMD64'
  function Invoke-RestMethod { throw 'release API unavailable' }
  Assert-Throws { Install-OpenHuman -DryRun } 'Could not query release API: release API unavailable' 'release API failures preserve their cause'
} finally {
  Remove-Item Function:\Invoke-RestMethod -ErrorAction SilentlyContinue
  $env:PROCESSOR_ARCHITECTURE = $originalArch
  $env:OS = $originalOs
}

$originalOs = $env:OS
try {
  $env:OS = 'OpenHumanTestUnsupported'
  Assert-Throws { Get-Content -Raw $installScript | Invoke-Expression } 'This installer is for Windows only.' 'piped irm|iex-style execution preserves terminating errors'
} finally {
  $env:OS = $originalOs
}

Write-Host "`n== $($testCount) checks, $failCount failed ==" -ForegroundColor $(if ($failCount -eq 0) { 'Green' } else { 'Red' })
if ($failCount -gt 0) {
  exit 1
}
exit 0

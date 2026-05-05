#!/usr/bin/env pwsh
<#
.SYNOPSIS
  Unit tests for scripts/install.ps1 helpers (#913 MSI argument contract).

.DESCRIPTION
  Dot-sources install.ps1 (does not run Install-OpenHuman) and validates
  Get-OpenHumanMsiexecInstallArgumentList, Select-OpenHumanWindowsAssetFromRelease,
  and Test-OpenHumanWindowsProcessElevated.

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

Write-Host "`n== $($testCount) checks, $failCount failed ==" -ForegroundColor $(if ($failCount -eq 0) { 'Green' } else { 'Red' })
if ($failCount -gt 0) {
  exit 1
}
exit 0

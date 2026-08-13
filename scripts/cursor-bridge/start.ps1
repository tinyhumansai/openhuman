# Starts the Cursor bridge if it is not already listening on 127.0.0.1:8790.
# OpenHuman sends the user's Cursor API key as the Bearer token; this script
# does not need a key of its own.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

function Test-BridgeUp {
    try {
        $h = Invoke-RestMethod -Uri 'http://127.0.0.1:8790/health' -TimeoutSec 2
        return [bool]$h.ok
    } catch {
        return $false
    }
}

if (Test-BridgeUp) {
    Write-Host '[cursor-bridge] already running on http://127.0.0.1:8790'
    exit 0
}

if (-not $env:CURSOR_BRIDGE_PORT) { $env:CURSOR_BRIDGE_PORT = '8790' }

$npm = Get-Command npm.cmd -ErrorAction SilentlyContinue
if (-not $npm) { $npm = Get-Command npm -ErrorAction SilentlyContinue }
if (-not $npm) {
    Write-Host '[cursor-bridge] npm not found on PATH'
    exit 1
}

Write-Host '[cursor-bridge] starting...'
Start-Process -FilePath $npm.Source -ArgumentList 'start' -WorkingDirectory $here -WindowStyle Hidden

$deadline = (Get-Date).AddSeconds(15)
while ((Get-Date) -lt $deadline) {
    if (Test-BridgeUp) {
        Write-Host '[cursor-bridge] listening on http://127.0.0.1:8790/v1'
        exit 0
    }
    Start-Sleep -Milliseconds 400
}

Write-Host '[cursor-bridge] started but health check did not pass within 15s'
exit 1

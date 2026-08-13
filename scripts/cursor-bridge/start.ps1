# Starts the Cursor bridge if it is not already listening.
# OpenHuman sends the user's Cursor API key as the Bearer token; this script
# does not need a key of its own.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

if (-not $env:CURSOR_BRIDGE_PORT) { $env:CURSOR_BRIDGE_PORT = '8790' }
$port = 0
if (-not [int]::TryParse($env:CURSOR_BRIDGE_PORT, [ref]$port) -or $port -lt 1 -or $port -gt 65535) {
    Write-Host "[cursor-bridge] invalid CURSOR_BRIDGE_PORT='$($env:CURSOR_BRIDGE_PORT)'; using 8790"
    $port = 8790
    $env:CURSOR_BRIDGE_PORT = '8790'
}
$healthUrl = "http://127.0.0.1:$port/health"
$listenUrl = "http://127.0.0.1:$port/v1"

function Test-BridgeUp {
    try {
        $h = Invoke-RestMethod -Uri $healthUrl -TimeoutSec 2
        return [bool]$h.ok
    } catch {
        return $false
    }
}

if (Test-BridgeUp) {
    Write-Host "[cursor-bridge] already running on $listenUrl"
    exit 0
}

$node = Get-Command node.exe -ErrorAction SilentlyContinue
if (-not $node) { $node = Get-Command node -ErrorAction SilentlyContinue }
if (-not $node) {
    Write-Host '[cursor-bridge] node not found on PATH'
    exit 1
}

Write-Host '[cursor-bridge] starting...'
$proc = Start-Process -FilePath $node.Source `
    -ArgumentList '--experimental-strip-types','src/index.ts' `
    -WorkingDirectory $here `
    -WindowStyle Hidden `
    -PassThru

$deadline = (Get-Date).AddSeconds(15)
while ((Get-Date) -lt $deadline) {
    if (Test-BridgeUp) {
        Write-Host "[cursor-bridge] listening on $listenUrl"
        exit 0
    }
    if ($proc.HasExited) {
        $code = $proc.ExitCode
        if ($code -eq 0) { $code = 1 }
        Write-Host "[cursor-bridge] process exited with status $code before becoming healthy"
        exit $code
    }
    Start-Sleep -Milliseconds 400
}

Write-Host '[cursor-bridge] started but health check did not pass within 15s'
exit 1

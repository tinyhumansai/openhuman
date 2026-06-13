---
description: >-
  Diagnose sign-in failures, OAuth callbacks that do not complete, and remote
  core RPC authentication problems.
icon: key
---

# Troubleshooting Sign-In

Use this checklist when social sign-in hangs, returns to the welcome screen, or the core logs an unauthorized `/auth` request.

## Check backend reachability

From the same network as the desktop app, verify the public OpenHuman endpoints:

```bash
curl -I https://tinyhumans.ai/
curl -I https://api.tinyhumans.ai/health
```

If the website loads but the API endpoint fails, the desktop app may not be able to exchange OAuth callbacks for a session. Capture the HTTP status, region, and DNS result in the issue report.

## Check the selected core

If you use the **Advanced** remote-core mode, confirm both the RPC URL and bearer token before starting OAuth:

```bash
curl -sS https://your-core.example/rpc \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer CORE_TOKEN" \
  -d '{"jsonrpc":"2.0","id":1,"method":"core.ping","params":{}}'
```

A `401` response means the desktop token and remote core token do not match. Fix that before retrying Google or GitHub sign-in.

## Check the deep-link callback

Successful desktop OAuth ends with an `openhuman://auth?...` callback. If the browser shows that URL but the app stays on the welcome screen:

1. Make sure only one OpenHuman desktop instance is running.
2. Restart the app, keep the same remote-core settings, and retry sign-in.
3. If using a remote core, check whether the core receives `openhuman.auth_store_session`.

## Windows: `openhuman://` handler not registered

On Windows the `openhuman://` URL scheme is registered to the running executable via `HKEY_CURRENT_USER\Software\Classes\openhuman\shell\open\command` at first launch. If that registration silently failed — or if the install was moved/copied after first launch — the browser cannot hand the OAuth callback back to the app, and sign-in stalls after the provider step (issue #2699).

The Tauri shell now emits a `log::error!` line at startup when this happens. Look for it in your log file (default `%USERPROFILE%\.openhuman\logs\openhuman.*.log`):

```
[deep-link] openhuman:// scheme registration unhealthy — OAuth callbacks may never reach the app.
register_all_error=…, hkcu_status=NotRegistered|MissingCommand|Stale { … }|ReadError(…)
```

To repair manually, open PowerShell **as the same user that runs OpenHuman** (no admin required — HKCU is per-user) and replace the path with your actual install location:

```powershell
$exe = 'C:\Path\To\OpenHuman.exe'   # update this
New-Item -Path 'HKCU:\Software\Classes\openhuman' -Force | Out-Null
Set-ItemProperty -Path 'HKCU:\Software\Classes\openhuman' -Name '(Default)' -Value 'URL:OpenHuman Protocol'
New-ItemProperty -Path 'HKCU:\Software\Classes\openhuman' -Name 'URL Protocol' -Value '' -Force | Out-Null
New-Item -Path 'HKCU:\Software\Classes\openhuman\shell\open\command' -Force | Out-Null
Set-ItemProperty -Path 'HKCU:\Software\Classes\openhuman\shell\open\command' -Name '(Default)' -Value ('"' + $exe + '" "%1"')
```

Restart OpenHuman afterwards and retry sign-in. If `register_all_error` is non-`None` in the log — for example because antivirus or a locked-down image is blocking writes to `HKCU\Software\Classes` — fixing the underlying policy is required; the manual script above will hit the same block.

For a remote core, a temporary manual injection can confirm the core is otherwise healthy:

```bash
curl -sS https://your-core.example/rpc \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer CORE_TOKEN" \
  -d '{"jsonrpc":"2.0","id":1,"method":"openhuman.auth_store_session","params":{"token":"JWT_FROM_CALLBACK"} }'
```

Do not paste real JWTs into public GitHub issues. Redact tokens and attach only status codes, hostnames, app version, OS, and the relevant log lines.

## What to include in a bug report

* App version and OS.
* Whether the core mode is local or remote.
* The RPC URL host, redacted token status, and `core.ping` result.
* The OAuth provider used.
* Whether an `openhuman://auth` URL appeared in the browser.
* The first unauthorized log line, if present.

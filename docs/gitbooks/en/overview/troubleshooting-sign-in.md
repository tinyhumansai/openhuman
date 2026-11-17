---
description: >-
  Diagnose sign-in failures, OAuth callbacks that do not complete, and remote
  core RPC authentication problems.
icon: key
---

# Troubleshooting sign-in

Use this checklist when social sign-in hangs, returns to the welcome screen, or the core logs an unauthorized `/auth` request.

## Check backend reachability

From the same network as the desktop app, check the public OpenHuman endpoints:

```bash
curl -I https://tinyhumans.ai/
curl -I https://api.tinyhumans.ai/health
```

If the website loads but the API endpoint fails, the desktop app may not be able to exchange OAuth callbacks for a session. Note the HTTP status, region and DNS result for any bug report.

## Check the selected core

If you use the **Advanced** remote-core mode, confirm both the RPC URL and bearer token before starting OAuth:

```bash
curl -sS https://your-core.example/rpc \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer CORE_TOKEN" \
  -d '{"jsonrpc":"2.0","id":1,"method":"core.ping","params":{}}'
```

A `401` means the desktop token and the remote core token do not match. Fix that before you retry Google or GitHub sign-in.

## Check the deep-link callback

A successful desktop sign-in ends with an `openhuman://auth?...` callback. If the browser shows that URL but the app stays on the welcome screen:

1. Make sure only one OpenHuman desktop instance is running.
2. Restart the app, keep the same remote-core settings, and retry sign-in.
3. With a remote core, check whether the core receives `openhuman.auth_set_credential`. The desktop shell checks the session against the backend first, then hands the credential to the core.

## Windows: `openhuman://` handler not registered

On Windows, the app registers the `openhuman://` URL scheme at first launch under `HKEY_CURRENT_USER\Software\Classes\openhuman\shell\open\command`. If that registration failed, or you moved or copied the install after first launch, the browser cannot hand the OAuth callback back to the app. Sign-in then stalls after the provider step.

The app logs an error at startup when this happens. Look for it in your log file (by default `%USERPROFILE%\.openhuman\logs\openhuman.*.log`):

```text
[deep-link] openhuman:// scheme registration unhealthy: OAuth callbacks may never reach the app.
register_all_error=..., hkcu_status=NotRegistered|MissingCommand|Stale { ... }|ReadError(...)
```

To repair it, open PowerShell as the same user that runs OpenHuman. You do not need admin rights, because HKCU is per user. Replace the path with your install location:

```powershell
$exe = 'C:\Path\To\OpenHuman.exe'   # update this
New-Item -Path 'HKCU:\Software\Classes\openhuman' -Force | Out-Null
Set-ItemProperty -Path 'HKCU:\Software\Classes\openhuman' -Name '(Default)' -Value 'URL:OpenHuman Protocol'
New-ItemProperty -Path 'HKCU:\Software\Classes\openhuman' -Name 'URL Protocol' -Value '' -Force | Out-Null
New-Item -Path 'HKCU:\Software\Classes\openhuman\shell\open\command' -Force | Out-Null
Set-ItemProperty -Path 'HKCU:\Software\Classes\openhuman\shell\open\command' -Name '(Default)' -Value ('"' + $exe + '" "%1"')
```

Restart OpenHuman and retry sign-in. If `register_all_error` is not `None` in the log, something is blocking writes to `HKCU\Software\Classes`, such as antivirus or a locked-down image. You must fix that policy first, because the script above hits the same block.

With a remote core, you can inject a credential by hand to confirm the core is otherwise healthy. The core stores the credential as given, so supply the user id the token belongs to unless the JWT carries a subject claim:

```bash
curl -sS https://your-core.example/rpc \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer CORE_TOKEN" \
  -d '{"jsonrpc":"2.0","id":1,"method":"openhuman.auth_set_credential","params":{"token":"JWT_FROM_CALLBACK","userId":"YOUR_USER_ID"} }'
```

Never paste real JWTs into public GitHub issues. Redact tokens and share only status codes, hostnames, app version, OS and the relevant log lines.

## What to include in a bug report

- App version and OS.
- Whether the core is local or remote.
- The RPC URL host, whether a token was set (not the token), and the `core.ping` result.
- The OAuth provider used.
- Whether an `openhuman://auth` URL appeared in the browser.
- The first unauthorized log line, if present.

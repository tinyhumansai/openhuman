---
paths:
  - "**/capabilities/**"
  - "**/tauri.conf.json"
---

# Permissions and Capabilities

## Overview

Tauri v2 uses a capability-based security model. Permissions must be explicitly granted for the frontend to access system resources.

## Capability Files

Located in `src-tauri/capabilities/`:

```
src-tauri/capabilities/
├── default.json        # Default permissions for all windows
└── mobile.json         # Mobile-specific permissions
```

## Default Capability

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Default permissions for the main window",
  "windows": ["main"],
  "permissions": ["core:default", "opener:default"]
}
```

## Adding Permissions

### File System Access

```json
{
  "permissions": [
    "fs:default",
    "fs:allow-read-text-file",
    "fs:allow-write-text-file",
    { "identifier": "fs:scope", "allow": ["$APPDATA/*", "$DOCUMENT/*"] }
  ]
}
```

### Dialog Access

```json
{
  "permissions": [
    "dialog:default",
    "dialog:allow-open",
    "dialog:allow-save",
    "dialog:allow-message"
  ]
}
```

### HTTP Access

```json
{
  "permissions": [
    "http:default",
    {
      "identifier": "http:scope",
      "allow": [{ "url": "https://api.example.com/*" }, { "url": "https://*.myapp.com/*" }]
    }
  ]
}
```

### Notification Access

```json
{
  "permissions": [
    "notification:default",
    "notification:allow-is-permission-granted",
    "notification:allow-request-permission",
    "notification:allow-notify"
  ]
}
```

## Mobile-Specific Capabilities

Create `src-tauri/capabilities/mobile.json`:

```json
{
  "$schema": "../gen/schemas/mobile-schema.json",
  "identifier": "mobile",
  "description": "Mobile-specific permissions",
  "platforms": ["android", "iOS"],
  "windows": ["main"],
  "permissions": ["core:default", "barcode-scanner:default", "biometric:default", "haptics:default"]
}
```

## Available Permission Plugins

| Plugin       | Description          | Install Command                       |
| ------------ | -------------------- | ------------------------------------- |
| fs           | File system access   | `npm run tauri add fs`                |
| dialog       | System dialogs       | `npm run tauri add dialog`            |
| http         | HTTP requests        | `npm run tauri add http`              |
| notification | System notifications | `npm run tauri add notification`      |
| clipboard    | Clipboard access     | `npm run tauri add clipboard-manager` |
| shell        | Shell commands       | `npm run tauri add shell`             |
| store        | Persistent storage   | `npm run tauri add store`             |
| os           | OS information       | `npm run tauri add os`                |

## Scope Paths

Available path variables:

| Variable        | Description                  |
| --------------- | ---------------------------- |
| `$APPDATA`      | Application data directory   |
| `$APPCONFIG`    | Application config directory |
| `$APPLOCALDATA` | Application local data       |
| `$APPCACHE`     | Application cache            |
| `$APPLOG`       | Application logs             |
| `$AUDIO`        | User's audio directory       |
| `$CACHE`        | System cache                 |
| `$CONFIG`       | System config                |
| `$DATA`         | System data                  |
| `$DOCUMENT`     | User's documents             |
| `$DOWNLOAD`     | User's downloads             |
| `$PICTURE`      | User's pictures              |
| `$VIDEO`        | User's videos                |
| `$TEMP`         | Temporary directory          |

## Best Practices

1. **Principle of Least Privilege**: Only request permissions you need
2. **Scope Restrictions**: Limit file access to specific directories
3. **URL Whitelisting**: Only allow HTTP to known domains
4. **Platform Separation**: Use separate capability files for mobile/desktop
5. **Document Permissions**: Comment why each permission is needed

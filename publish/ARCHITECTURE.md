# Architecture Overview

> This document describes AlphaHuman's architecture at a conceptual level. AlphaHuman is closed-source software -- no source code is included in this repository.

## Overview

AlphaHuman is a native desktop application built with modern web technologies and a Rust backend. Unlike Electron-based apps, it uses the operating system's built-in web view, resulting in significantly smaller binaries and lower memory usage.

The app runs entirely on your machine. There is no mandatory cloud backend -- services like Telegram are connected directly from the app using their official APIs.

## Technology Stack

| Layer         | Technology                   | Why                                                            |
| ------------- | ---------------------------- | -------------------------------------------------------------- |
| **Frontend**  | React + TypeScript           | Component-based UI with type safety                            |
| **Backend**   | Rust                         | Native performance, memory safety, no garbage collector        |
| **Framework** | Tauri v2                     | Lightweight cross-platform framework (alternative to Electron) |
| **Real-time** | WebSocket                    | Low-latency bidirectional communication with services          |
| **AI**        | MCP (Model Context Protocol) | Standardized interface for AI tool execution                   |

## Architecture Diagram

```
+-----------------------------------------------------------+
|                       AlphaHuman App                       |
+-----------------------------------------------------------+
|                                                           |
|   +---------------------------------------------------+   |
|   |                    UI Layer                        |   |
|   |              React + TypeScript                    |   |
|   |   (Views, Components, State Management, Routing)   |   |
|   +---------------------------------------------------+   |
|                          |                                |
|   +---------------------------------------------------+   |
|   |                 Skills Engine                      |   |
|   |              Plugin Architecture                   |   |
|   |                                                   |   |
|   |   +-----------+  +-----------+  +------------+    |   |
|   |   | Telegram  |  |  Google   |  |  Web3      |    |   |
|   |   |  Skill    |  |  Skill   |  |  Skill     |    |   |
|   |   +-----------+  +-----------+  +------------+    |   |
|   +---------------------------------------------------+   |
|                          |                                |
|   +---------------------------------------------------+   |
|   |                  AI Engine                         |   |
|   |         Model Context Protocol (MCP)               |   |
|   |      (Tool Discovery, Execution, Responses)        |   |
|   +---------------------------------------------------+   |
|                          |                                |
|   +---------------------------------------------------+   |
|   |               Native Runtime                       |   |
|   |                 Rust / Tauri                        |   |
|   |    (IPC, Networking, File System, OS Keychain)     |   |
|   +---------------------------------------------------+   |
|                          |                                |
+-----------------------------------------------------------+
|                    Platform Layer                          |
|          macOS  |  Linux  |  Windows  |  Mobile           |
+-----------------------------------------------------------+
```

## Skills System

AlphaHuman's core extensibility comes from its **skills** architecture. Each skill is an independent module that connects to an external service and exposes a set of actions.

### How Skills Work

```
                  +-----------------+
                  |   Skill Store   |
                  | (discovery &    |
                  |  installation)  |
                  +-----------------+
                         |
                    install skill
                         |
                         v
+------------+    +-----------------+    +------------------+
|   User     |--->|     Skill       |--->| External Service |
| (or AI)    |    |                 |    | (Telegram, etc.) |
+------------+    |  - setup()      |    +------------------+
                  |  - connect()    |
                  |  - tools[]      |
                  |  - handlers[]   |
                  +-----------------+
```

### Skill Lifecycle

1. **Install** -- The skill module is loaded into the app
2. **Setup** -- User provides any required configuration (API keys, auth tokens)
3. **Connect** -- The skill establishes a connection to the external service
4. **Ready** -- The skill's tools become available to the user and AI engine

### What a Skill Provides

- **Tools** -- Discrete actions the skill can perform (e.g., "send a message", "search chats")
- **Views** -- Optional UI components for displaying skill-specific data
- **Event handlers** -- Reactions to real-time events from the connected service

### Current and Planned Skills

| Skill     | Services                         | Status    |
| --------- | -------------------------------- | --------- |
| Telegram  | Telegram chats, channels, groups | Available |
| Google    | Gmail, Calendar, Drive           | Planned   |
| Notion    | Pages, databases                 | Planned   |
| Web3      | Wallets, on-chain data           | Planned   |
| Exchanges | CEX/DEX trading                  | Planned   |

## AI Integration

AlphaHuman uses the **Model Context Protocol (MCP)** to give the AI engine a standardized way to discover and invoke tools provided by skills.

### How It Works

1. **Discovery** -- The AI engine queries all connected skills for their available tools
2. **Planning** -- Based on the user's request, the AI selects which tools to use
3. **Execution** -- The AI invokes tools through MCP, which routes calls to the appropriate skill
4. **Response** -- Results are returned to the AI, which synthesizes a response for the user

### Example Flow

```
User: "Summarize what I missed in the Trading group today"

  1. AI receives the request
  2. AI discovers Telegram skill has a "get_messages" tool
  3. AI calls get_messages(chat="Trading group", since="today")
  4. Telegram skill fetches messages via Telegram API
  5. AI receives the messages and generates a summary
  6. Summary is displayed to the user
```

The AI never has direct access to your credentials. It interacts with services only through the skill layer, which manages authentication independently.

## Security Model

### Rust Backend

The native backend is written in Rust, which provides memory safety guarantees at compile time. This eliminates entire classes of vulnerabilities (buffer overflows, use-after-free, data races) that are common in C/C++ applications.

### Sandboxed Web View

The UI runs inside the operating system's native web view (WebKit on macOS, WebKitGTK on Linux). The web view is sandboxed and can only communicate with the Rust backend through a controlled IPC (inter-process communication) bridge. It cannot make arbitrary system calls.

### Local-First Data

All user data is stored locally on your machine. Credentials and authentication tokens are stored in the operating system's keychain (Keychain on macOS, Secret Service on Linux). No data is sent to AlphaHuman's servers.

### Permission Model

The app uses a capability-based permission system. Each feature must explicitly declare what system resources it needs access to. The user interface cannot access the file system, network, or OS APIs without going through the Rust backend's permission checks.

## Platform Support

| Platform              | Status    | Notes                        |
| --------------------- | --------- | ---------------------------- |
| macOS (Apple Silicon) | Available | Primary development target   |
| macOS (Intel)         | Available | Universal binary support     |
| Linux (x64)           | Available | `.deb`, `.rpm`, `.AppImage`  |
| Windows               | Planned   | `.msi` and `.exe` installers |
| Android               | Planned   | Native mobile app            |
| iOS                   | Planned   | Native mobile app            |

## Why Not Electron?

AlphaHuman uses [Tauri](https://tauri.app) instead of Electron. Key differences:

|                  | Tauri                        | Electron                    |
| ---------------- | ---------------------------- | --------------------------- |
| Binary size      | ~10-15 MB                    | ~150+ MB                    |
| RAM usage        | Lower (uses system web view) | Higher (bundles Chromium)   |
| Backend language | Rust                         | Node.js                     |
| Security         | Sandboxed, capability-based  | Less restrictive by default |
| System web view  | Yes (no bundled browser)     | No (ships Chromium)         |

---

<p align="center">
  <sub>AlphaHuman is closed-source software. This document describes the architecture at a conceptual level for transparency and educational purposes.</sub>
</p>

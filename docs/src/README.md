# Source Code Documentation

This documentation covers the `/src` folder structure of the Outsourced Crypto Community Platform.

## Quick Reference

| Document                                      | Description                                         |
| --------------------------------------------- | --------------------------------------------------- |
| [Architecture Overview](./01-architecture.md) | High-level system architecture and provider chain   |
| [State Management](./02-state-management.md)  | Redux Toolkit slices, persistence, and selectors    |
| [Services Layer](./03-services.md)            | Singleton services (Socket.io, MTProto, API client) |
| [MCP System](./04-mcp-system.md)              | Model Context Protocol with 81 Telegram tools       |
| [Pages & Routing](./05-pages-routing.md)      | Route definitions, guards, and page components      |
| [Components](./06-components.md)              | Reusable UI components and settings modal system    |
| [Providers](./07-providers.md)                | React context providers and lifecycle management    |
| [Hooks & Utils](./08-hooks-utils.md)          | Custom hooks and utility functions                  |

## File Count Summary

| Category              | Files    | Purpose                                 |
| --------------------- | -------- | --------------------------------------- |
| Entry & Configuration | 7        | App init, routing, styles, types        |
| State Management      | 13       | Redux slices, selectors, hooks          |
| Providers             | 3        | Socket, Telegram, User contexts         |
| Services              | 5        | Singleton API clients                   |
| Pages                 | 9        | Full-page route components              |
| Components            | 28       | Reusable UI + settings modal (16 files) |
| MCP Core              | 14       | MCP interfaces, transport, logging      |
| MCP Telegram Tools    | 81       | Individual Telegram API operations      |
| Hooks                 | 2        | Custom React hooks                      |
| Types                 | 2        | TypeScript interfaces                   |
| Utils                 | 4        | Config, deep link, URL utilities        |
| Data                  | 1        | Static data (countries)                 |
| Assets                | 10+      | Icons and images                        |
| **TOTAL**             | **171+** | Complete frontend application           |

## Directory Structure

```
src/
├── App.tsx                    # Root component with provider chain
├── AppRoutes.tsx              # Route definitions
├── main.tsx                   # Entry point
├── polyfills.ts               # Node.js polyfills for browser
├── index.css                  # Global styles
│
├── store/                     # Redux state management (13 files)
├── providers/                 # Context providers (3 files)
├── services/                  # Singleton services (5 files)
├── lib/mcp/                   # MCP system (95+ files)
├── pages/                     # Page components (9 files)
├── components/                # UI components (28 files)
├── hooks/                     # Custom hooks (2 files)
├── types/                     # TypeScript types (2 files)
├── utils/                     # Utilities (4 files)
├── data/                      # Static data (1 file)
└── assets/                    # Icons and images
```

## Key Architectural Decisions

1. **HashRouter over BrowserRouter** - Required for Tauri deep link compatibility
2. **Redux Toolkit with Persistence** - Robust state management with rehydration
3. **Singleton Services** - Prevents connection leaks for Socket.io and MTProto
4. **Per-User State Scoping** - Telegram/socket state keyed by user ID
5. **Portal-Based Settings Modal** - URL routing without affecting main routes
6. **81-Tool MCP System** - Comprehensive Telegram API coverage

## Getting Started

1. Read [Architecture Overview](./01-architecture.md) for the big picture
2. Understand [State Management](./02-state-management.md) for data flow
3. Review [Services Layer](./03-services.md) for backend communication
4. Explore [MCP System](./04-mcp-system.md) for AI tool integration

---

_Documentation maintained by stevenbaba_

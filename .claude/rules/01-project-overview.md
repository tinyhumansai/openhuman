# Project Overview

## Crypto Community Platform

This project is a **crypto-focused communication platform** built with Tauri v2, designed to serve the cryptocurrency ecosystem across multiple platforms:

- **Windows** (Desktop)
- **macOS** (Desktop)
- **Android** (Mobile)
- **iOS** (Mobile)

## Target Users

- **Crypto Professionals**: Traders, Yield Farmers, Investors
- **Researchers & Analysts**: KOLs, Developers, Researchers
- **Teams & Organizations**: Collaborative workspaces for crypto projects
- **General Community**: Crypto enthusiasts and community members

## Technology Stack

| Layer                 | Technology      | Version | Purpose                  |
| --------------------- | --------------- | ------- | ------------------------ |
| **Frontend Core**     |
| UI Framework          | React           | 19.1.0  | Component-based UI       |
| Language              | TypeScript      | 5.8.3   | Type safety              |
| Build Tool            | Vite            | 7.0.4   | Fast development         |
| **UI & Styling**      |
| Styling               | Tailwind CSS    | Latest  | Utility-first CSS        |
| Components            | Headless UI     | Latest  | Accessible components    |
| Animation             | Framer Motion   | Latest  | Smooth animations        |
| **State & Data**      |
| State Management      | Redux Toolkit   | Latest  | Predictable state mgmt   |
| State Persistence     | Redux Persist   | Latest  | State rehydration        |
| Data Fetching         | TanStack Query  | Latest  | Server state management  |
| Form Handling         | React Hook Form | Latest  | Form validation          |
| **AI & Intelligence** |
| AI Memory             | Custom System   | Latest  | Context & learning       |
| Entity Graph          | Neo4j           | Latest  | Knowledge relationships  |
| Embeddings            | OpenAI          | Latest  | Semantic search          |
| **Communication**     |
| Real-time             | Socket.io       | Latest  | Live messaging           |
| Telegram Integration  | MTProto         | Latest  | Deep Telegram access     |
| MCP Protocol          | JSON-RPC 2.0    | Latest  | AI tool execution        |
| **Team & Skills**     |
| Skills Platform       | GitHub Sync     | Latest  | Dynamic skill loading    |
| Team Management       | REST API        | Latest  | Multi-user collaboration |
| **Backend Core**      |
| Language              | Rust            | 1.93.0  | Performance & safety     |
| Framework             | Tauri           | 2.x     | Cross-platform apps      |
| **Backend Libraries** |
| Async Runtime         | Tokio           | Latest  | Async operations         |
| JSON Handling         | Serde JSON      | Latest  | Serialization            |
| Database              | SQLx + SQLite   | Latest  | Local storage            |
| HTTP Client           | Reqwest         | Latest  | API requests             |
| WebSocket             | WebSocket crate | Latest  | Real-time messaging      |
| Utilities             | UUID            | Latest  | Unique identifiers       |

## Project Structure

```
frontend-runner-openhuman/
├── .claude/                # Claude AI configuration
│   ├── rules/              # Modular documentation
│   └── agents/             # Subagent configurations
├── src/                    # React frontend source
│   ├── lib/                # Core libraries
│   │   ├── ai/             # AI system (memory, constitution, entities)
│   │   └── mcp/            # Model Context Protocol implementation
│   ├── components/         # React components
│   │   └── settings/       # Settings modal system
│   ├── store/              # Redux state management
│   ├── services/           # API clients and services
│   └── pages/              # Application pages
├── src-tauri/              # Rust backend source
│   ├── gen/                # Generated platform code
│   │   ├── android/        # Android project
│   │   └── apple/          # iOS/macOS project
│   ├── icons/              # Application icons
│   └── src/                # Rust source code
├── skills/                 # Skills submodule (GitHub synced)
├── public/                 # Static assets
│   └── lottie/             # Animation files
├── .github/workflows/      # CI/CD pipelines
└── dist/                   # Build output
```

## Key Configuration Files

- `tauri.conf.json` - Tauri configuration (app identifier: com.openhuman.app)
- `Cargo.toml` - Rust dependencies and workspace configuration
- `package.json` - Node.js dependencies and scripts
- `vite.config.ts` - Vite build configuration with Node.js polyfills
- `tsconfig.json` - TypeScript configuration with strict settings
- `eslint.config.js` - ESLint configuration with ES modules
- `.prettierrc` - Code formatting rules
- `.husky/` - Git hooks for pre-commit/pre-push quality checks
- `CLAUDE.md` - Main project documentation for Claude Code

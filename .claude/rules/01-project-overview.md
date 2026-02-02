# Project Overview

## Crypto Community Platform

This project is a **crypto-focused communication platform** built with Tauri v2, designed to serve the cryptocurrency ecosystem across multiple platforms:

- **Windows** (Desktop)
- **macOS** (Desktop)
- **Android** (Mobile)
- **iOS** (Mobile)

## Target Users

- Traders, Yield Farmers, Investors
- Researchers, KOLs, Developers
- General crypto community members

## Technology Stack

| Layer                 | Technology      | Version | Purpose                 |
| --------------------- | --------------- | ------- | ----------------------- |
| **Frontend Core**     |
| UI Framework          | React           | 19.1.0  | Component-based UI      |
| Language              | TypeScript      | 5.8.3   | Type safety             |
| Build Tool            | Vite            | 7.0.4   | Fast development        |
| **UI & Styling**      |
| Styling               | Tailwind CSS    | Latest  | Utility-first CSS       |
| Components            | Headless UI     | Latest  | Accessible components   |
| Animation             | Framer Motion   | Latest  | Smooth animations       |
| **State & Data**      |
| State Management      | Zustand         | Latest  | Lightweight state       |
| Data Fetching         | TanStack Query  | Latest  | Server state management |
| Form Handling         | React Hook Form | Latest  | Form validation         |
| **Backend Core**      |
| Language              | Rust            | 1.93.0  | Performance & safety    |
| Framework             | Tauri           | 2.x     | Cross-platform apps     |
| **Backend Libraries** |
| Async Runtime         | Tokio           | Latest  | Async operations        |
| JSON Handling         | Serde JSON      | Latest  | Serialization           |
| Database              | SQLx + SQLite   | Latest  | Local storage           |
| HTTP Client           | Reqwest         | Latest  | API requests            |
| WebSocket             | WebSocket crate | Latest  | Real-time messaging     |
| Utilities             | UUID            | Latest  | Unique identifiers      |

## Project Structure

```
tauri-crossplatform-app/
├── .claude/                # Claude AI configuration
│   ├── rules/              # Modular documentation
│   └── agents/             # Subagent configurations
├── src/                    # React frontend source
├── src-tauri/              # Rust backend source
│   ├── gen/                # Generated platform code
│   │   ├── android/        # Android project
│   │   └── apple/          # iOS/macOS project
│   ├── icons/              # Application icons
│   └── src/                # Rust source code
├── public/                 # Static assets
└── dist/                   # Build output
```

## Key Configuration Files

- `tauri.conf.json` - Tauri configuration
- `Cargo.toml` - Rust dependencies
- `package.json` - Node.js dependencies
- `vite.config.ts` - Vite build configuration
- `tsconfig.json` - TypeScript configuration

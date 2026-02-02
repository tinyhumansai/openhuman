# Detailed Tech Stack Documentation

## Frontend Technologies

### Core Framework

- **React 19.1.0**
  - Latest React with concurrent features
  - Component-based architecture
  - Built-in state management primitives

- **TypeScript 5.8.3**
  - Type safety for large applications
  - Enhanced developer experience
  - Better refactoring support

- **Vite 7.0.4**
  - Fast development server
  - Hot module replacement
  - Optimized builds

### UI & Styling

- **Tailwind CSS**
  - Utility-first CSS framework
  - Rapid prototyping
  - Consistent design system
  - Mobile-first responsive design

- **Headless UI**
  - Accessible UI components
  - Unstyled, flexible components
  - Keyboard navigation support
  - Screen reader compatibility

- **Framer Motion**
  - Smooth animations and transitions
  - Gesture support
  - Layout animations
  - Performance optimized

### State Management & Data

- **Zustand**
  - Lightweight state management
  - TypeScript friendly
  - No boilerplate
  - DevTools support

- **TanStack Query (React Query)**
  - Server state management
  - Caching and synchronization
  - Background updates
  - Optimistic updates

- **React Hook Form**
  - Performant form handling
  - Minimal re-renders
  - Built-in validation
  - Easy integration with UI libraries

## Backend Technologies

### Core Runtime

- **Rust 1.93.0**
  - Memory safety without garbage collection
  - Zero-cost abstractions
  - Excellent performance
  - Cross-platform compilation

- **Tauri 2.x**
  - Cross-platform desktop/mobile apps
  - Small bundle size
  - Native OS integration
  - Secure IPC communication

### Essential Crates

- **Tokio**
  - Async runtime for Rust
  - High-performance networking
  - Task scheduling
  - Real-time capabilities

- **Serde + Serde JSON**
  - Serialization/deserialization
  - Type-safe JSON handling
  - Performance optimized
  - Extensive ecosystem support

- **SQLx + SQLite**
  - Compile-time checked SQL
  - Async database operations
  - Local-first storage
  - Cross-platform compatibility

- **Reqwest**
  - HTTP client library
  - Async/await support
  - JSON support built-in
  - Connection pooling

- **WebSocket Libraries**
  - Real-time communication
  - Bidirectional messaging
  - Connection management
  - Error handling

- **UUID**
  - Unique identifier generation
  - Various UUID versions
  - Cryptographically secure
  - Cross-platform consistency

## Platform Targets

### Desktop

- **Windows** (x64, ARM64)
- **macOS** (Intel, Apple Silicon)
- **Linux** (x64, ARM64) - Optional

### Mobile

- **Android** (API 21+)
- **iOS** (iOS 13+)

## Development Tools

### Build & Development

- **Cargo** - Rust package manager
- **NPM** - Node.js package manager
- **Tauri CLI** - Build and development tools

### Code Quality

- **ESLint** - JavaScript/TypeScript linting
- **Prettier** - Code formatting
- **Clippy** - Rust linting
- **Rustfmt** - Rust code formatting

## Architecture Principles

### Frontend

- **Component-driven development**
- **Type-safe state management**
- **Responsive design first**
- **Accessible by default**

### Backend

- **Async-first architecture**
- **Local-first data storage**
- **Secure communication**
- **Performance optimized**

### Cross-Platform

- **Native OS integration**
- **Consistent user experience**
- **Platform-specific optimizations**
- **Shared business logic**

---

_Last updated: 2026-01-27_

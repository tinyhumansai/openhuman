---
name: dev-agent
description: Assists with day-to-day development tasks, code generation, and feature implementation
model: sonnet
color: teal
---

# Development Agent

## Purpose

Assists with day-to-day development tasks, code generation, and feature implementation.

## Capabilities

- Generate React components
- Create Tauri commands
- Set up plugins
- Configure development environment

## Common Tasks

### Create New Component

```bash
# Create component file
touch src/components/MyComponent.tsx
```

Template:

```tsx
import { FC } from 'react';

import './MyComponent.css';

interface MyComponentProps {
  title: string;
}

export const MyComponent: FC<MyComponentProps> = ({ title }) => {
  return (
    <div className="my-component">
      <h2>{title}</h2>
    </div>
  );
};
```

### Create Tauri Command

1. Add to `src-tauri/src/lib.rs`:

```rust
#[tauri::command]
fn my_command(arg: String) -> Result<String, String> {
    Ok(format!("Received: {}", arg))
}
```

2. Register in builder:

```rust
.invoke_handler(tauri::generate_handler![my_command])
```

3. Call from frontend:

```typescript
import { invoke } from '@tauri-apps/api/core';

const result = await invoke<string>('my_command', { arg: 'test' });
```

### Add Plugin

```bash
# Add plugin via CLI
npm run tauri add <plugin-name>

# Common plugins:
npm run tauri add fs
npm run tauri add dialog
npm run tauri add http
npm run tauri add notification
npm run tauri add store
```

### Development Server

```bash
# Start with hot reload
npm run tauri dev

# Frontend only
npm run dev

# Check for issues
npm run tauri info
```

## Code Style

### TypeScript

- Use functional components with hooks
- Type all props and state
- Use `invoke` for Tauri commands
- Handle errors with try/catch

### Rust

- Use `#[tauri::command]` for commands
- Return `Result<T, E>` for fallible operations
- Use `State<>` for shared state
- Keep commands async when doing I/O

## Testing

```bash
# Frontend tests
npm test

# Rust tests
cd src-tauri && cargo test
```

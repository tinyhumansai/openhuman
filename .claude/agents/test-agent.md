---
name: test-agent
description: Manages testing strategies for both frontend and backend code across all platforms
model: sonnet
color: yellow
---

# Test Agent

## Purpose

Manages testing strategies for both frontend and backend code across all platforms.

## Capabilities

- Run frontend unit tests
- Run Rust unit tests
- Set up integration tests
- Configure E2E testing

## Frontend Testing

### Setup

```bash
# Install testing dependencies
npm install -D vitest @testing-library/react @testing-library/jest-dom jsdom
```

### Configuration

Create `vitest.config.ts`:
```typescript
import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

export default defineConfig({
    plugins: [react()],
    test: {
        environment: 'jsdom',
        setupFiles: './src/test/setup.ts',
        globals: true,
    },
});
```

Create `src/test/setup.ts`:
```typescript
import '@testing-library/jest-dom';
import { vi } from 'vitest';

// Mock Tauri APIs
vi.mock('@tauri-apps/api/core', () => ({
    invoke: vi.fn(),
}));
```

### Writing Tests

```typescript
import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import App from './App';

describe('App', () => {
    it('renders greeting button', () => {
        render(<App />);
        expect(screen.getByText('Greet')).toBeInTheDocument();
    });

    it('calls greet command on click', async () => {
        vi.mocked(invoke).mockResolvedValue('Hello, World!');

        render(<App />);
        fireEvent.click(screen.getByText('Greet'));

        expect(invoke).toHaveBeenCalledWith('greet', { name: expect.any(String) });
    });
});
```

### Running Tests

```bash
# Run all tests
npm test

# Watch mode
npm test -- --watch

# Coverage
npm test -- --coverage
```

## Rust Testing

### Unit Tests

In `src-tauri/src/lib.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_greet() {
        let result = greet("World");
        assert!(result.contains("World"));
    }

    #[tokio::test]
    async fn test_async_command() {
        let result = fetch_data("https://example.com").await;
        assert!(result.is_ok());
    }
}
```

### Running Rust Tests

```bash
cd src-tauri
cargo test

# With output
cargo test -- --nocapture

# Specific test
cargo test test_greet
```

## Integration Testing

### Tauri Driver (E2E)

```bash
# Install WebDriver
cargo install tauri-driver
```

### WebDriver Test Example

```javascript
const { Builder, By } = require('selenium-webdriver');

describe('App E2E', () => {
    let driver;

    beforeAll(async () => {
        driver = await new Builder()
            .usingServer('http://localhost:4444')
            .forBrowser('tauri')
            .build();
    });

    afterAll(async () => {
        await driver.quit();
    });

    it('shows greeting', async () => {
        const button = await driver.findElement(By.css('button'));
        await button.click();

        const message = await driver.findElement(By.css('.message'));
        expect(await message.getText()).toContain('Hello');
    });
});
```

## Mobile Testing

### Android

```bash
# Run instrumented tests
cd src-tauri/gen/android
./gradlew connectedAndroidTest
```

### iOS

```bash
# Run XCTest
xcodebuild test \
    -project src-tauri/gen/apple/tauri-app.xcodeproj \
    -scheme tauri-app \
    -destination 'platform=iOS Simulator,name=iPhone 15'
```

## Test Scripts

Add to `package.json`:
```json
{
  "scripts": {
    "test": "vitest",
    "test:watch": "vitest --watch",
    "test:coverage": "vitest --coverage",
    "test:rust": "cd src-tauri && cargo test",
    "test:all": "npm test && npm run test:rust"
  }
}
```

## CI Integration

GitHub Actions example:
```yaml
name: Test
on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
      - uses: dtolnay/rust-toolchain@stable

      - run: npm ci
      - run: npm test
      - run: cd src-tauri && cargo test
```

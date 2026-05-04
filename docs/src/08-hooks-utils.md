# Hooks & Utilities

Custom React hooks and utility functions.

## Custom Hooks

### useSocket (`hooks/useSocket.ts`)

Access Socket.io functionality from any component.

```typescript
interface UseSocketReturn {
  socket: Socket | null;
  isConnected: boolean;
  emit: (event: string, data: unknown) => void;
  on: (event: string, handler: Function) => void;
  off: (event: string, handler: Function) => void;
  once: (event: string, handler: Function) => void;
}

function useSocket(): UseSocketReturn;
```

**Usage:**

```typescript
import { useSocket } from "../hooks/useSocket";

function ChatInput() {
  const { emit, isConnected } = useSocket();

  const sendMessage = (text: string) => {
    if (isConnected) {
      emit("chat:message", { text });
    }
  };

  return (
    <input
      disabled={!isConnected}
      onKeyDown={(e) => e.key === "Enter" && sendMessage(e.target.value)}
    />
  );
}
```

**With event listeners:**

```typescript
function Notifications() {
  const { on, off } = useSocket();
  const [notifications, setNotifications] = useState([]);

  useEffect(() => {
    const handler = (notification) => {
      setNotifications((prev) => [...prev, notification]);
    };

    on("notification", handler);
    return () => off("notification", handler);
  }, [on, off]);

  return <NotificationList items={notifications} />;
}
```

### useUser (`hooks/useUser.ts`)

Access user profile data and loading state.

```typescript
interface UseUserReturn {
  user: User | null;
  loading: boolean;
  error: string | null;
  refetch: () => Promise<void>;
}

function useUser(): UseUserReturn;
```

**Usage:**

```typescript
import { useUser } from "../hooks/useUser";

function ProfileHeader() {
  const { user, loading, error, refetch } = useUser();

  if (loading) return <Skeleton />;
  if (error) return <Error message={error} onRetry={refetch} />;
  if (!user) return null;

  return (
    <div className="profile">
      <Avatar src={user.avatar} />
      <span>
        {user.firstName} {user.lastName}
      </span>
    </div>
  );
}
```

### Settings Modal Hooks

#### useSettingsNavigation (`components/settings/hooks/useSettingsNavigation.ts`)

URL-based navigation for settings modal.

```typescript
interface UseSettingsNavigationReturn {
  currentRoute: string; // Current settings path
  navigateTo: (panel: string) => void; // Navigate to panel
  navigateBack: () => void; // Go back one level
  closeModal: () => void; // Close settings entirely
}

function useSettingsNavigation(): UseSettingsNavigationReturn;
```

**Usage:**

```typescript
import { useSettingsNavigation } from "./hooks/useSettingsNavigation";

function SettingsMenu() {
  const { navigateTo, closeModal } = useSettingsNavigation();

  return (
    <nav>
      <button onClick={() => navigateTo("connections")}>Connections</button>
      <button onClick={() => navigateTo("privacy")}>Privacy</button>
      <button onClick={closeModal}>Close</button>
    </nav>
  );
}
```

#### useSettingsAnimation (`components/settings/hooks/useSettingsAnimation.ts`)

Animation state management for settings modal.

```typescript
interface UseSettingsAnimationReturn {
  isEntering: boolean; // Modal is animating in
  isExiting: boolean; // Modal is animating out
  animationClass: string; // CSS class for current state
}

function useSettingsAnimation(): UseSettingsAnimationReturn;
```

**Usage:**

```typescript
import { useSettingsAnimation } from "./hooks/useSettingsAnimation";

function SettingsModal() {
  const { animationClass, isExiting } = useSettingsAnimation();

  return <div className={`modal ${animationClass}`}>{/* Content */}</div>;
}
```

## Utilities

### Configuration (`utils/config.ts`)

Build-time environment variable access. These constants only carry the value
that was baked into the bundle — for the **runtime** URL the app actually
talks to, see `services/backendUrl` and `hooks/useBackendUrl` below.

```typescript
// Build-time fallback only (used outside Tauri).
export const BACKEND_URL = import.meta.env.VITE_BACKEND_URL || 'https://api.example.com';

// Debug mode
export const DEBUG = import.meta.env.VITE_DEBUG === 'true';
```

**Usage (build-time only — feature flags, debug toggles, …):**

```typescript
import { DEBUG } from '../utils/config';

if (DEBUG) {
  console.log('debug enabled');
}
```

> **Do not** import `BACKEND_URL` directly to make API calls. Resolve the URL
> at runtime so the core sidecar's `api_url` (set on the login screen via
> `openhuman.config_resolve_api_url`) takes effect:
>
> ```typescript
> // React components
> import { useBackendUrl } from '../hooks/useBackendUrl';
> const backendUrl = useBackendUrl();
>
> // Non-React code
> import { getBackendUrl } from '../services/backendUrl';
> const backendUrl = await getBackendUrl();
> ```

### Deep Link (`utils/deeplink.ts`)

Build deep link URLs for authentication handoff.

```typescript
// Build auth deep link
function buildAuthDeepLink(token: string): string;

// Parse deep link URL
function parseDeepLink(url: string): { path: string; params: URLSearchParams };
```

**Usage:**

```typescript
import { buildAuthDeepLink } from '../utils/deeplink';

// Build URL for browser redirect
const deepLink = buildAuthDeepLink(loginToken);
// → "openhuman://auth?token=abc123"

// In web frontend after auth:
window.location.href = deepLink;
```

### Desktop Deep Link Listener (`utils/desktopDeepLinkListener.ts`)

Handle incoming deep links in desktop app.

```typescript
// Setup listener for deep link events
async function setupDesktopDeepLinkListener(): Promise<void>;
```

**Called in main.tsx:**

```typescript
// Lazy import to ensure Tauri IPC is ready
import('./utils/desktopDeepLinkListener').then(m => {
  m.setupDesktopDeepLinkListener().catch(console.error);
});
```

**What it does:**

1. Listens for `onOpenUrl` events from Tauri deep-link plugin
2. Parses `openhuman://auth?token=...` URLs
3. Calls Rust `exchange_token` command (bypasses CORS)
4. Stores session in Redux
5. Navigates to `/onboarding` or `/home`

**Loop prevention:**

```typescript
// Set flag before navigation to prevent reprocessing
localStorage.setItem('deepLinkHandled', 'true');
window.location.replace('/');

// On next load, clear flag
if (localStorage.getItem('deepLinkHandled') === 'true') {
  localStorage.removeItem('deepLinkHandled');
  return; // Don't process again
}
```

### URL Opener (`utils/openUrl.ts`)

Cross-platform URL opening.

```typescript
// Open URL in system browser
async function openUrl(url: string): Promise<void>;
```

**Usage:**

```typescript
import { openUrl } from '../utils/openUrl';

// Opens in system browser (not in-app WebView)
await openUrl('https://telegram.org/auth');
```

**Implementation:**

```typescript
export async function openUrl(url: string): Promise<void> {
  try {
    // Try Tauri opener plugin first
    const { open } = await import('@tauri-apps/plugin-opener');
    await open(url);
  } catch {
    // Fallback to browser API
    window.open(url, '_blank');
  }
}
```

## Polyfills (`polyfills.ts`)

Node.js polyfills for browser environment.

The `telegram` npm package requires Node.js APIs. These are polyfilled:

```typescript
// polyfills.ts
import { Buffer } from 'buffer';
import process from 'process';
import util from 'util';

window.Buffer = Buffer;
window.process = process;
window.util = util;
```

**Imported at app entry:**

```typescript
// main.tsx
import './polyfills';

// ... rest of app
```

**Vite configuration:**

```typescript
// vite.config.ts
export default defineConfig({
  resolve: { alias: { buffer: 'buffer', process: 'process/browser', util: 'util' } },
  define: { 'process.env': {}, global: 'globalThis' },
});
```

## Types

### API Types (`types/api.ts`)

```typescript
// API response wrapper
interface ApiResponse<T> {
  success: boolean;
  data?: T;
  error?: string;
}

// API error
interface ApiError {
  code: string;
  message: string;
  details?: unknown;
}

// User interface
interface User {
  id: string;
  firstName: string;
  lastName?: string;
  username?: string;
  email?: string;
  avatar?: string;
  telegramId?: string;
  subscription?: SubscriptionInfo;
  usage?: UsageInfo;
  createdAt: string;
  updatedAt: string;
}
```

### Onboarding Types (`types/onboarding.ts`)

```typescript
// Onboarding step definition
interface OnboardingStep {
  id: string;
  title: string;
  component: React.ComponentType<StepProps>;
}

// Step component props
interface StepProps {
  onNext: () => void;
  onBack: () => void;
}

// Connection option
interface ConnectionOption {
  id: string;
  label: string;
  icon: React.ComponentType;
  description: string;
  comingSoon?: boolean;
}
```

## Static Data

### Countries (`data/countries.ts`)

Country list for phone number input.

```typescript
interface Country {
  code: string; // "US"
  name: string; // "United States"
  dialCode: string; // "+1"
  flag: string; // "🇺🇸"
}

export const countries: Country[];
```

**Usage:**

```typescript
import { countries } from "../data/countries";

function PhoneInput() {
  const [country, setCountry] = useState(countries[0]);

  return (
    <div>
      <select
        value={country.code}
        onChange={(e) =>
          setCountry(countries.find((c) => c.code === e.target.value))
        }
      >
        {countries.map((c) => (
          <option key={c.code} value={c.code}>
            {c.flag} {c.name} ({c.dialCode})
          </option>
        ))}
      </select>
      <input placeholder="Phone number" />
    </div>
  );
}
```

## Best Practices

### Hook Dependencies

Always include dependencies in useEffect:

```typescript
// Good
useEffect(() => {
  on('event', handler);
  return () => off('event', handler);
}, [on, off, handler]);

// Bad - missing dependencies
useEffect(() => {
  on('event', handler);
  return () => off('event', handler);
}, []);
```

### Cleanup Functions

Always clean up subscriptions:

```typescript
useEffect(() => {
  const subscription = subscribe();
  return () => subscription.unsubscribe();
}, []);
```

### Error Boundaries

Wrap utility calls in try-catch:

```typescript
try {
  await openUrl(url);
} catch (error) {
  console.error('Failed to open URL:', error);
  // Fallback behavior
}
```

### Type Safety

Use TypeScript generics for API calls:

```typescript
const user = await apiClient.get<User>('/users/me');
// user is typed as User
```

---

_Previous: [Providers](./07-providers.md) | [Back to Index](./README.md)_

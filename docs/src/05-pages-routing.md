# Pages & Routing

The application uses HashRouter with protected and public route guards.

## Route Structure

```
/                  → Welcome (public)
/login             → Login (public)
/onboarding        → Onboarding (protected, requires auth, not yet onboarded)
/home              → Home (protected, requires auth + onboarded)
/settings          → Settings modal overlay
/settings/*        → Settings sub-panels
*                  → DefaultRedirect (fallback)
```

## Route Configuration (`AppRoutes.tsx`)

```typescript
export function AppRoutes() {
  return (
    <>
      <Routes>
        {/* Public routes - redirect if authenticated */}
        <Route element={<PublicRoute />}>
          <Route path="/" element={<Welcome />} />
          <Route path="/login" element={<Login />} />
        </Route>

        {/* Protected routes - require authentication */}
        <Route element={<ProtectedRoute />}>
          <Route path="/onboarding/*" element={<Onboarding />} />
        </Route>

        {/* Protected + onboarded routes */}
        <Route element={<ProtectedRoute requireOnboarded />}>
          <Route path="/home" element={<Home />} />
        </Route>

        {/* Fallback redirect */}
        <Route path="*" element={<DefaultRedirect />} />
      </Routes>

      {/* Settings modal overlay - renders on top of routes */}
      <SettingsModal />
    </>
  );
}
```

## Route Guards

### PublicRoute (`components/PublicRoute.tsx`)

Redirects authenticated users away from public pages.

```typescript
export function PublicRoute() {
  const token = useAppSelector((state) => state.auth.token);
  const isOnboarded = useAppSelector((state) =>
    selectIsOnboarded(state, userId),
  );

  if (token) {
    // Authenticated - redirect to appropriate page
    return <Navigate to={isOnboarded ? "/home" : "/onboarding"} replace />;
  }

  return <Outlet />;
}
```

### ProtectedRoute (`components/ProtectedRoute.tsx`)

Enforces authentication and optionally onboarding status.

```typescript
interface ProtectedRouteProps {
  requireOnboarded?: boolean;
}

export function ProtectedRoute({ requireOnboarded = false }) {
  const token = useAppSelector((state) => state.auth.token);
  const isOnboarded = useAppSelector((state) =>
    selectIsOnboarded(state, userId),
  );

  if (!token) {
    return <Navigate to="/login" replace />;
  }

  if (requireOnboarded && !isOnboarded) {
    return <Navigate to="/onboarding" replace />;
  }

  return <Outlet />;
}
```

### DefaultRedirect (`components/DefaultRedirect.tsx`)

Fallback route that redirects based on auth state.

```typescript
export function DefaultRedirect() {
  const token = useAppSelector((state) => state.auth.token);
  const isOnboarded = useAppSelector((state) =>
    selectIsOnboarded(state, userId),
  );

  if (!token) {
    return <Navigate to="/" replace />;
  }

  if (!isOnboarded) {
    return <Navigate to="/onboarding" replace />;
  }

  return <Navigate to="/home" replace />;
}
```

## Pages

### Welcome Page (`pages/Welcome.tsx`)

Landing page for unauthenticated users.

**Features:**

- App introduction and branding
- CTA to login/signup
- Public route (redirects if authenticated)

### Login Page (`pages/Login.tsx`)

Authentication page.

**Features:**

- Telegram OAuth button
- Opens `/auth/telegram?platform=desktop` in browser
- Handles deep link callback

```typescript
export function Login() {
  const handleTelegramLogin = () => {
    // Opens Telegram OAuth in system browser
    openUrl(`${BACKEND_URL}/auth/telegram?platform=desktop`);
  };

  return (
    <div className="login-page">
      <TelegramLoginButton onClick={handleTelegramLogin} />
    </div>
  );
}
```

### Home Page (`pages/Home.tsx`)

Main dashboard after authentication.

**Features:**

- Protected route (requires auth + onboarded)
- Connection status indicators
- Navigation to settings modal
- Future: Chat list, messages, etc.

```typescript
export function Home() {
  const navigate = useNavigate();
  const user = useAppSelector((state) => state.user.profile);
  const telegramStatus = useAppSelector((state) =>
    selectTelegramConnectionStatus(state, user?.id),
  );

  return (
    <div className="home-page">
      <header>
        <h1>Welcome, {user?.firstName}</h1>
        <button onClick={() => navigate("/settings")}>Settings</button>
      </header>

      <TelegramConnectionIndicator status={telegramStatus} />
      <ConnectionIndicator />

      {/* Main content */}
    </div>
  );
}
```

## Onboarding Flow (`pages/onboarding/`)

Multi-step onboarding process.

### Structure

```
pages/onboarding/
├── Onboarding.tsx           # Flow controller
└── steps/
    ├── GetStartedStep.tsx   # Welcome
    ├── PrivacyStep.tsx      # Privacy policy
    ├── AnalyticsStep.tsx    # Analytics opt-in
    ├── ConnectStep.tsx      # Telegram connection
    └── FeaturesStep.tsx     # Features overview
```

### Onboarding Controller (`Onboarding.tsx`)

```typescript
const STEPS = [
  { id: "get-started", component: GetStartedStep },
  { id: "privacy", component: PrivacyStep },
  { id: "analytics", component: AnalyticsStep },
  { id: "connect", component: ConnectStep },
  { id: "features", component: FeaturesStep },
];

export function Onboarding() {
  const [currentStep, setCurrentStep] = useState(0);
  const dispatch = useAppDispatch();
  const navigate = useNavigate();

  const handleNext = () => {
    if (currentStep < STEPS.length - 1) {
      setCurrentStep(currentStep + 1);
    } else {
      // Complete onboarding
      dispatch(setOnboarded({ userId, isOnboarded: true }));
      navigate("/home");
    }
  };

  const handleBack = () => {
    if (currentStep > 0) {
      setCurrentStep(currentStep - 1);
    }
  };

  const StepComponent = STEPS[currentStep].component;

  return (
    <div className="onboarding">
      <ProgressIndicator current={currentStep} total={STEPS.length} />
      <StepComponent onNext={handleNext} onBack={handleBack} />
    </div>
  );
}
```

### Step Components

Each step receives `onNext` and `onBack` callbacks:

```typescript
interface StepProps {
  onNext: () => void;
  onBack: () => void;
}

export function ConnectStep({ onNext, onBack }: StepProps) {
  const [showModal, setShowModal] = useState(false);
  const telegramStatus = useAppSelector(/* ... */);

  return (
    <div className="step">
      <h2>Connect Your Accounts</h2>

      {connectOptions.map((option) => (
        <ConnectionOption
          key={option.id}
          {...option}
          onClick={() => option.id === "telegram" && setShowModal(true)}
        />
      ))}

      <TelegramConnectionModal
        isOpen={showModal}
        onClose={() => setShowModal(false)}
      />

      <div className="actions">
        <button onClick={onBack}>Back</button>
        <button onClick={onNext}>Continue</button>
      </div>
    </div>
  );
}
```

## Settings Modal Routing

The settings modal overlays existing content using URL-based routing.

### Modal Detection

```typescript
// In SettingsModal.tsx
const location = useLocation();
const isOpen = location.pathname.startsWith('/settings');
```

### Sub-Routes

```
/settings              → SettingsHome (main menu)
/settings/connections  → ConnectionsPanel
/settings/messaging    → MessagingPanel (future)
/settings/privacy      → PrivacyPanel (future)
/settings/profile      → ProfilePanel (future)
/settings/advanced     → AdvancedPanel (future)
/settings/billing      → BillingPanel (future)
```

### Navigation

```typescript
import { useSettingsNavigation } from "./hooks/useSettingsNavigation";

function SettingsHome() {
  const { navigateTo, closeModal } = useSettingsNavigation();

  return (
    <div>
      <SettingsMenuItem
        label="Connections"
        onClick={() => navigateTo("connections")}
      />
      <button onClick={closeModal}>Close</button>
    </div>
  );
}
```

## HashRouter vs BrowserRouter

The app uses HashRouter for desktop compatibility:

```typescript
// App.tsx
import { HashRouter } from 'react-router-dom';

// URLs look like: app://localhost/#/home
// Instead of: app://localhost/home
```

**Why HashRouter:**

1. Tauri deep links work with hash-based URLs
2. No server configuration needed
3. Works with file:// protocol
4. Prevents 404 on direct URL access

## Deep Link Handling

Deep links are handled before routing:

```typescript
// main.tsx
import('./utils/desktopDeepLinkListener').then(m => {
  m.setupDesktopDeepLinkListener().catch(console.error);
});
```

The listener intercepts `openhuman://auth?token=...` and:

1. Exchanges token via Rust command
2. Stores session in Redux
3. Navigates to `/onboarding` or `/home`

## Navigation Patterns

### Programmatic Navigation

```typescript
import { useNavigate } from 'react-router-dom';

const navigate = useNavigate();

// Navigate to route
navigate('/home');

// Replace history entry
navigate('/login', { replace: true });

// Go back
navigate(-1);
```

### Link Component

```typescript
import { Link } from "react-router-dom";

<Link to="/settings">Settings</Link>;
```

### State Transfer

```typescript
// Pass state to route
navigate('/details', { state: { itemId: 123 } });

// Receive state
const location = useLocation();
const { itemId } = location.state;
```

---

_Previous: [MCP System](./04-mcp-system.md) | Next: [Components](./06-components.md)_

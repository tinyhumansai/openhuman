# Components

Reusable React components organized by feature.

## Component Structure

```
components/
├── Route Guards
│   ├── ProtectedRoute.tsx
│   ├── PublicRoute.tsx
│   └── DefaultRedirect.tsx
│
├── Authentication
│   └── TelegramLoginButton.tsx
│
├── Connection Status
│   ├── ConnectionIndicator.tsx
│   ├── TelegramConnectionIndicator.tsx
│   ├── TelegramConnectionModal.tsx
│   └── GmailConnectionIndicator.tsx
│
├── Onboarding
│   ├── ProgressIndicator.tsx
│   └── LottieAnimation.tsx
│
├── Settings Modal (16 files)
│   ├── SettingsModal.tsx
│   ├── SettingsLayout.tsx
│   ├── SettingsHome.tsx
│   ├── panels/
│   ├── components/
│   └── hooks/
│
└── Development
    └── DesignSystemShowcase.tsx
```

## Route Guard Components

### ProtectedRoute

Requires authentication and optionally onboarding.

```typescript
interface ProtectedRouteProps {
  requireOnboarded?: boolean;
}

// Usage in AppRoutes.tsx
<Route element={<ProtectedRoute />}>
  <Route path="/onboarding/*" element={<Onboarding />} />
</Route>

<Route element={<ProtectedRoute requireOnboarded />}>
  <Route path="/home" element={<Home />} />
</Route>
```

### PublicRoute

Redirects authenticated users away.

```typescript
// Usage in AppRoutes.tsx
<Route element={<PublicRoute />}>
  <Route path="/" element={<Welcome />} />
  <Route path="/login" element={<Login />} />
</Route>
```

### DefaultRedirect

Fallback that routes based on auth state.

```typescript
// Redirects to:
// - "/" if not authenticated
// - "/onboarding" if authenticated but not onboarded
// - "/home" if authenticated and onboarded
```

## Authentication Components

### TelegramLoginButton

OAuth login button for Telegram.

```typescript
interface TelegramLoginButtonProps {
  onClick: () => void;
  disabled?: boolean;
}

// Usage
<TelegramLoginButton
  onClick={() => openUrl(`${BACKEND_URL}/auth/telegram?platform=desktop`)}
/>
```

## Connection Status Components

### ConnectionIndicator

Generic connection status badge.

```typescript
interface ConnectionIndicatorProps {
  status: 'connected' | 'connecting' | 'disconnected' | 'error';
  label?: string;
}

<ConnectionIndicator status="connected" label="Socket" />
```

### TelegramConnectionIndicator

Telegram-specific status display.

```typescript
interface TelegramConnectionIndicatorProps {
  status: 'connected' | 'connecting' | 'disconnected' | 'error';
}

// Usage with Redux state
const telegramStatus = useAppSelector((state) =>
  selectTelegramConnectionStatus(state, userId)
);

<TelegramConnectionIndicator status={telegramStatus} />
```

### TelegramConnectionModal

Modal for setting up Telegram connection.

```typescript
interface TelegramConnectionModalProps {
  isOpen: boolean;
  onClose: () => void;
}

// Usage in onboarding/settings
const [showModal, setShowModal] = useState(false);

<TelegramConnectionModal
  isOpen={showModal}
  onClose={() => setShowModal(false)}
/>
```

**Features:**

- QR code login flow
- Phone number login flow
- Connection status display
- Error handling

### GmailConnectionIndicator

Gmail status badge (future integration).

```typescript
<GmailConnectionIndicator status="coming-soon" />
```

## Onboarding Components

### ProgressIndicator

Visual progress through onboarding steps.

```typescript
interface ProgressIndicatorProps {
  current: number;
  total: number;
}

<ProgressIndicator current={2} total={5} />
```

### LottieAnimation

Lottie animation player for onboarding.

```typescript
interface LottieAnimationProps {
  animationData: object;
  loop?: boolean;
  autoplay?: boolean;
  className?: string;
}

import welcomeAnimation from '../assets/animations/welcome.json';

<LottieAnimation
  animationData={welcomeAnimation}
  loop={true}
  autoplay={true}
/>
```

## Settings Modal System

Complete modal system with URL-based routing.

### File Structure

```
components/settings/
├── SettingsModal.tsx          # Route-based container
├── SettingsLayout.tsx         # Portal + backdrop wrapper
├── SettingsHome.tsx           # Main menu with profile
├── panels/
│   ├── ConnectionsPanel.tsx   # Connection management
│   ├── MessagingPanel.tsx     # (Future)
│   ├── PrivacyPanel.tsx       # (Future)
│   ├── ProfilePanel.tsx       # (Future)
│   ├── AdvancedPanel.tsx      # (Future)
│   └── BillingPanel.tsx       # (Future)
├── components/
│   ├── SettingsHeader.tsx     # User profile section
│   ├── SettingsMenuItem.tsx   # Menu item component
│   ├── SettingsBackButton.tsx # Back navigation
│   └── SettingsPanelLayout.tsx# Panel wrapper
└── hooks/
    ├── useSettingsNavigation.ts # URL routing
    └── useSettingsAnimation.ts  # Animation state
```

### SettingsModal

Main container that renders based on URL.

```typescript
export function SettingsModal() {
  const location = useLocation();
  const isOpen = location.pathname.startsWith('/settings');

  if (!isOpen) return null;

  return (
    <SettingsLayout>
      {/* Route to appropriate panel */}
      {location.pathname === '/settings' && <SettingsHome />}
      {location.pathname === '/settings/connections' && <ConnectionsPanel />}
      {/* ... more panels */}
    </SettingsLayout>
  );
}
```

### SettingsLayout

Portal-based modal wrapper.

```typescript
export function SettingsLayout({ children }) {
  const { closeModal } = useSettingsNavigation();

  return createPortal(
    <div className="fixed inset-0 z-50">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/50 backdrop-blur-sm"
        onClick={closeModal}
      />

      {/* Modal */}
      <div className="absolute inset-4 flex items-center justify-center">
        <div className="bg-white rounded-2xl w-full max-w-[520px] shadow-xl">
          {children}
        </div>
      </div>
    </div>,
    document.body
  );
}
```

### SettingsHome

Main menu with user profile.

```typescript
export function SettingsHome() {
  const { navigateTo, closeModal } = useSettingsNavigation();
  const user = useAppSelector((state) => state.user.profile);

  const menuItems = [
    { id: 'connections', label: 'Connections', icon: LinkIcon },
    { id: 'messaging', label: 'Messaging', icon: MessageIcon },
    { id: 'privacy', label: 'Privacy', icon: ShieldIcon },
    // ... more items
  ];

  return (
    <div>
      <SettingsHeader user={user} onClose={closeModal} />

      {menuItems.map((item) => (
        <SettingsMenuItem
          key={item.id}
          {...item}
          onClick={() => navigateTo(item.id)}
        />
      ))}
    </div>
  );
}
```

### ConnectionsPanel

Connection management interface.

```typescript
export function ConnectionsPanel() {
  const { navigateBack } = useSettingsNavigation();
  const [telegramModalOpen, setTelegramModalOpen] = useState(false);

  const telegramStatus = useAppSelector((state) =>
    selectTelegramConnectionStatus(state, userId)
  );

  // Reuses connectOptions from onboarding
  const connections = connectOptions.map((opt) => ({
    ...opt,
    status: opt.id === 'telegram' ? telegramStatus : 'coming-soon'
  }));

  return (
    <SettingsPanelLayout title="Connections" onBack={navigateBack}>
      {connections.map((conn) => (
        <ConnectionItem
          key={conn.id}
          {...conn}
          onConnect={() => conn.id === 'telegram' && setTelegramModalOpen(true)}
        />
      ))}

      <TelegramConnectionModal
        isOpen={telegramModalOpen}
        onClose={() => setTelegramModalOpen(false)}
      />
    </SettingsPanelLayout>
  );
}
```

### Settings Hooks

#### useSettingsNavigation

URL-based navigation for settings modal.

```typescript
interface UseSettingsNavigationReturn {
  currentRoute: string;
  navigateTo: (panel: string) => void;
  navigateBack: () => void;
  closeModal: () => void;
}

const { navigateTo, navigateBack, closeModal } = useSettingsNavigation();

// Navigate to panel
navigateTo('connections'); // → /settings/connections

// Go back
navigateBack(); // → /settings

// Close modal
closeModal(); // → previous non-settings route
```

#### useSettingsAnimation

Animation state management.

```typescript
interface UseSettingsAnimationReturn {
  isEntering: boolean;
  isExiting: boolean;
  animationClass: string;
}

const { animationClass } = useSettingsAnimation();

<div className={`modal ${animationClass}`}>
  {/* Content */}
</div>
```

### Settings Components

#### SettingsHeader

User profile section at top of settings.

```typescript
interface SettingsHeaderProps {
  user: User | null;
  onClose: () => void;
}

<SettingsHeader user={user} onClose={handleClose} />
```

#### SettingsMenuItem

Individual menu item with icon and chevron.

```typescript
interface SettingsMenuItemProps {
  label: string;
  icon: React.ComponentType;
  onClick: () => void;
  badge?: string;
  disabled?: boolean;
}

<SettingsMenuItem
  label="Connections"
  icon={LinkIcon}
  onClick={() => navigateTo('connections')}
  badge="2"
/>
```

#### SettingsBackButton

Back navigation button.

```typescript
interface SettingsBackButtonProps {
  onClick: () => void;
}

<SettingsBackButton onClick={navigateBack} />
```

#### SettingsPanelLayout

Wrapper for settings panels.

```typescript
interface SettingsPanelLayoutProps {
  title: string;
  onBack: () => void;
  children: React.ReactNode;
}

<SettingsPanelLayout title="Connections" onBack={navigateBack}>
  {/* Panel content */}
</SettingsPanelLayout>
```

## Component Patterns

### Reusing Connection Options

The `connectOptions` array is shared between onboarding and settings:

```typescript
// Defined in ConnectStep.tsx, imported elsewhere
export const connectOptions = [
  {
    id: 'telegram',
    label: 'Telegram',
    icon: TelegramIcon,
    description: 'Connect your Telegram account',
  },
  {
    id: 'gmail',
    label: 'Gmail',
    icon: GmailIcon,
    description: 'Connect your Gmail account',
    comingSoon: true,
  },
];
```

### Modal via Portal

Settings modal uses `createPortal` to render outside the component tree:

```typescript
return createPortal(
  <div className="modal-container">
    {/* Modal content */}
  </div>,
  document.body
);
```

### Controlled vs Uncontrolled

Connection modals are controlled components:

```typescript
// Parent controls open state
const [isOpen, setIsOpen] = useState(false);

<TelegramConnectionModal
  isOpen={isOpen}
  onClose={() => setIsOpen(false)}
/>
```

---

_Previous: [Pages & Routing](./05-pages-routing.md) | Next: [Providers](./07-providers.md)_

---
paths:
  - "app/src/components/settings/**"
---

# Settings Modal System - URL-Based Modal Architecture

## Overview

Complete settings modal system with clean white design that overlays on existing content. Features URL-based routing, Redux integration, and reusable component architecture for system settings management.

## Architecture

### Modal Infrastructure

**Location**: `src/components/settings/`

The settings modal system uses a clean architectural pattern:

```
SettingsModal.tsx           # Route-based modal container
├── SettingsLayout.tsx      # createPortal modal wrapper with backdrop
├── SettingsHome.tsx        # Main menu with user profile
├── panels/
│   └── ConnectionsPanel.tsx # Connection management interface
├── components/
│   ├── SettingsHeader.tsx  # User profile section
│   ├── SettingsMenuItem.tsx # Individual menu items
│   ├── SettingsBackButton.tsx # Back navigation
│   └── SettingsPanelLayout.tsx # Panel wrapper
└── hooks/
    ├── useSettingsNavigation.ts # URL routing logic
    └── useSettingsAnimation.ts  # Animation state
```

### URL Routing Pattern

```
/settings                   # Main settings menu
/settings/connections       # Connection management
/settings/messaging         # Future: messaging settings
/settings/privacy          # Future: privacy settings
/settings/profile          # Future: profile settings
/settings/advanced         # Future: advanced settings
/settings/billing          # Future: billing settings
```

## Design Specifications

### Modal Container

- **Width**: 520px (desktop), responsive on mobile
- **Background**: Pure white (#FFFFFF) - contrasts with app's glass morphism
- **Border-radius**: 16px
- **Shadow**: `0 20px 25px -5px rgba(0, 0, 0, 0.1)`
- **Backdrop**: Black 50% opacity with 8px blur
- **Position**: Fixed center with flexbox

### User Profile Section

- **Avatar**: 56px circular with border and shadow
- **Typography**: 18px semibold name, 14px gray email
- **Background**: Subtle gradient from white to gray-50
- **Integration**: Redux user state for name and email display

### Menu Items

- **Height**: 52px with proper touch targets
- **Hover**: bg-gray-50 with smooth transitions
- **Icons**: 20px with consistent spacing
- **Chevron**: 16px with translateX(2px) hover animation
- **Typography**: 15px medium weight for clarity

### Animation System

- **Entry**: 200ms ease-out modal slide up
- **Panel transitions**: 250ms slide from right
- **Micro-interactions**: 150ms hover effects
- **Exit**: 150ms ease-in with backdrop fade

## Component Usage

### Basic Modal Implementation

```tsx
// Trigger settings modal (from any component)
import { useNavigate } from 'react-router-dom';

const navigate = useNavigate();
const openSettings = () => navigate('/settings');
const openConnections = () => navigate('/settings/connections');
```

### Settings Navigation Hook

```tsx
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const { currentRoute, navigateTo, navigateBack, closeModal } = useSettingsNavigation();

// Navigate to connections
navigateTo('connections');

// Go back or close
navigateBack(); // or closeModal();
```

### Redux Integration

```tsx
// User profile data
const { user } = useAppSelector(state => state.user);
const displayName = user?.username || user?.firstName || 'User';

// Connection status
const { isAuthenticated } = useAppSelector(state => state.telegram);

// Logout functionality
const dispatch = useAppDispatch();
const handleLogout = () => {
  dispatch(clearToken());
  navigate('/');
};
```

## Connection Management

### Status Display

- **Connected**: Green badge with proper status
- **Offline**: Gray badge for disconnected services
- **Coming Soon**: Disabled state for future integrations

### Integration Points

- **Telegram**: Uses existing `TelegramConnectionModal` for setup
- **Redux State**: Real-time status from telegram slice
- **Component Reuse**: Leverages `connectOptions` from onboarding

### Connection Actions

```tsx
// Connect new service
const handleConnect = (serviceId: string) => {
  if (serviceId === 'telegram') {
    setTelegramModalOpen(true);
  }
  // Future: other service connection flows
};

// Disconnect service
const handleDisconnect = (serviceId: string) => {
  // Service-specific disconnection logic
};
```

## Mobile Responsiveness

### Breakpoint Behavior

- **Mobile (<640px)**: Full-screen modal with slight margins
- **Tablet (640-1024px)**: Scaled modal with backdrop
- **Desktop (>1024px)**: Fixed 520px width

### Touch Interactions

- **Minimum target size**: 48px for accessibility
- **Swipe gestures**: Down-to-close support
- **Safe areas**: iOS notch and navigation accommodation

## Accessibility Features

### Focus Management

- Trap focus within modal during interaction
- Return focus to trigger element on close
- Keyboard navigation between menu items

### ARIA Labels

- `role="dialog"` with proper modal attributes
- `aria-labelledby` for modal title
- Screen reader friendly navigation

### Keyboard Support

- **Escape key**: Close modal and return to previous page
- **Arrow keys**: Navigate between menu items
- **Enter/Space**: Activate menu items
- **Tab**: Focus trap within modal

## Integration Patterns

### Existing Component Reuse

- **Connection Options**: Reuses `connectOptions` array from `ConnectStep.tsx`
- **Modal Pattern**: Follows `TelegramConnectionModal.tsx` pattern
- **Redux Patterns**: Uses existing slice patterns and selectors

### State Management

- **No new Redux state**: Leverages existing auth, user, telegram slices
- **URL state**: Modal state driven by route parameters
- **Component state**: Local state for animations and temporary UI state

### Future Extensibility

- **Panel Structure**: Easy to add new settings panels
- **Menu Items**: Simple configuration for new settings categories
- **Service Integration**: Pattern established for new connection types

## Performance Considerations

### Code Splitting

- Settings panels lazy-loaded when accessed
- Modal infrastructure loaded on first settings access
- Minimal impact on initial app bundle size

### Animation Performance

- Hardware-accelerated CSS transforms
- Proper will-change declarations for animations
- Debounced interactions for smooth experience

### Memory Management

- Proper cleanup of event listeners and timers
- Component unmounting handled correctly
- Redux subscriptions managed efficiently

## Development Guidelines

### Adding New Settings Panels

1. Create panel component in `src/components/settings/panels/`
2. Add route in `SettingsModal.tsx` switch statement
3. Add menu item in `SettingsHome.tsx` menu array
4. Follow `ConnectionsPanel.tsx` pattern for consistency

### Styling Conventions

- Use existing Tailwind classes where possible
- Follow clean white design (not glass morphism)
- Maintain 52px height for interactive elements
- Use consistent spacing and typography scales

### Testing Patterns

- Test modal open/close functionality
- Verify URL navigation between panels
- Test Redux state integration
- Ensure mobile responsive behavior
- Validate accessibility requirements

---

_This settings modal system provides a robust, extensible foundation for app configuration while maintaining the sophisticated design standards of the crypto community platform._

# Frontend Development Guide - Crypto Community Platform

## Overview
Frontend development guide for crypto-focused communication platform using modern React ecosystem with Tauri.

## ✅ CURRENT IMPLEMENTATION STATUS

### Design System (FULLY IMPLEMENTED)
- **Glass Morphism UI**: Enhanced frosted glass effects with 16px backdrop blur throughout interface
- **Crypto Price Ticker**: Animated scrolling ticker with BTC/ETH brand colors and JetBrains Mono font
- **Navigation System**: Complete nav bar with active states (Dashboard, Portfolio, Chat, Markets)
- **Chat Interface**: Full messaging system with sent/received bubble styles and crypto addresses
- **Button Variants**: All 4 types implemented (Primary, Secondary, Success, Danger) with hover states
- **Form Components**: Enhanced inputs with focus states, select dropdowns, crypto-specific placeholders
- **Status Indicators**: Online/Offline/Warning badges with proper sage/stone/amber colors
- **Loading States**: Animated pulse placeholders for async operations
- **Typography**: Inter + JetBrains Mono fonts with crypto-optimized hierarchy
- **Color System**: Premium crypto palette (canvas, primary, sage, amber, coral, stone, market colors)
- **Animations**: Smooth transitions, hover scales, ticker animation, fade-in effects
- **Responsive Design**: Mobile-first approach with proper breakpoints
- **Accessibility**: Focus rings, proper contrast, WCAG compliance ready

## Current Project Structure (Updated)

```
src/
├── App.tsx                        # Main app with HashRouter + provider chain
├── AppRoutes.tsx                  # Route definitions with protected/public routes
├── main.tsx                       # Entry point with desktop deep link handling
├── providers/                     # Context providers
│   ├── SocketProvider.tsx         # Socket.io real-time communication
│   ├── TelegramProvider.tsx       # Telegram MTProto integration
│   └── UserProvider.tsx           # User state management
├── store/                         # Redux Toolkit state management
│   ├── index.ts                   # Store configuration with persist
│   ├── authSlice.ts              # Authentication state
│   ├── socketSlice.ts            # Socket connection state
│   ├── userSlice.ts              # User profile state
│   └── telegram/                  # Telegram state management
├── services/                      # Service layer (singletons)
│   ├── apiClient.ts              # HTTP REST client
│   ├── socketService.ts          # Socket.io client
│   └── mtprotoService.ts         # Telegram MTProto service
├── lib/mcp/                       # Model Context Protocol system
│   ├── transport.ts              # Socket.io JSON-RPC transport
│   └── telegram/                  # 99 Telegram MCP tools
├── pages/                         # Route components
│   ├── Welcome.tsx               # Landing page
│   ├── Login.tsx                 # Authentication
│   ├── Home.tsx                  # Main dashboard
│   └── onboarding/               # Multi-step onboarding
├── components/                    # Reusable UI components
│   ├── TelegramLoginButton.tsx   # OAuth login integration
│   ├── ProtectedRoute.tsx        # Auth-gated routes
│   ├── PublicRoute.tsx           # Guest-only routes
│   ├── ConnectionIndicator.tsx   # Status indicators
│   └── settings/                 # Settings modal system
│       ├── SettingsModal.tsx     # Main container with routing
│       ├── SettingsLayout.tsx    # Modal wrapper with portal
│       ├── SettingsHome.tsx      # Main menu with profile
│       ├── panels/ConnectionsPanel.tsx # Connection management
│       ├── components/           # Menu items, header, back button
│       └── hooks/                # Navigation and animation hooks
└── utils/                         # Utilities and config
    ├── config.ts                 # Environment variables
    └── desktopDeepLinkListener.ts # Deep link handling
```

### Recent Architecture Changes
- **Settings Modal System**: Complete URL-based modal system with clean white design
  - Modal routes: `/settings`, `/settings/connections` overlaying existing content
  - Component structure: SettingsModal, SettingsLayout, ConnectionsPanel, hooks
  - Redux integration: auth, user, telegram state for profile and connection management
- **HashRouter**: Switched from BrowserRouter for better desktop app compatibility
- **165+ TypeScript files**: Comprehensive component library with settings modal system
- **Provider chain**: Redux → PersistGate → Socket → Telegram → HashRouter → Routes
- **MCP Integration**: 99 Telegram tools for AI-driven interactions
- **Deep Link Auth**: Web-to-desktop handoff using `alphahuman://` scheme

## React with Tauri

### Basic Component

```tsx
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

function App() {
    const [result, setResult] = useState('');

    async function handleClick() {
        const greeting = await invoke<string>('greet', { name: 'User' });
        setResult(greeting);
    }

    return (
        <div>
            <button onClick={handleClick}>Greet</button>
            <p>{result}</p>
        </div>
    );
}

export default App;
```

## Tauri APIs

### Window Management

```typescript
import { getCurrentWindow } from '@tauri-apps/api/window';

const appWindow = getCurrentWindow();

// Minimize
await appWindow.minimize();

// Maximize
await appWindow.maximize();

// Close
await appWindow.close();

// Set title
await appWindow.setTitle('New Title');
```

### File System

First, add the plugin:
```bash
npm run tauri add fs
```

```typescript
import { readTextFile, writeTextFile, BaseDirectory } from '@tauri-apps/plugin-fs';

// Read file
const content = await readTextFile('config.json', {
    baseDir: BaseDirectory.AppData
});

// Write file
await writeTextFile('config.json', JSON.stringify(data), {
    baseDir: BaseDirectory.AppData
});
```

### Dialogs

First, add the plugin:
```bash
npm run tauri add dialog
```

```typescript
import { open, save, message } from '@tauri-apps/plugin-dialog';

// Open file picker
const filePath = await open({
    multiple: false,
    filters: [{
        name: 'Text',
        extensions: ['txt', 'md']
    }]
});

// Save dialog
const savePath = await save({
    defaultPath: 'document.txt'
});

// Message box
await message('Operation completed!', { title: 'Success' });
```

### HTTP Requests

First, add the plugin:
```bash
npm run tauri add http
```

```typescript
import { fetch } from '@tauri-apps/plugin-http';

const response = await fetch('https://api.example.com/data', {
    method: 'GET',
    headers: {
        'Content-Type': 'application/json'
    }
});

const data = await response.json();
```

## Platform Detection

```typescript
import { platform } from '@tauri-apps/plugin-os';

const currentPlatform = await platform();

switch (currentPlatform) {
    case 'windows':
        // Windows-specific UI
        break;
    case 'macos':
        // macOS-specific UI
        break;
    case 'linux':
        // Linux-specific UI
        break;
    case 'android':
        // Android-specific UI
        break;
    case 'ios':
        // iOS-specific UI
        break;
}
```

## Responsive Design for Mobile

```css
/* Base styles for mobile-first */
.container {
    padding: 16px;
    font-size: 16px;
}

/* Tablet and larger */
@media (min-width: 768px) {
    .container {
        padding: 24px;
        max-width: 720px;
        margin: 0 auto;
    }
}

/* Desktop */
@media (min-width: 1024px) {
    .container {
        max-width: 960px;
    }
}

/* Safe areas for notched devices (iOS) */
.app {
    padding-top: env(safe-area-inset-top);
    padding-bottom: env(safe-area-inset-bottom);
    padding-left: env(safe-area-inset-left);
    padding-right: env(safe-area-inset-right);
}
```

## Recommended Tech Stack

### UI & Styling
- **Tailwind CSS** - Utility-first CSS framework
- **Headless UI** - Accessible, unstyled UI components
- **Framer Motion** - Animation library for React

### State & Data Management (Current Implementation)
- **Redux Toolkit** - Currently implemented state management with persistence
- **Redux Persist** - Persists auth and telegram state to localStorage
- **Socket.io** - Real-time communication via socketService singleton
- **MTProto Client** - Telegram integration via mtprotoService singleton
- **MCP System** - 99 AI tools for Telegram interactions over Socket.io transport

## Current State Management (Redux Toolkit)

```typescript
// Current implementation uses Redux Toolkit with these slices:
import { useAppSelector, useAppDispatch } from '../store/hooks';

// Auth state (persisted)
const authState = useAppSelector((state) => state.auth);
// { token: string | null, isOnboarded: boolean }

// User state
const userState = useAppSelector((state) => state.user);
// { profile: UserProfile | null, loading: boolean, error: string | null }

// Socket state
const socketState = useAppSelector((state) => state.socket);
// { isConnected: boolean, socketId: string | null }

// Telegram state (selectively persisted)
const telegramState = useAppSelector((state) => state.telegram);
// Complex nested state with chats, messages, threads, auth status

// Usage in component
function ChatHeader() {
    const { token } = useAppSelector((state) => state.auth);
    const { profile } = useAppSelector((state) => state.user);
    const { isConnected } = useAppSelector((state) => state.socket);

    return (
        <div className="flex items-center justify-between p-4">
            <h1>Chat</h1>
            <div className="flex items-center gap-2">
                <span>{profile?.username}</span>
                <div className={`w-2 h-2 rounded-full ${isConnected ? 'bg-sage-500' : 'bg-coral-500'}`} />
            </div>
        </div>
    );
}
```

## Form Handling with React Hook Form

```typescript
import { useForm } from 'react-hook-form';
import { zodResolver } from '@hookform/resolvers/zod';
import { z } from 'zod';

const messageSchema = z.object({
    content: z.string().min(1, 'Message cannot be empty').max(1000),
    channel: z.string().uuid(),
});

type MessageForm = z.infer<typeof messageSchema>;

function MessageInput() {
    const { register, handleSubmit, reset, formState: { errors } } = useForm<MessageForm>({
        resolver: zodResolver(messageSchema)
    });

    const onSubmit = (data: MessageForm) => {
        // Send message via Tauri IPC
        invoke('send_message', data);
        reset();
    };

    return (
        <form onSubmit={handleSubmit(onSubmit)} className="flex gap-2">
            <input
                {...register('content')}
                placeholder="Type a message..."
                className="flex-1 p-2 border rounded"
            />
            <button type="submit" className="px-4 py-2 bg-blue-500 text-white rounded">
                Send
            </button>
            {errors.content && <p className="text-red-500">{errors.content.message}</p>}
        </form>
    );
}
```

## Data Fetching with TanStack Query

```typescript
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { invoke } from '@tauri-apps/api/core';

// Fetch channels
function useChannels() {
    return useQuery({
        queryKey: ['channels'],
        queryFn: () => invoke<Channel[]>('get_channels'),
        staleTime: 5 * 60 * 1000, // 5 minutes
    });
}

// Send message mutation
function useSendMessage() {
    const queryClient = useQueryClient();

    return useMutation({
        mutationFn: (message: NewMessage) => invoke('send_message', message),
        onSuccess: () => {
            // Invalidate and refetch messages
            queryClient.invalidateQueries({ queryKey: ['messages'] });
        },
    });
}

// Usage in component
function ChannelList() {
    const { data: channels, isLoading, error } = useChannels();

    if (isLoading) return <div>Loading channels...</div>;
    if (error) return <div>Error loading channels</div>;

    return (
        <div className="space-y-2">
            {channels?.map(channel => (
                <div key={channel.id} className="p-2 hover:bg-gray-100 rounded">
                    {channel.name}
                </div>
            ))}
        </div>
    );
}
```

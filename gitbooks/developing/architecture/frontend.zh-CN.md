---
description: >-
  React + Vite 前端 (`app/src/`) —— 架构、状态、服务、
  提供商、路由、组件、hook。
icon: browsers
---

# 前端 (app/src/)

OpenHuman 桌面 UI：`app/src/` 下的 Vite + React 19 树（Yarn workspace `openhuman-app`）。它使用 Redux Toolkit 配合持久化来管理会话状态，通过 REST + Socket.io 与后端通信，并通过 JSON-RPC 调用 Rust core sidecar（`coreRpcClient` / Tauri `core_rpc_relay`）。重逻辑在核心中，不在此处。

这是一份整合的参考。使用上方目录（或你的阅读器大纲）在章节间跳转。

## 快速参考

| 章节 | 涵盖内容 |
| ------------------------------------------------- | --------------------------------------------- |
| [架构](frontend.zh-CN.md#architecture-overview) | Provider 链、构建、布局、规范 |
| [状态管理](frontend.zh-CN.md#state-management) | Redux Toolkit slice、selector、持久化 |
| [服务层](frontend.zh-CN.md#services-layer) | `apiClient`、`socketService`、`coreRpcClient` |
| [Providers](frontend.zh-CN.md#providers) | `User`、`Socket`、`AI`、`Skill` providers |
| [页面与路由](frontend.zh-CN.md#pages-routing) | `HashRouter`、路由守卫、主路由 |
| [组件](frontend.zh-CN.md#components) | UI / 设置组件模式 |
| [Hook 与工具](frontend.zh-CN.md#hooks-utilities) | 共享 hook、辅助函数、配置 |

## 规模

| 指标 | 值 |
| --------------------------------------- | ------------------------------------------------------------------------ |
| `app/src/` 下的 TypeScript / TSX 文件 | \~285 (`find app/src -name '*.ts' -o -name '*.tsx' \| wc -l` 刷新) |
| 测试 runner | Vitest (`app/test/vitest.config.ts`) |

## 目录布局

```text
app/src/
├── App.tsx                 # Provider 链 + HashRouter shell
├── AppRoutes.tsx           # 路由表 + 守卫
├── main.tsx                # 入口 (Sentry、store、样式)
├── store/                  # Redux slice 和 selector
├── providers/              # UserProvider、SocketProvider、AIProvider、SkillProvider
├── services/               # apiClient、socketService、coreRpcClient、api/*
├── lib/                    # AI loader、MCP 辅助函数、技能同步等
├── pages/                  # 路由级页面
├── components/             # 共享 UI
├── hooks/                  # 应用 hook
├── utils/                  # 配置、Tauri 辅助函数、路由工具
└── assets/                 # 图标和静态资源
```

## 架构概览

### 系统架构

OpenHuman 的桌面 UI 是一个 **React 19** 应用 (`app/src/`)，它：

* 使用 **Redux Toolkit** 配合持久化来管理与会话相关的状态
* 通过 **REST** (`apiClient`) 和 **Socket.io** (`socketService`) 连接后端
* 通过 **`coreRpcClient`** / Tauri **`core_rpc_relay`** 调用 **Rust 核心**进程（JSON-RPC 方法实现在仓库根目录 `src/openhuman/` 中，通过 `core_server` 暴露）
* 从捆绑的 `src/openhuman/agent/prompts`（仓库根目录）和打包时的 Tauri **`ai_get_config`** 加载 **AI 提示**
* 在 `lib/mcp/` 下使用 **最小 MCP 风格**辅助层（传输、验证），而非大型的仓库内 Telegram MCP 工具包

### 入口点

| 文件 | 用途 |
| ----------------------- | ------------------------------------------------------------------------------------ |
| `app/src/main.tsx` | React 根节点、Sentry 边界、store、全局样式 |
| `app/src/App.tsx` | Provider 链：Redux → PersistGate → User → Socket → AI → Skill → Router |
| `app/src/AppRoutes.tsx` | `HashRouter` 路由、`ProtectedRoute` / `PublicRoute`、onboarding 和 mnemonic 门禁 |

### Provider 链

```text
Redux Provider
  └─ PersistGate
      └─ UserProvider
          └─ SocketProvider
              └─ AIProvider
                  └─ SkillProvider
                      └─ HashRouter
                          └─ AppRoutes (pages + settings)
```

**为什么是这个顺序**

1. Redux 在最外层，以便到处使用 `useAppSelector` / dispatch。
2. `PersistGate` 在子组件假设稳定认证前重新水合持久化的 slice。
3. `SocketProvider` 使用 auth token 进行 Socket.io。
4. `AIProvider` / `SkillProvider` 包装依赖 socket 和 store 状态的功能。
5. `HashRouter` 为所有路由提供导航。

### 模块关系（简化）

```text
App.tsx
  ├─ Redux store + persistor
  ├─ UserProvider - 用户 profile / workspace 上下文
  ├─ SocketProvider - token 存在时连接 socketService
  ├─ AIProvider - AI 会话 / 记忆客户端协调
  ├─ SkillProvider - 技能目录和同步
  └─ AppRoutes
       ├─ PublicRoute - 例如 `/` 上的 Welcome
       ├─ ProtectedRoute - onboarding、home、skills、settings、…
       └─ DefaultRedirect - 未认证用户
```

### 服务层（概念性）

```text
services/
  ├─ apiClient        → 通过运行时解析的 URL 的 REST，使用 `services/backendUrl#getBackendUrl`
  ├─ backendUrl       → 调用 `openhuman.config_resolve_api_url`；仅在 Tauri 外 fallback 到 VITE_BACKEND_URL
  ├─ socketService    → Socket.io；实时 + MCP 风格信封
  └─ coreRpcClient    → 本地 openhuman 核心的 HTTP (JSON-RPC)，配合 Tauri relay 使用
```

#### 运行时配置优先级

桌面应用不会将核心 RPC URL 或 API 主机作为硬性要求烘焙到 bundle 中。运行时应用按此顺序解析它们（最高优先）：

1. **登录屏幕 RPC URL 字段**，通过 `utils/configPersistence` 保存并在下次启动时恢复。终端用户在此配置 sidecar 地址，而非手动编辑 `config.toml` 或 `.env` 文件。
2. **Tauri `core_rpc_url` 命令**，bundled sidecar 为本进程监听的端口。
3. **`VITE_OPENHUMAN_CORE_RPC_URL`**，开发时的构建时 fallback。
4. 硬编码的 `http://127.0.0.1:7788/rpc` 默认值。

RPC 握手成功后，`services/backendUrl` 调用 `openhuman.config_resolve_api_url` 从加载的核心 `Config` 中拉取 `api_url`（和其他安全客户端字段）。`VITE_BACKEND_URL` 仅在应用运行在 Tauri 外时作为 Web fallback 使用。

需要后端 URL 的组件应调用 `useBackendUrl()`（或非 React 代码调用 `getBackendUrl()`），它们绝不能从 `utils/config` 导入静态的 `BACKEND_URL` 常量，那只代表构建时值。

### 相关文档

* Rust 架构：[架构](../architecture.zh-CN.md)
* Tauri 壳层：[Tauri Shell](tauri-shell.zh-CN.md)

## 状态管理

应用使用 Redux Toolkit 配合 Redux-Persist 进行健壮的状态管理。

### Store 配置

**文件：** `store/index.ts`

```typescript
// 合并所有 slice 并持久化
const persistConfig = {
  key: 'root',
  storage,
  whitelist: ['auth', 'telegram'], // 持久化的 slice
};
```

### Redux 状态结构

```typescript
RootState = {
  auth: {
    token: string | null, // JWT (持久化)
    isOnboardedByUser: Record<string, boolean>, // 每用户 flag (持久化)
  },
  socket: {
    byUser: Record<
      string,
      {
        // 每用户 ID
        status: 'connecting' | 'connected' | 'disconnected';
        socketId: string | null;
      }
    >,
  },
  user: { profile: User | null, loading: boolean, error: string | null },
  telegram: {
    byUser: Record<string, TelegramState>, // 每 Telegram 用户 (持久化)
  },
};
```

### Slice

#### Auth Slice (`store/authSlice.ts`)

管理 JWT token 和每用户 onboarding 状态。

**状态：**

```typescript
interface AuthState {
  token: string | null;
  isOnboardedByUser: Record<string, boolean>;
}
```

**Actions：**

* `setToken(token: string)` - 登录后存储 JWT
* `clearToken()` - 登出时移除 token
* `setOnboarded({ userId, isOnboarded })` - 将用户标记为已 onboard

**Selectors (`store/authSelectors.ts`)：**

* `selectToken` - 获取当前 JWT
* `selectIsOnboarded(userId)` - 检查用户是否完成 onboarding

#### Socket Slice (`store/socketSlice.ts`)

跟踪每用户的 Socket.io 连接状态。

**状态：**

```typescript
interface SocketState {
  byUser: Record<
    string,
    { status: 'connecting' | 'connected' | 'disconnected'; socketId: string | null }
  >;
}
```

**Actions：**

* `setSocketStatus({ userId, status })` - 更新连接状态
* `setSocketId({ userId, socketId })` - 存储 socket ID
* `clearSocketState(userId)` - 清除用户 socket 状态

**Selectors (`store/socketSelectors.ts`)：**

* `selectSocketStatus(userId)` - 获取连接状态
* `selectIsSocketConnected(userId)` - 布尔连接检查

#### User Slice (`store/userSlice.ts`)

存储用户 profile 数据。

**状态：**

```typescript
interface UserState {
  profile: User | null;
  loading: boolean;
  error: string | null;
}
```

**Actions：**

* `setUser(user)` - 存储用户 profile
* `setUserLoading(loading)` - 设置加载状态
* `setUserError(error)` - 设置错误状态
* `clearUser()` - 登出时清除 profile

#### Telegram Slice (`store/telegram/`)

Telegram 集成的复杂嵌套状态管理。

**文件：**

* `index.ts` - Slice 导出（actions、thunks）
* `types.ts` - 实体和状态接口
* `reducers.ts` - 同步 reducers
* `extraReducers.ts` - 异步 thunk handlers
* `thunks.ts` - 异步操作

**状态结构：**

```typescript
telegram.byUser[telegramUserId] = {
  connectionStatus: "disconnected" | "connecting" | "connected" | "error",
  authStatus: "not_authenticated" | "authenticating" | "authenticated" | "error",
  currentUser: TelegramUser | null,
  sessionString: string | null,              // 存储在这里，而非 localStorage
  chats: Record<string, TelegramChat>,
  chatsOrder: string[],
  messages: Record<chatId, Record<msgId, TelegramMessage>>,
  threads: Record<chatId, TelegramThread[]>
}
```

**Reducers：**

* `setCurrentUser` - 存储已认证的 Telegram 用户
* `setSessionString` - 存储 MTProto 会话（用于持久化）
* `setConnectionStatus` - 更新连接状态
* `setAuthStatus` - 更新认证状态
* `addChat` / `updateChat` - 管理聊天列表
* `addMessage` / `updateMessage` - 管理消息历史
* `setThreads` - 存储 thread 数据

**Thunks (`store/telegram/thunks.ts`)：**

* `initializeTelegram(userId)` - 初始化 MTProto 客户端
* `connectTelegram(userId)` - 建立 Telegram 连接
* `fetchChats(userId)` - 加载聊天列表
* `fetchMessages({ userId, chatId })` - 加载消息历史
* `disconnectTelegram(userId)` - 干净断开

**Selectors (`store/telegramSelectors.ts`)：**

* `selectTelegramState(userId)` - 获取完整 Telegram 状态
* `selectTelegramConnectionStatus(userId)` - 获取连接状态
* `selectTelegramAuthStatus(userId)` - 获取 auth 状态
* `selectTelegramChats(userId)` - 获取聊天列表
* `selectTelegramMessages(userId, chatId)` - 获取聊天的消息

### Typed Hooks

**文件：** `store/hooks.ts`

```typescript
// 使用这些代替普通的 useDispatch/useSelector
export const useAppDispatch: () => AppDispatch = useDispatch;
export const useAppSelector: TypedUseSelectorHook<RootState> = useSelector;
```

### 持久化配置

#### 什么被持久化

* `auth.token` - 用于认证的 JWT
* `auth.isOnboardedByUser` - 每用户 onboarding 状态
* `telegram.byUser` - Telegram 状态（会话、聊天等）

#### 什么**不**被持久化

* `socket` - 连接状态（应用启动时重连）
* `user.loading` / `user.error` - 瞬态 UI 状态
* Telegram 加载/错误状态

#### 存储后端

Redux-Persist 默认使用 localStorage adapter。这是应用中唯一可接受的 localStorage 使用。

### 使用示例

#### 读取状态

```typescript
import { useAppSelector } from '../store/hooks';

function MyComponent() {
  const token = useAppSelector(state => state.auth.token);
  const isConnected = useAppSelector(state => state.socket.byUser[userId]?.status === 'connected');
  const chats = useAppSelector(state => state.telegram.byUser[userId]?.chats);
}
```

#### Dispatch Actions

```typescript
import { clearToken, setToken } from '../store/authSlice';
import { useAppDispatch } from '../store/hooks';
import { initializeTelegram } from '../store/telegram/thunks';

function MyComponent() {
  const dispatch = useAppDispatch();

  // 同步 action
  const handleLogin = (token: string) => {
    dispatch(setToken(token));
  };

  // 异步 thunk
  const handleConnect = async () => {
    await dispatch(initializeTelegram(userId)).unwrap();
  };
}
```

#### 使用 Selectors

```typescript
import { selectIsOnboarded } from '../store/authSelectors';
import { useAppSelector } from '../store/hooks';
import { selectTelegramConnectionStatus } from '../store/telegramSelectors';

function MyComponent({ userId }) {
  const isOnboarded = useAppSelector(state => selectIsOnboarded(state, userId));
  const connectionStatus = useAppSelector(state => selectTelegramConnectionStatus(state, userId));
}
```

### 最佳实践

1. **始终使用 typed hooks** - `useAppDispatch` 和 `useAppSelector`
2. **使用 selector 处理派生状态** - 可记忆且可测试
3. **将 thunks 放在单独文件中** - 更好的组织
4. **每用户状态作用域** - 按用户 ID 键控状态
5. **避免 localStorage** - 改用 Redux-Persist

***

## 服务层

应用使用单例服务进行外部通信。这防止连接泄漏并提供一致的 API 访问。

### 服务架构

```text
app/src/services/
  ├─ apiClient (HTTP REST)
  │   ├─ 从 Redux 读取 auth.token
  ��   └─ 调用 VITE_BACKEND_URL（见 utils/config.ts）
  ├─ socketService (Socket.io)
  │   ├─ web: JS 客户端
  │   └─ Tauri: 通过 utils/tauriSocket.ts 与 Rust 端 socket 协调
  ├─ coreRpcClient.ts
  │   └─ invoke('core_rpc_relay', …) → 本地 openhuman 核心 (JSON-RPC)
  └─ services/api/* - 领域 REST 模块 (auth、user、teams、…)
```

### API Client (`services/apiClient.ts`)

用于后端通信的 HTTP REST 客户端。

#### 特性

* 基于 Fetch 的实现
* 自动从 Redux store 注入 JWT
* 类型化的请求/响应处理
* 带类型错误的错误处理

#### 用法

```typescript
import apiClient from "../services/apiClient";

// GET 请求
const user = await apiClient.get<User>("/users/me");

// POST 请求
const result = await apiClient.post<LoginResponse>("/auth/login", {
  email,
  password,
});

// 带自定义头
const data = await apiClient.get<Data>("/endpoint", {
  headers: { "X-Custom": "value" },
});
```

#### 配置

从环境读取 `VITE_BACKEND_URL` 或使用默认值：

```typescript
const BACKEND_URL =
  import.meta.env.VITE_BACKEND_URL || "https://api.example.com";
```

### API Endpoints (`services/api/`)

#### Auth API (`services/api/authApi.ts`)

认证相关端点。

```typescript
import { authApi } from "../services/api/authApi";

// 登录
const { token, user } = await authApi.login(credentials);

// Token 交换（用于深度链接流程）
const { sessionToken, user } = await authApi.exchangeToken(loginToken);

// 登出
await authApi.logout();
```

#### User API (`services/api/userApi.ts`)

用户 profile 端点。

```typescript
import { userApi } from "../services/api/userApi";

// 获取当前用户
const user = await userApi.getCurrentUser();

// 更新 profile
const updated = await userApi.updateProfile({ firstName, lastName });

// 获取设置
const settings = await userApi.getSettings();
```

### Socket Service (`services/socketService.ts`)

用于实时通信的 Socket.io 客户端单例。

#### 特性

* 单例模式 - 每应用一个连接
* Auth token 通过 socket `auth` 对象传递
* 传输：先 polling，然后 WebSocket 升级
* 自动重连处理

#### API

```typescript
import socketService from "../services/socketService";

// 用 auth token 连接
socketService.connect(token);

// 断开
socketService.disconnect();

// 发射事件
socketService.emit("event-name", data);

// 监听事件
socketService.on("event-name", (data) => {
  // 处理事件
});

// 移除监听器
socketService.off("event-name", handler);

// 一次性监听器
socketService.once("event-name", (data) => {
  // 处理一次
});

// 获取 socket 实例
const socket = socketService.getSocket();

// 检查连接状态
const isConnected = socketService.isConnected();
```

#### 连接流程

```typescript
// 在 SocketProvider.tsx 中
useEffect(() => {
  if (token) {
    socketService.connect(token);

    socketService.on("connect", () => {
      dispatch(setSocketStatus({ userId, status: "connected" }));
      dispatch(setSocketId({ userId, socketId: socket.id }));
      // 初始化 MCP 服务器
      initMCPServer(socketService.getSocket());
    });

    socketService.on("disconnect", () => {
      dispatch(setSocketStatus({ userId, status: "disconnected" }));
    });
  }

  return () => {
    socketService.disconnect();
  };
}, [token]);
```

#### 配置

```typescript
const socket = io(BACKEND_URL, {
  auth: { token },
  transports: ["polling", "websocket"],
  reconnection: true,
  reconnectionAttempts: 5,
  reconnectionDelay: 1000,
});
```

#### Socket 事件契约 (Tauri)

在 Tauri 模式下，连接和事件通过 **`utils/tauriSocket.ts`** (`setupTauriSocketListeners`、`connectRustSocket` 等) 桥接。见 `providers/SocketProvider.tsx` 获取完整流程（包括 daemon 生命周期 hook）。

### Core RPC (`services/coreRpcClient.ts`)

桌面应用运行一个单独的 **`openhuman`** Rust 二进制文件（staging 在 `app/src-tauri/binaries/` 下）。UI 通过 Tauri 调用该进程上的 JSON-RPC 方法：

```typescript
import { callCoreRpc } from "../services/coreRpcClient";

const result = await callCoreRpc<MyType>({
  method: "some.openhuman.method",
  params: {
    /* … */
  },
  serviceManaged: false, // true 如果 relay 应确保 systemd/launchd 风格服务
});
```

实现：`invoke('core_rpc_relay', { request: { method, params, serviceManaged } })` → `app/src-tauri/src/commands/core_relay.rs` → `app/src-tauri/src/core_rpc.rs` 中的 HTTP 客户端。

### 服务与 provider 集成

#### SocketProvider

`app/src/providers/SocketProvider.tsx` 在 `auth.token` 存在时连接。在 **Tauri** 中，它优先使用 Rust-backed socket 路径；在 **web** 中，它使用 JS Socket.io 客户端。见源码获取日志和 `useDaemonLifecycle` 集成。

#### UserProvider、AIProvider、SkillProvider

这些包装用户 profile 加载、AI/记忆客户端协调和技能目录/同步。它们位于 `PersistGate` **内部** 和路由器旁边或外部，如 `App.tsx` 所示。

### 最佳实践

1. **使用单例** - 永远不要创建多个服务实例
2. **在 Redux 中存储会话** - 不用 localStorage
3. **卸载时清理** - 在 useEffect cleanup 中断开连接
4. **优雅处理错误** - 瞬态失败时重试
5. **通过正确通道传递 auth** - Socket auth 对象，而非 query string

***

## Providers

React context providers 管理服务生命周期并提供共享状态。

### Provider 链

providers 按特定顺序包装应用 (`app/src/App.tsx`)：

```tsx
<Sentry.ErrorBoundary>
  <Provider store={store}>
    <PersistGate persistor={persistor} onBeforeLift={...}>
      <UserProvider>
        <SocketProvider>
          <AIProvider>
            <SkillProvider>
              <Router>
                <AppRoutes />
              </Router>
            </SkillProvider>
          </AIProvider>
        </SocketProvider>
      </UserProvider>
    </PersistGate>
  </Provider>
</Sentry.ErrorBoundary>
```

(`Router` 是 `react-router-dom` 的 `HashRouter`。)

**顺序重要，因为：**

1. Redux 在最外层用于 store 访问。
2. `PersistGate` 在子组件依赖 auth 前重新水合持久化的 slice。
3. `SocketProvider` 使用 store 中的 JWT。
4. `AIProvider` / `SkillProvider` 依赖 socket 和 store-backed 功能。
5. 路由器为所有路由提供导航。

### SocketProvider (`app/src/providers/SocketProvider.tsx`)

管理实时连接：**web** 使用 JS Socket.io 客户端；**Tauri** 通过 `utils/tauriSocket.ts` 桥接到 Rust socket 并向 Redux 报告状态。

#### 职责

* `auth.token` 可用时连接；清除时断开
* Tauri 中：安装监听器一次，连接 Rust socket，协调 daemon 生命周期 (`useDaemonLifecycle`)
* 更新 Redux socket slice / 连接状态

#### 实现

见 **`app/src/providers/SocketProvider.tsx`**。文件在 **`isTauri()`** 上分叉：web 模式直接使用 `socketService`；Tauri 设置 `tauriSocket` 监听器和 `connectRustSocket` / `disconnectRustSocket`。不要将下方的伪代码视为实时实现。

#### 用法

```typescript
import { useSocket } from '../providers/SocketProvider';

function MyComponent() {
  const { socket, isConnected, emit, on, off } = useSocket();

  useEffect(() => {
    const handler = (data) => console.log('Received:', data);
    on('event-name', handler);
    return () => off('event-name', handler);
  }, [on, off]);

  const sendMessage = () => {
    emit('send-message', { text: 'Hello!' });
  };

  return (
    <div>
      <span>Status: {isConnected ? 'Connected' : 'Disconnected'}</span>
      <button onClick={sendMessage}>Send</button>
    </div>
  );
}
```

### AIProvider (`app/src/providers/AIProvider.tsx`)

初始化 **memory**、**sessions**、**tool registry**（包括 memory + web-search 工具）、**entity manager**、**LLM / embedding providers** 和 **constitution** 加载。为子组件暴露 `useAI()`。重逻辑位于 `app/src/lib/ai/` 下。

### SkillProvider (`app/src/providers/SkillProvider.tsx`)

挂载时（认证后），通过 Tauri 辅助函数 (`runtimeDiscoverSkills`) 从 **QuickJS** 技能引擎发现技能，将 manifest 同步到 Redux，监听技能相关的 Tauri 事件，并可以在开发中自动启动配置的技能。

### UserProvider (`providers/UserProvider.tsx`)

最小用户 context provider（大多数用户状态在 Redux 中）。

#### 职责

* 兼容性用的遗留用户 context
* 可能弃用，改为 Redux

#### 实现

```typescript
interface UserContextValue {
  user: User | null;
  loading: boolean;
}

export function UserProvider({ children }) {
  const user = useAppSelector((state) => state.user.profile);
  const loading = useAppSelector((state) => state.user.loading);

  return (
    <UserContext.Provider value={{ user, loading }}>
      {children}
    </UserContext.Provider>
  );
}
```

#### 用法

```typescript
import { useUserContext } from '../providers/UserProvider';

function Header() {
  const { user, loading } = useUserContext();

  if (loading) return <Skeleton />;
  if (!user) return null;

  return <span>Welcome, {user.firstName}</span>;
}
```

### Provider 模式

#### 基于 Effect 的生命周期

Providers 使用 `useEffect` 管理服务生命周期：

```typescript
useEffect(() => {
  // 挂载或依赖变更时设置
  service.connect();

  // 卸载或依赖变更时清理
  return () => {
    service.disconnect();
  };
}, [dependencies]);
```

#### Redux 集成

Providers 从 Redux 读取并 dispatch：

```typescript
// 读取状态
const token = useAppSelector((state) => state.auth.token);

// Dispatch actions
const dispatch = useAppDispatch();
dispatch(setStatus({ userId, status: "connected" }));
```

#### 并行初始化

`SkillProvider` 和 `AIProvider` 可能在挂载时启动多个异步任务（技能发现、记忆初始化、constitution 加载）。优先阅读源码获取排序保证，而非假设到处都是并行 `Promise.all`。

#### 会话恢复

Providers 在挂载时恢复持久化状态：

```typescript
useEffect(() => {
  if (persistedSession) {
    service.restoreSession(persistedSession);
  }
}, [persistedSession]);
```

### Context vs Redux

| 使用 Context 用于 | 使用 Redux 用于 |
| ---------------------------------- | ---------------------------------- |
| 服务实例 (socket、client) | 可序列化状态 (status、data) |
| 方法 (emit、on、off) | 持久化状态 (sessions、tokens) |
| 派生值 | 复杂状态逻辑 |

示例：

* `SocketContext` 提供 `socket` 实例和 `emit` 方法
* Redux 存储 `socketStatus` 和 `socketId`

### 测试 Providers

#### 测试用的 Mock Provider

```typescript
// test-utils.tsx
const mockSocketContext: SocketContextValue = {
  socket: null,
  isConnected: true,
  emit: jest.fn(),
  on: jest.fn(),
  off: jest.fn()
};

export function TestProviders({ children }) {
  return (
    <Provider store={testStore}>
      <SocketContext.Provider value={mockSocketContext}>
        {children}
      </SocketContext.Provider>
    </Provider>
  );
}
```

#### 测试 Provider Effects

```typescript
test('SocketProvider 在 token 可用时连接', () => {
  const store = createTestStore({ auth: { token: 'test-token' } });

  render(
    <Provider store={store}>
      <SocketProvider>
        <TestComponent />
      </SocketProvider>
    </Provider>
  );

  expect(socketService.connect).toHaveBeenCalledWith('test-token');
});
```

***

## Human Mascot 表面

Human 页面 (`app/src/features/human/HumanPage.tsx`) 在对话侧边栏旁渲染主
`YellowMascot`。mascot  face 仍然来自 `useHumanMascot`，它订阅聊天生命周期事件以获取 thinking、
speaking、acknowledgement 和 error 状态。

子智能体委托由 `SubMascotLayer` 可视化。它不引入新的 socket 协议。相反，它读取已选或活跃 thread 的
`chatRuntime.toolTimelineByThread` 条目，`ChatRuntimeProvider` 已经从
`subagent_spawned`、`subagent_completed`、`subagent_failed`、
`subagent_iteration_start`、`subagent_tool_call` 和 `subagent_tool_result` 构建了这些条目。

生命周期映射：

| Runtime timeline 状态 | Sub-mascot 状态 |
| ---------------------- | ---------------- |
| `running` | 带 thinking face 和短活动气泡的小型彩色 mascot |
| `success` | 相同 mascot 解析为 happy face 和完成气泡 |
| `error` | 相同 mascot 解析为 concerned face 和失败气泡 |

活动气泡文本有意保持紧凑：当前子工具调用、子迭代、委托提示摘录或最终状态。Thread timeline 仍然是权威的详细视图；sub-mascot 只是主 mascot 周围可一瞥的编排层。

***

## 页面与路由

应用使用 HashRouter 配合受保护和公共路由守卫。

### 路由结构

在 **`app/src/AppRoutes.tsx`** (HashRouter) 中定义。近似映射：

```text
/                  → Welcome (公共包装器)
/onboarding        → Onboarding (auth，onboarding 未完成)
/mnemonic          → Mnemonic / 加密设置 (auth)
/home              → Home (auth + onboarding + 加密密钥)
/intelligence      → Intelligence (auth)
/skills            → Skills (auth)
/conversations     → Conversations (auth)
/invites           → Invites (auth)
/agents            → Agents (auth)
/settings/*        → Settings (auth)
*                  → DefaultRedirect
```

`AppRoutes` 中**没有**顶级 `/login` 路由；认证流程通过 welcome/onboarding 和后端重定向处理。

### 路由配置 (`AppRoutes.tsx`)

```typescript
export function AppRoutes() {
  return (
    <>
      <Routes>
        {/* 公共路由 - 已认证时重定向 */}
        <Route element={<PublicRoute />}>
          <Route path="/" element={<Welcome />} />
          <Route path="/login" element={<Login />} />
        </Route>

        {/* 受保护路由 - 需要认证 */}
        <Route element={<ProtectedRoute />}>
          <Route path="/onboarding/*" element={<Onboarding />} />
        </Route>

        {/* 受保护 + 已 onboard 路由 */}
        <Route element={<ProtectedRoute requireOnboarded />}>
          <Route path="/home" element={<Home />} />
        </Route>

        {/* Fallback 重定向 */}
        <Route path="*" element={<DefaultRedirect />} />
      </Routes>

      {/* 设置模态覆盖层 - 在路由之上渲染 */}
      <SettingsModal />
    </>
  );
}
```

### 路由守卫

#### PublicRoute (`components/PublicRoute.tsx`)

将已认证用户从公共页面重定向走。

```typescript
export function PublicRoute() {
  const token = useAppSelector((state) => state.auth.token);
  const isOnboarded = useAppSelector((state) =>
    selectIsOnboarded(state, userId),
  );

  if (token) {
    // 已认证 - 重定向到适当页面
    return <Navigate to={isOnboarded ? "/home" : "/onboarding"} replace />;
  }

  return <Outlet />;
}
```

#### ProtectedRoute (`components/ProtectedRoute.tsx`)

强制执行认证和可选的 onboarding 状态。

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

#### DefaultRedirect (`components/DefaultRedirect.tsx`)

基于 auth 状态的 fallback 路由。

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

### 页面

#### Welcome 页面 (`pages/Welcome.tsx`)

未认证用户的落地页。

**特性：**

* 应用介绍和品牌
* 登录/注册 CTA
* 公共路由（已认证时重定向）

#### Login 页面 (`pages/Login.tsx`)

认证页面。

**特性：**

* Telegram OAuth 按钮
* 在浏览器中打开 `/auth/telegram?platform=desktop`
* 处理深度链接回调

```typescript
export function Login() {
  const handleTelegramLogin = () => {
    // 在系统浏览器中打开 Telegram OAuth
    openUrl(`${BACKEND_URL}/auth/telegram?platform=desktop`);
  };

  return (
    <div className="login-page">
      <TelegramLoginButton onClick={handleTelegramLogin} />
    </div>
  );
}
```

#### Home 页面 (`pages/Home.tsx`)

认证后的主仪表板。

**特性：**

* 受保护路由（需要 auth + onboarded）
* 连接状态指示器
* 导航到设置模态
* 未来：聊天列表、消息等

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

      {/* 主内容 */}
    </div>
  );
}
```

### Onboarding 流程 (`pages/onboarding/`)

多步 onboarding 流程。

#### 结构

```text
pages/onboarding/
├── Onboarding.tsx           # 流程控制器
└── steps/
    ├── GetStartedStep.tsx   # Welcome
    ├── PrivacyStep.tsx      # 隐私政策
    ├── AnalyticsStep.tsx    # Analytics 选择加入
    ├── ConnectStep.tsx      # Telegram 连接
    └── FeaturesStep.tsx     # 特性概览
```

#### Onboarding 控制器 (`Onboarding.tsx`)

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
      // 完成 onboarding
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

#### Step 组件

每个 step 接收 `onNext` 和 `onBack` 回调：

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

### 设置模态路由

设置模态使用基于 URL 的路由覆盖现有内容。

#### 模态检测

```typescript
// 在 SettingsModal.tsx 中
const location = useLocation();
const isOpen = location.pathname.startsWith("/settings");
```

#### 子路由

```text
/settings              → SettingsHome (主菜单)
/settings/connections  → ConnectionsPanel
/settings/messaging    → MessagingPanel (未来)
/settings/privacy      → PrivacyPanel (未来)
/settings/profile      → ProfilePanel (未来)
/settings/advanced     → AdvancedPanel (未来)
/settings/billing      → BillingPanel (未来)
```

#### 导航

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

### HashRouter vs BrowserRouter

应用使用 HashRouter 以兼容桌面：

```typescript
// App.tsx
import { HashRouter } from "react-router-dom";

// URL 看起来像这样：app://localhost/#/home
// 而不是：app://localhost/home
```

**为什么用 HashRouter：**

1. Tauri 深度链接与基于 hash 的 URL 配合工作
2. 不需要服务器配置
3. 与 file:// 协议配合工作
4. 防止直接 URL 访问时的 404

### 深度链接处理

深度链接在路由前处理：

```typescript
// main.tsx
import("./utils/desktopDeepLinkListener").then((m) => {
  m.setupDesktopDeepLinkListener().catch(console.error);
});
```

监听器拦截 `openhuman://auth?token=...` 并：

1. 通过 Rust 命令交换 token
2. 在 Redux 中存储会话
3. 导航到 `/onboarding` 或 `/home`

### 导航模式

#### 程序化导航

```typescript
import { useNavigate } from "react-router-dom";

const navigate = useNavigate();

// 导航到路由
navigate("/home");

// 替换历史条目
navigate("/login", { replace: true });

// 返回
navigate(-1);
```

#### Link 组件

```typescript
import { Link } from "react-router-dom";

<Link to="/settings">Settings</Link>;
```

#### 状态传递

```typescript
// 向路由传递状态
navigate("/details", { state: { itemId: 123 } });

// 接收状态
const location = useLocation();
const { itemId } = location.state;
```

***

## 组件

按功能组织的可复用 React 组件。

### 组件结构

```text
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

### 路由守卫组件

#### ProtectedRoute

需要认证和可选的 onboarding。

```typescript
interface ProtectedRouteProps {
  requireOnboarded?: boolean;
}

// 在 AppRoutes.tsx 中的用法
<Route element={<ProtectedRoute />}>
  <Route path="/onboarding/*" element={<Onboarding />} />
</Route>

<Route element={<ProtectedRoute requireOnboarded />}>
  <Route path="/home" element={<Home />} />
</Route>
```

#### PublicRoute

将已认证用户重定向走。

```typescript
// 在 AppRoutes.tsx 中的用法
<Route element={<PublicRoute />}>
  <Route path="/" element={<Welcome />} />
  <Route path="/login" element={<Login />} />
</Route>
```

#### DefaultRedirect

基于 auth 状态的 fallback。

```typescript
// 重定向到：
// - "/" 如果未认证
// - "/onboarding" 如果已认证但未 onboard
// - "/home" 如果已认证且已 onboard
```

### 认证组件

#### TelegramLoginButton

Telegram 的 OAuth 登录按钮。

```typescript
interface TelegramLoginButtonProps {
  onClick: () => void;
  disabled?: boolean;
}

// 用法
<TelegramLoginButton
  onClick={() => openUrl(`${BACKEND_URL}/auth/telegram?platform=desktop`)}
/>
```

### 连接状态组件

#### ConnectionIndicator

通用连接状态徽章。

```typescript
interface ConnectionIndicatorProps {
  status: 'connected' | 'connecting' | 'disconnected' | 'error';
  label?: string;
}

<ConnectionIndicator status="connected" label="Socket" />
```

#### TelegramConnectionIndicator

Telegram 特定的状态显示。

```typescript
interface TelegramConnectionIndicatorProps {
  status: 'connected' | 'connecting' | 'disconnected' | 'error';
}

// 配合 Redux 状态使用
const telegramStatus = useAppSelector((state) =>
  selectTelegramConnectionStatus(state, userId)
);

<TelegramConnectionIndicator status={telegramStatus} />
```

#### TelegramConnectionModal

设置 Telegram 连接的模态。

```typescript
interface TelegramConnectionModalProps {
  isOpen: boolean;
  onClose: () => void;
}

// 在 onboarding/settings 中的用法
const [showModal, setShowModal] = useState(false);

<TelegramConnectionModal
  isOpen={showModal}
  onClose={() => setShowModal(false)}
/>
```

**特性：**

* QR 码登录流程
* 手机号登录流程
* 连接状态显示
* 错误处理

#### GmailConnectionIndicator

Gmail 状态徽章（未来集成）。

```typescript
<GmailConnectionIndicator status="coming-soon" />
```

### Onboarding 组件

#### ProgressIndicator

通过 onboarding step 的视觉进度。

```typescript
interface ProgressIndicatorProps {
  current: number;
  total: number;
}

<ProgressIndicator current={2} total={5} />
```

#### LottieAnimation

Onboarding 的 Lottie 动画播放器。

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

### 设置模态系统

带基于 URL 路由的完整模态系统。

#### 文件结构

```text
components/settings/
├── SettingsModal.tsx          # 基于路由的容器
├── SettingsLayout.tsx         # Portal + 背景包装器
├── SettingsHome.tsx           # 带 profile 的主菜单
├── panels/
│   ├── ConnectionsPanel.tsx   # 连接管理
│   ├── MessagingPanel.tsx     # (未来)
│   ├── PrivacyPanel.tsx       # (未来)
│   ├── ProfilePanel.tsx       # (未来)
│   ├── AdvancedPanel.tsx      # (未来)
│   └── BillingPanel.tsx       # (未来)
├── components/
│   ├── SettingsHeader.tsx     # 用户 profile 部分
│   ├── SettingsMenuItem.tsx   # 菜单项组件
│   ├── SettingsBackButton.tsx # 返回导航
│   └── SettingsPanelLayout.tsx# Panel 包装器
└── hooks/
    ├── useSettingsNavigation.ts # URL 路由
    └── useSettingsAnimation.ts  # 动画状态
```

#### SettingsModal

基于 URL 渲染的主容器。

```typescript
export function SettingsModal() {
  const location = useLocation();
  const isOpen = location.pathname.startsWith('/settings');

  if (!isOpen) return null;

  return (
    <SettingsLayout>
      {/* 路由到适当的 panel */}
      {location.pathname === '/settings' && <SettingsHome />}
      {location.pathname === '/settings/connections' && <ConnectionsPanel />}
      {/* ... 更多 panels */}
    </SettingsLayout>
  );
}
```

#### SettingsLayout

基于 Portal 的模态包装器。

```typescript
export function SettingsLayout({ children }) {
  const { closeModal } = useSettingsNavigation();

  return createPortal(
    <div className="fixed inset-0 z-50">
      {/* 背景 */}
      <div
        className="absolute inset-0 bg-black/50 backdrop-blur-sm"
        onClick={closeModal}
      />

      {/* 模态 */}
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

#### SettingsHome

带用户 profile 的主菜单。

```typescript
export function SettingsHome() {
  const { navigateTo, closeModal } = useSettingsNavigation();
  const user = useAppSelector((state) => state.user.profile);

  const menuItems = [
    { id: 'connections', label: 'Connections', icon: LinkIcon },
    { id: 'messaging', label: 'Messaging', icon: MessageIcon },
    { id: 'privacy', label: 'Privacy', icon: ShieldIcon },
    // ... 更多项
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

#### ConnectionsPanel

连接管理界面。

```typescript
export function ConnectionsPanel() {
  const { navigateBack } = useSettingsNavigation();
  const [telegramModalOpen, setTelegramModalOpen] = useState(false);

  const telegramStatus = useAppSelector((state) =>
    selectTelegramConnectionStatus(state, userId)
  );

  // 复用 onboarding 中的 connectOptions
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

#### 设置 Hooks

**useSettingsNavigation**

设置模态的基于 URL 导航。

```typescript
interface UseSettingsNavigationReturn {
  currentRoute: string;
  navigateTo: (panel: string) => void;
  navigateBack: () => void;
  closeModal: () => void;
}

const { navigateTo, navigateBack, closeModal } = useSettingsNavigation();

// 导航到 panel
navigateTo('connections'); // → /settings/connections

// 返回
navigateBack(); // → /settings

// 关闭模态
closeModal(); // → 之前的非设置路由
```

**useSettingsAnimation**

设置模态的动画状态管理。

```typescript
interface UseSettingsAnimationReturn {
  isEntering: boolean;
  isExiting: boolean;
  animationClass: string;
}

const { animationClass } = useSettingsAnimation();

<div className={`modal ${animationClass}`}>{/* Content */}</div>
```

#### 设置组件

**SettingsHeader**

设置顶部的用户 profile 部分。

```typescript
interface SettingsHeaderProps {
  user: User | null;
  onClose: () => void;
}

<SettingsHeader user={user} onClose={handleClose} />
```

**SettingsMenuItem**

带图标和 chevron 的单个菜单项。

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

**SettingsBackButton**

返回导航按钮。

```typescript
interface SettingsBackButtonProps {
  onClick: () => void;
}

<SettingsBackButton onClick={navigateBack} />
```

**SettingsPanelLayout**

设置 panel 的包装器。

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

### 组件模式

#### 复用连接选项

`connectOptions` 数组在 onboarding 和 settings 之间共享：

```typescript
// 在 ConnectStep.tsx 中定义，在其他地方导入
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

#### 通过 Portal 的模态

设置模态使用 `createPortal` 在组件树外部渲染：

```typescript
return createPortal(
  <div className="modal-container">
    {/* 模态内容 */}
  </div>,
  document.body
);
```

#### 受控 vs 非受控

连接模态是受控组件：

```typescript
// 父级控制 open 状态
const [isOpen, setIsOpen] = useState(false);

<TelegramConnectionModal
  isOpen={isOpen}
  onClose={() => setIsOpen(false)}
/>
```

***

## Hook 与工具

自定义 React hook 和工具函数。

### 自定义 Hooks

#### useSocket (`hooks/useSocket.ts`)

从任何组件访问 Socket.io 功能。

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

**用法：**

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

**配合事件监听器：**

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

#### useUser (`hooks/useUser.ts`)

访问用户 profile 数据和加载状态。

```typescript
interface UseUserReturn {
  user: User | null;
  loading: boolean;
  error: string | null;
  refetch: () => Promise<void>;
}

function useUser(): UseUserReturn;
```

**用法：**

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

#### 设置模态 Hooks

**useSettingsNavigation (`components/settings/hooks/useSettingsNavigation.ts`)**

设置模态的基于 URL 导航。

```typescript
interface UseSettingsNavigationReturn {
  currentRoute: string; // 当前设置路径
  navigateTo: (panel: string) => void; // 导航到 panel
  navigateBack: () => void; // 返回一级
  closeModal: () => void; // 完全关闭设置
}

function useSettingsNavigation(): UseSettingsNavigationReturn;
```

**用法：**

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

**useSettingsAnimation (`components/settings/hooks/useSettingsAnimation.ts`)**

设置模态的动画状态管理。

```typescript
interface UseSettingsAnimationReturn {
  isEntering: boolean; // 模态正在动画进入
  isExiting: boolean; // 模态正在动画退出
  animationClass: string; // 当前状态的 CSS 类
}

function useSettingsAnimation(): UseSettingsAnimationReturn;
```

**用法：**

```typescript
import { useSettingsAnimation } from "./hooks/useSettingsAnimation";

function SettingsModal() {
  const { animationClass, isExiting } = useSettingsAnimation();

  return <div className={`modal ${animationClass}`}>{/* Content */}</div>;
}
```

### 工具

#### 配置 (`utils/config.ts`)

构建时环境变量访问。这些常量只携带烘焙到 bundle 中的值，对于应用实际通信的**运行时** URL，见 `services/backendUrl` 和下方的 `hooks/useBackendUrl`。

```typescript
// 仅构建时 fallback（在 Tauri 外使用）。
export const BACKEND_URL = import.meta.env.VITE_BACKEND_URL || 'https://api.example.com';

// 调试模式
export const DEBUG = import.meta.env.VITE_DEBUG === 'true';
```

**用法（仅构建时、feature flag、调试开关、…）：**

```typescript
import { DEBUG } from '../utils/config';

if (DEBUG) {
  console.log('debug enabled');
}
```

> **不要**直接导入 `BACKEND_URL` 来发起 API 调用。在运行时解析 URL，以便核心 sidecar 的 `api_url`（通过登录屏幕上的 `openhuman.config_resolve_api_url` 设置）生效：
>
> ```typescript
> // React 组件
> import { useBackendUrl } from '../hooks/useBackendUrl';
> const backendUrl = useBackendUrl();
>
> // 非 React 代码
> import { getBackendUrl } from '../services/backendUrl';
> const backendUrl = await getBackendUrl();
> ```

#### 深度链接 (`utils/deeplink.ts`)

为认证交接构建深度链接 URL。

```typescript
// 构建 auth 深度链接
function buildAuthDeepLink(token: string): string;

// 解析深度链接 URL
function parseDeepLink(url: string): { path: string; params: URLSearchParams };
```

**用法：**

```typescript
import { buildAuthDeepLink } from '../utils/deeplink';

// 为浏览器重定向构建 URL
const deepLink = buildAuthDeepLink(loginToken);
// → "openhuman://auth?token=abc123"

// 在 Web 前端 auth 后：
window.location.href = deepLink;
```

#### 桌面深度链接监听器 (`utils/desktopDeepLinkListener.ts`)

在桌面应用中处理传入的深度链接。

```typescript
// 设置深度链接事件监听器
async function setupDesktopDeepLinkListener(): Promise<void>;
```

**在 main.tsx 中调用：**

```typescript
// 懒加载以确保 Tauri IPC 就绪
import('./utils/desktopDeepLinkListener').then(m => {
  m.setupDesktopDeepLinkListener().catch(console.error);
});
```

**它做什么：**

1. 监听来自 Tauri 深度链接插件的 `onOpenUrl` 事件
2. 解析 `openhuman://auth?token=...` URL
3. 调用 Rust `exchange_token` 命令（绕过 CORS）
4. 在 Redux 中存储会话
5. 导航到 `/onboarding` 或 `/home`

**循环预防：**

```typescript
// 导航前设置 flag 以防止重新处理
localStorage.setItem('deepLinkHandled', 'true');
window.location.replace('/');

// 下次加载时，清除 flag
if (localStorage.getItem('deepLinkHandled') === 'true') {
  localStorage.removeItem('deepLinkHandled');
  return; // 不再处理
}
```

#### URL 打开器 (`utils/openUrl.ts`)

跨平台 URL 打开。

```typescript
// 在系统浏览器中打开 URL
async function openUrl(url: string): Promise<void>;
```

**用法：**

```typescript
import { openUrl } from '../utils/openUrl';

// 在系统浏览器中打开（非应用内 WebView）
await openUrl('https://telegram.org/auth');
```

**实现：**

```typescript
export async function openUrl(url: string): Promise<void> {
  try {
    // 先尝试 Tauri opener 插件
    const { open } = await import('@tauri-apps/plugin-opener');
    await open(url);
  } catch {
    // Fallback 到浏览器 API
    window.open(url, '_blank');
  }
}
```

### Polyfills (`polyfills.ts`)

浏览器环境的 Node.js polyfills。

`telegram` npm 包需要 Node.js API。这些被 polyfill：

```typescript
// polyfills.ts
import { Buffer } from 'buffer';
import process from 'process';
import util from 'util';

window.Buffer = Buffer;
window.process = process;
window.util = util;
```

**在应用入口导入：**

```typescript
// main.tsx
import './polyfills';

// ... 应用的其余部分
```

**Vite 配置：**

```typescript
// vite.config.ts
export default defineConfig({
  resolve: { alias: { buffer: 'buffer', process: 'process/browser', util: 'util' } },
  define: { 'process.env': {}, global: 'globalThis' },
});
```

### 类型

#### API 类型 (`types/api.ts`)

```typescript
// API 响应包装器
interface ApiResponse<T> {
  success: boolean;
  data?: T;
  error?: string;
}

// API 错误
interface ApiError {
  code: string;
  message: string;
  details?: unknown;
}

// User 接口
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

#### Onboarding 类型 (`types/onboarding.ts`)

```typescript
// Onboarding step 定义
interface OnboardingStep {
  id: string;
  title: string;
  component: React.ComponentType<StepProps>;
}

// Step 组件 props
interface StepProps {
  onNext: () => void;
  onBack: () => void;
}

// 连接选项
interface ConnectionOption {
  id: string;
  label: string;
  icon: React.ComponentType;
  description: string;
  comingSoon?: boolean;
}
```

### 静态数据

#### 国家 (`data/countries.ts`)

手机号输入的国家列表。

```typescript
interface Country {
  code: string; // "US"
  name: string; // "United States"
  dialCode: string; // "+1"
  flag: string; // "🇺🇸"
}

export const countries: Country[];
```

**用法：**

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

### 最佳实践

#### Hook 依赖

始终在 useEffect 中包含依赖：

```typescript
// 好
useEffect(() => {
  on('event', handler);
  return () => off('event', handler);
}, [on, off, handler]);

// 坏 - 缺失依赖
useEffect(() => {
  on('event', handler);
  return () => off('event', handler);
}, []);
```

#### 清理函数

始终清理订阅：

```typescript
useEffect(() => {
  const subscription = subscribe();
  return () => subscription.unsubscribe();
}, []);
```

#### 错误边界

将工具调用包装在 try-catch 中：

```typescript
try {
  await openUrl(url);
} catch (error) {
  console.error('Failed to open URL:', error);
  // Fallback 行为
}
```

#### 类型安全

对 API 调用使用 TypeScript 泛型：

```typescript
const user = await apiClient.get<User>('/users/me');
// user 被类型化为 User
```

***

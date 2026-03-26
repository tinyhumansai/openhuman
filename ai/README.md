# OpenHuman AI Configuration

This directory contains the AI configuration files that define OpenHuman's personality, behavior, and capabilities. These files follow the OpenClaw framework pattern for AI agent configuration.

## 📁 Configuration Files

### **SOUL.md** ✅ Active

Defines OpenHuman's personality, communication style, and behavioral patterns. This is the core file that shapes how the AI interacts with users.

- **Status**: Fully implemented with human, vibrant personality
- **Features**: Curious, witty, empathetic, authentic, and optimistic traits
- **Usage**: Automatically injected into every user message for consistent behavior

### **TOOLS.md** 🚧 TODO

Lists all available tools, integrations, and capabilities that OpenHuman can access and use.

- **Should include**: Telegram, Discord, MCP tools, Skills system, Platform APIs
- **Purpose**: Defines what actions OpenHuman can perform
- **Usage**: Tool discovery and capability awareness

### **AGENTS.md** 🚧 TODO

Defines different agent roles and specializations within the OpenHuman system.

- **Should include**: Primary agent role, specialized sub-agents, collaboration patterns
- **Purpose**: Agent coordination and role-based interactions
- **Usage**: Context switching and task delegation

### **IDENTITY.md** 🚧 TODO

Establishes the fundamental identity and core values that remain consistent across all interactions.

- **Should include**: Mission, core values, relationship principles, ethical boundaries
- **Purpose**: Foundational identity that never changes
- **Usage**: Core personality and value system

### **USER.md** 🚧 TODO

Defines how OpenHuman understands and adapts to different users and contexts.

- **Should include**: User profiling, personalization strategies, privacy considerations
- **Purpose**: Contextual adaptation and user-specific customization
- **Usage**: Personalizing interactions while maintaining consistency

### **BOOTSTRAP.md** 🚧 TODO

Initialization and setup procedures for new conversations and user onboarding.

- **Should include**: First interaction protocols, onboarding flows, context establishment
- **Purpose**: Consistent startup and initialization behavior
- **Usage**: New user experience and conversation setup

### **MEMORY.md** 🚧 TODO

Curated long-term knowledge and memories that persist across sessions.

- **Should include**: Platform knowledge, successful patterns, user insights, technical knowledge
- **Purpose**: Continuous learning and knowledge retention
- **Usage**: Cross-session memory and accumulated wisdom

## 🔧 Technical Details

### How It Works

1. **SOUL Injection System**: Automatically adds SOUL.md content to every user message
2. **Multi-layer Caching**: Memory → localStorage → GitHub → bundled fallback
3. **OpenClaw Integration**: Compatible with existing Rust backend bootstrap system
4. **Real-time Updates**: Changes to files are reflected immediately with cache refresh

### File Location Strategy

- **Local Development**: Files loaded from this `/ai/` directory
- **Production/GitHub**: Files can be loaded from remote GitHub repository
- **Fallback**: Bundled versions ensure system never breaks
- **Organized Structure**: All AI config in one logical location

### Implementation Status

- ✅ **SOUL.md**: Fully implemented with injection system
- ✅ **File Structure**: Organized `/ai/` directory created
- ✅ **Caching System**: Multi-layer caching with refresh functionality
- ✅ **Settings UI**: AI Configuration panel for viewing and refreshing
- 🚧 **Remaining Files**: TODO placeholders created for future implementation

## 🚀 Usage

### Viewing Current Configuration

1. Go to **Settings → AI Configuration**
2. View live SOUL personality preview
3. Check source (GitHub vs bundled) and last loaded time
4. Use "Refresh SOUL Configuration" to load latest changes

### Editing AI Behavior

1. **Edit SOUL.md** to change personality and communication style
2. **Refresh** in Settings panel to load changes immediately
3. **Test** in conversations to see new behavior
4. **Iterate** until personality feels right

### Future Development

1. **Fill in TODO files** with specific configuration content
2. **Extend loader system** to support all bootstrap files
3. **Add UI controls** for editing and managing each file
4. **Implement cross-file** coordination and consistency checks

## 📚 Documentation

- **OpenClaw Framework**: See Rust backend `src-tauri/src/openhuman/channels/prompt.rs`
- **SOUL Injection**: See `src/lib/ai/soul/` for implementation details
- **Settings UI**: See `src/components/settings/panels/AIPanel.tsx`
- **Message Flow**: See conversation injection in `src/pages/Conversations.tsx`

---

**Note**: This is a living configuration system. As OpenHuman evolves, these files will be expanded and refined to create an increasingly sophisticated and helpful AI assistant.

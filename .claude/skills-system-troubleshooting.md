# Skills System Troubleshooting Guide

## Overview

The Skills System is a Python-based plugin architecture that allows AI agents to have domain-specific knowledge, tools, and automated behaviors. Skills run as isolated Python subprocesses and communicate with the main Tauri application via JSON-RPC.

## Common Issue: "Setup Failed" with Exit Code 1

### Symptoms

- Skills modal shows "Setup Failed" with "Skill process exited with code: 1"
- Console shows `ModuleNotFoundError: No module named 'pydantic'`
- Error paths like `/Users/cyrus/openhuman/skills/skills/telegram/`
- Python import failures and subprocess stderr messages

### Root Cause Analysis

**Primary Issue: Missing Skills Git Submodule**
The main cause is that the `skills` Git submodule is not initialized. The system expects skills to be available in the `skills/skills/` directory structure but finds an empty directory.

**Secondary Issues:**

1. **Missing Python Virtual Environment**: No `.venv` directory in the skills folder
2. **Missing Python Dependencies**: Core packages like `pydantic`, `telethon`, `mcp` not installed
3. **Incorrect Python Paths**: PYTHONPATH configuration issues

### Skills System Architecture

```
skills/                          # Git submodule root
├── .venv/                       # Python virtual environment
├── requirements.txt             # Shared dependencies
├── skills/                      # Individual skill packages
│   ├── telegram/               # Telegram skill
│   │   ├── skill.py           # Main skill logic
│   │   ├── manifest.json      # Skill metadata
│   │   ├── requirements.txt   # Skill-specific dependencies
│   │   └── ...
│   ├── browser/               # Browser automation skill
│   ├── calendar/              # Calendar integration skill
│   └── ...                    # Other skills
└── ...
```

### Solution Steps

#### 1. Initialize Git Submodule

```bash
git submodule init
git submodule update
```

This downloads the skills repository from `https://github.com/alphahumanxyz/skills`.

#### 2. Create Python Virtual Environment

```bash
cd skills
python3 -m venv .venv
.venv/bin/pip install --upgrade pip
```

#### 3. Install Dependencies

```bash
.venv/bin/pip install -r requirements.txt
```

This installs:

- **Core Dependencies**: `mcp>=1.0.0`, `pydantic>=2.0`, `aiosqlite>=0.20.0`
- **Skill-Specific Dependencies**: Each skill's requirements.txt (telegram, browser, etc.)

#### 4. Verify Installation

```bash
# Test core imports
.venv/bin/python -c "import pydantic, mcp; print('✅ Core dependencies OK')"

# Test skill import
.venv/bin/python -c "import skills.telegram; print('✅ Telegram skill OK')"
```

### How Skills System Works

#### Development vs Production Paths

- **Development**: Skills in git submodule at `./skills/skills/`
- **Production**: Skills in `~/.openhuman/skills/`
- **Configuration**: `src/lib/skills/paths.ts` handles path resolution

#### Skill Execution Process

1. **Discovery**: `SkillProvider` scans for skill manifests
2. **Registration**: Skills registered in Redux store
3. **Startup**: Python subprocess spawned with proper environment
4. **Communication**: JSON-RPC transport over stdin/stdout
5. **Setup**: Interactive setup flow if required

#### Environment Variables

The system automatically configures:

- **PYTHONPATH**: Includes skills directory and virtual environment
- **Telegram API**: `TELEGRAM_API_ID`, `TELEGRAM_API_HASH`
- **Working Directory**: Skills submodule root

### Verification Commands

```bash
# Check submodule status
git submodule status

# Verify skills directory structure
ls -la skills/skills/telegram/

# Check virtual environment
ls -la skills/.venv/

# Test Python environment
cd skills && .venv/bin/python -c "import sys; print('\\n'.join(sys.path))"
```

### Prevention

To prevent this issue in fresh checkouts:

1. **Always initialize submodules**:

   ```bash
   git clone --recurse-submodules <repo-url>
   # or after clone:
   git submodule update --init --recursive
   ```

2. **Setup script**: Consider adding to `package.json`:
   ```json
   {
     "scripts": {
       "setup": "git submodule update --init && cd skills && python3 -m venv .venv && .venv/bin/pip install -r requirements.txt"
     }
   }
   ```

### Related Files

- **Frontend**: `src/lib/skills/` - Skills management system
- **Backend**: `src-tauri/src/commands/skills.rs` - Rust skill commands
- **Configuration**: `src/utils/config.ts` - Environment variables
- **Providers**: `src/providers/SkillProvider.tsx` - Skills lifecycle

### Expected Behavior After Fix

1. Skills modal should show "Connect Telegram" instead of error
2. No more Python import errors in console
3. Skill setup process should work correctly
4. Background GitHub sync should function properly

This fix resolves the fundamental infrastructure issue preventing skills from loading and running properly.

# Automated Instruction Prompt for AI Agent (Antigravity agy)
## OpenHuman Thai Local Setup & Installation Execution Plan

คำสั่ง Prompt ด้านล่างนี้ ออกแบบมาเพื่อให้คุณส่งให้กับ AI Agent (เช่น Antigravity `agy` agent) เพื่อให้อ่านและสั่งงานดำเนินการติดตั้ง ตั้งค่า และทดสอบระบบ OpenHuman ภาษาไทยในเครื่องใหม่แบบอัตโนมัติ 100%

---

```markdown
# MISSION: Autonomous Setup and Configuration for OpenHuman Thai Local Edition

You are tasked with executing the end-to-end installation, Thai localization, Ollama integration, Obsidian vault setup, and system configuration for OpenHuman on this machine.

Follow these strict operational steps sequentially without stopping until full verification is achieved.

## Phase 1: Environment & Prerequisites Verification
1. Check if Node.js (>= v20), pnpm, Git, Ollama, and Obsidian are installed.
2. If Ollama is running, verify that the Thai LLM model `qwen3.5:4b` and embedding model `nomic-embed-text` are pulled:
   - Run `ollama pull qwen3.5:4b`
   - Run `ollama pull nomic-embed-text`
3. Verify that the OpenHuman desktop binary exists at `%LOCALAPPDATA%\OpenHuman\OpenHuman.exe`.

## Phase 2: Obsidian Vault Setup
1. Create the Obsidian Vault directory at `D:\openhuman-local\OpenHuman-Obsidian` if it does not exist.
2. Initialize default directories inside the vault: `00-Inbox`, `01-Projects`, `02-Areas`, `03-Resources`, `04-Archives`, `Daily-Notes`, `Agents-Memory`.

## Phase 3: Configuration & Environment Files (.ENV)
1. Ensure Root `.env` file exists at `D:\openhuman-local\.env` with:
   ```env
   OPENHUMAN_CORE_TOKEN=openhuman-local-token-12345
   OPENHUMAN_CORE_PORT=7788
   OPENHUMAN_CORE_REUSE_EXISTING=1
   OPENHUMAN_ACTION_DIR=D:\openhuman-local\OpenHuman-Obsidian
   OLLAMA_BASE_URL=http://localhost:11434
   OLLAMA_DEFAULT_MODEL=qwen3.5:4b
   OPENHUMAN_AUTONOMY_LEVEL=supervised
   ```
2. Ensure Frontend `app/.env.local` exists with:
   ```env
   VITE_CORE_RPC_URL=http://127.0.0.1:7788/rpc
   VITE_DEFAULT_LOCALE=th
   VITE_TELEMETRY_DISABLED=true
   ```

## Phase 4: Agent Thai System Prompts & Preferences
1. Resolve the active user workspace directory under `%USERPROFILE%\.openhuman\users\*\workspace`.
2. Write Thai profile and instruction files:
   - `USER.md`: Set preferred language to Thai (ภาษาไทย).
   - `AGENTS.md`: Instruct all agents to respond and converse in natural, fluent Thai.
   - `STYLE.md`: Set writing style and explicit Thai language rule.

## Phase 5: Dependencies & Production Build
1. Run `pnpm install` in `D:\openhuman-local`.
2. Ensure the bootCheck version compatibility patch in `app/src/lib/bootCheck/index.ts` is active (allowing 0.63.x version matches).
3. Ensure settings panels do not block on non-Tauri browser environments.
4. Run `pnpm build` and verify zero compile/typecheck errors.

## Phase 6: One-Click Launcher & Verification
1. Verify/create `Start-OpenHuman-Thai.bat` and the Desktop shortcut `OpenHuman (Thai).lnk`.
2. Test starting the OpenHuman Core server on port 7788:
   ```powershell
   $env:OPENHUMAN_CORE_TOKEN = "openhuman-local-token-12345"
   Start-Process -FilePath "$env:LOCALAPPDATA\OpenHuman\OpenHuman.exe" -ArgumentList "core run --port 7788" -WindowStyle Hidden
   ```
3. Verify JSON-RPC endpoint response:
   - Call `openhuman.app_state_snapshot` and `openhuman.config_get_autonomy_settings` via HTTP POST to `http://127.0.0.1:7788/rpc` with `Authorization: Bearer openhuman-local-token-12345`.
4. Report back with full operational status.
```

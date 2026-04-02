/**
 * Seeds the minimal QuickJS echo skill used by Rust `json_rpc_skills_runtime_start_tools_call_stop`
 * so the desktop core can run `openhuman.skills_start` → `skills_call_tool` against a real skill tree.
 */
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

export const E2E_RUNTIME_SKILL_ID = 'e2e-runtime';

const MANIFEST = {
  id: E2E_RUNTIME_SKILL_ID,
  name: 'E2E Runtime Skill',
  version: '1.0.0',
  description: 'Minimal skill for desktop E2E (echo tool)',
  runtime: 'quickjs',
  entry: 'index.js',
  auto_start: false,
};

const INDEX_JS = `globalThis.__skill = {
            name: "E2E Runtime Skill",
            tools: [
                {
                    name: "echo",
                    description: "Echoes back the input message",
                    inputSchema: {
                        type: "object",
                        properties: {
                            message: { type: "string", description: "Message to echo" }
                        },
                        required: ["message"]
                    },
                    execute(args) {
                        return { type: "text", text: "echo: " + (args.message || "empty") };
                    }
                }
            ]
        };

        function init() {
            if (globalThis.__ops && globalThis.__ops.log) {
                globalThis.__ops.log("info", "e2e-runtime-skill initialized");
            }
        }

        init();
`;

/** Resolve directories that should contain `e2e-runtime/manifest.json` (core may use either). */
export function resolveE2eRuntimeSkillDirs(): string[] {
  const dirs: string[] = [];
  const homeDir = path.join(
    os.homedir(),
    '.openhuman',
    'workspace',
    'skills',
    E2E_RUNTIME_SKILL_ID
  );
  dirs.push(homeDir);

  const w = process.env.OPENHUMAN_WORKSPACE?.trim();
  if (w) {
    const resolved = path.resolve(w);
    const nested =
      path.basename(resolved) === 'workspace'
        ? path.join(resolved, 'skills', E2E_RUNTIME_SKILL_ID)
        : path.join(resolved, 'workspace', 'skills', E2E_RUNTIME_SKILL_ID);
    if (nested !== homeDir) {
      dirs.push(nested);
    }
  }

  return dirs;
}

export async function seedMinimalEchoSkill(): Promise<void> {
  const manifestBody = JSON.stringify(MANIFEST, null, 2);
  for (const skillRoot of resolveE2eRuntimeSkillDirs()) {
    await fs.mkdir(skillRoot, { recursive: true });
    await fs.writeFile(path.join(skillRoot, 'manifest.json'), manifestBody, 'utf-8');
    await fs.writeFile(path.join(skillRoot, 'index.js'), INDEX_JS, 'utf-8');
  }
}

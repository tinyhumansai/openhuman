import { existsSync } from "node:fs";
import path from "node:path";

function firstExistingDirectory(candidates) {
  for (const candidate of candidates) {
    if (candidate && existsSync(candidate)) return path.resolve(candidate);
  }
  return "";
}

function candidateRoots(toolDir) {
  const roots = [];
  const resolvedToolDir = path.resolve(toolDir || ".");
  roots.push(process.env.AMAZON_QA_ROOT || "");
  roots.push(process.cwd());
  roots.push(resolvedToolDir);
  let current = resolvedToolDir;
  for (let index = 0; index < 6; index += 1) {
    roots.push(current);
    roots.push(path.dirname(current));
    current = path.dirname(current);
  }
  return [...new Set(roots.filter(Boolean).map((item) => path.resolve(item)))];
}

function findWorkspaceRoot(toolDir) {
  const configured = process.env.OPENHUMAN_KB_DIR || process.env.AMAZON_QA_WORKSPACE;
  if (configured) return path.resolve(configured);
  for (const root of candidateRoots(toolDir)) {
    const direct = path.join(root, "openhuman-kb");
    if (existsSync(direct)) return direct;
  }
  return path.resolve(toolDir || ".", "..", "openhuman-kb");
}

function findOpenHumanRepo(toolDir, workspaceRoot) {
  const configured = process.env.OPENHUMAN_REPO_DIR;
  if (configured) return path.resolve(configured);
  const roots = candidateRoots(toolDir);
  if (workspaceRoot) roots.unshift(path.dirname(workspaceRoot));
  for (const root of [...new Set(roots)]) {
    const direct = path.join(root, "openhuman");
    if (existsSync(path.join(direct, "package.json")) && existsSync(path.join(direct, "src"))) return direct;
    if (existsSync(path.join(root, "package.json")) && existsSync(path.join(root, "src", "openhuman"))) return root;
  }
  return path.resolve(toolDir || ".", "..", "openhuman");
}

function findDeliveryRoot(toolDir, workspaceRoot, repoRoot) {
  const configured = process.env.AMAZON_QA_ROOT;
  if (configured) return path.resolve(configured);
  if (workspaceRoot) return path.dirname(workspaceRoot);
  if (repoRoot && path.basename(repoRoot) === "openhuman") return path.dirname(repoRoot);
  return path.resolve(toolDir || ".", "..");
}

export function isSubpath(parent, child) {
  const relative = path.relative(path.resolve(parent), path.resolve(child));
  return Boolean(relative) && !relative.startsWith("..") && !path.isAbsolute(relative);
}

export function resolveAmazonQaPaths(toolDir = import.meta.dirname) {
  const resolvedToolDir = path.resolve(toolDir);
  const workspaceRoot = findWorkspaceRoot(resolvedToolDir);
  const repoRoot = findOpenHumanRepo(resolvedToolDir, workspaceRoot);
  const deliveryRoot = findDeliveryRoot(resolvedToolDir, workspaceRoot, repoRoot);
  const outputRoot = path.resolve(process.env.AMAZON_QA_OUTPUT_DIR || path.join(deliveryRoot, "output"));
  const runRoot = path.join(workspaceRoot, "run");

  return {
    toolDir: resolvedToolDir,
    root: deliveryRoot,
    workspace: workspaceRoot,
    repoRoot,
    outputRoot,
    runRoot,
    configPath: path.join(workspaceRoot, "config.toml"),
    manifestPath: path.join(workspaceRoot, "processed", "manifest.jsonl"),
    memoryDbPath: path.join(workspaceRoot, "workspace", "memory", "memory.db"),
    memoryTreeDbPath: path.join(workspaceRoot, "workspace", "memory_tree", "chunks.db"),
    docsDir: path.join(workspaceRoot, "workspace", "memory", "namespaces", "amazon-learning", "docs"),
    coreBin: path.join(repoRoot, "target", "debug", "openhuman-core"),
    drainBin: path.join(repoRoot, "target", "debug", "amazon-memory-tree-drain"),
    uiPath: path.join(resolvedToolDir, "amazon-qa-ui.html"),
    serverPath: path.join(resolvedToolDir, "amazon-qa-server.mjs"),
    sourceTreeDrainRunnerPath: path.join(resolvedToolDir, "amazon-source-tree-drain-runner.mjs"),
    handoffPath: path.join(outputRoot, "amazon-learning-product-handoff.md"),
    productSourceInRepo: repoRoot ? isSubpath(repoRoot, resolvedToolDir) || path.resolve(repoRoot) === resolvedToolDir : false,
  };
}

import { readdirSync } from "node:fs";

// Root targets are explicit; an empty directory may be absent in a checkout.
export function rootRustTargetNames(directory) {
  let entries;
  try {
    entries = readdirSync(directory);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
    entries = [];
  }
  return new Set(
    entries
      .filter((name) => name.endsWith(".rs"))
      .map((name) => name.slice(0, -3)),
  );
}

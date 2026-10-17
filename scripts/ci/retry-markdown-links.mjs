import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const TRANSIENT_HTTP_STATUSES = new Set(["500", "502", "503", "504"]);

/** Extract URLs only when every reported link failure is a transient HTTP 5xx. */
export function serviceUnavailableUrls(report) {
  const failures = [...report.matchAll(/^\*\s+\[([^\]]+)\]\s+<([^>]+)>/gm)];
  if (
    !failures.length ||
    failures.some(([, status]) => !TRANSIENT_HTTP_STATUSES.has(status))
  )
    return null;
  return [...new Set(failures.map(([, , url]) => url))];
}

/** Recheck a failed sweep without accepting unsuccessful HTTP responses. */
export async function retryServiceUnavailable({
  initialReport,
  check,
  wait = delay,
  log = console.info,
}) {
  let urls = serviceUnavailableUrls(initialReport);
  if (!urls) {
    log("[link-retry] reason=ineligible-report status=1");
    return 1;
  }
  let status = 1;
  for (let attempt = 0; attempt < 3; attempt++) {
    const backoff = 5000 * 2 ** attempt;
    log(
      `[link-retry] attempt=${attempt + 1} backoff_ms=${backoff} urls=${urls.length}`,
    );
    await wait(backoff);
    const result = await check(urls);
    status = result.status;
    log(`[link-retry] attempt=${attempt + 1} status=${status}`);
    if (status === 0) {
      log("[link-retry] reason=recovered status=0");
      return 0;
    }
    urls = serviceUnavailableUrls(result.report);
    if (!urls) {
      log(`[link-retry] reason=non-transient-failure status=${status}`);
      return status;
    }
  }
  log(`[link-retry] reason=retries-exhausted status=${status}`);
  return status;
}

/** Recheck a Lychee Markdown report with the installed executable. */
export async function recheckReport(reportPath, binary, { wait = delay } = {}) {
  const initialReport = readFileSync(reportPath, "utf8");
  let attempt = 0;
  const status = await retryServiceUnavailable({
    initialReport,
    wait,
    check(urls) {
      const input = join(dirname(reportPath), `retry-${++attempt}.md`);
      writeFileSync(input, urls.map((url) => `- <${url}>`).join("\n") + "\n");
      // These URLs passed the full sweep's inclusion/exclusion rules already.
      // Keep fragment checking on the retry, and require actual successful HTTP.
      const run = spawnSync(
        binary,
        [
          "--no-progress",
          "--max-concurrency",
          "4",
          "--include-fragments",
          "--format",
          "markdown",
          "--output",
          reportPath,
          input,
        ],
        { stdio: "inherit", timeout: 90000 },
      );
      return {
        status: run.status ?? 1,
        report: readFileSync(reportPath, "utf8"),
      };
    },
  });
  return status;
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  process.exitCode = await recheckReport(process.argv[2], process.argv[3]);
}

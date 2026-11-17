import { json } from "../http.mjs";
import { createMockId, getMockCronJobs } from "../state.mjs";

export function handleCronJobs(ctx) {
  const { method, url, parsedBody, res } = ctx;
  const jobs = getMockCronJobs();

  if (method === "GET" && /^\/settings\/cron-jobs\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: jobs });
    return true;
  }
  if (method === "POST" && /^\/settings\/cron-jobs\/?$/.test(url)) {
    const created = {
      ...(parsedBody || {}),
      id: createMockId("cron"),
      name:
        typeof parsedBody?.name === "string" && parsedBody.name.trim()
          ? parsedBody.name.trim()
          : "Mock cron job",
      enabled: parsedBody?.enabled !== false,
    };
    jobs.unshift(created);
    json(res, 200, { success: true, data: created });
    return true;
  }

  const item = url.match(/^\/settings\/cron-jobs\/([^/?]+)\/?(\?.*)?$/);
  if (method === "PATCH" && item) {
    const job = jobs.find((entry) => entry.id === item[1]);
    if (!job) {
      json(res, 404, { success: false, error: "Cron job not found" });
      return true;
    }
    Object.assign(job, parsedBody || {}, { id: job.id });
    json(res, 200, { success: true, data: job });
    return true;
  }
  return false;
}

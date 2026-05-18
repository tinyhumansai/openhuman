import { json } from "../http.mjs";
import {
  createMockId,
  fuzzyNumber,
  fuzzyPick,
  fuzzyTimestamp,
  getMockCronJobs,
  getMockWebhookTriggers,
} from "../state.mjs";

let fixturesSeeded = false;

export function resetCronFixturesState() {
  fixturesSeeded = false;
}

function ensureCronFixtures() {
  if (fixturesSeeded) return;
  const cronJobs = getMockCronJobs();
  const webhookTriggers = getMockWebhookTriggers();
  if (cronJobs.length === 0) {
    const count = fuzzyNumber("cron:count", 0, 2);
    for (let i = 0; i < count; i += 1) {
      cronJobs.push({
        id: createMockId("cron"),
        name: fuzzyPick(
          `cron:name:${i}`,
          ["Morning digest", "Support sweep", "Usage snapshot"],
          "Mock cron job",
        ),
        schedule: fuzzyPick(
          `cron:schedule:${i}`,
          ["0 8 * * *", "*/30 * * * *", "15 17 * * 1-5"],
          "0 8 * * *",
        ),
        enabled: i % 2 === 0,
        createdAt: fuzzyTimestamp(`cron:created:${i}`),
      });
    }
  }
  if (webhookTriggers.length === 0) {
    const count = fuzzyNumber("trg:count", 0, 2);
    for (let i = 0; i < count; i += 1) {
      webhookTriggers.push({
        id: createMockId("trg"),
        name: fuzzyPick(
          `trg:name:${i}`,
          ["Github push", "Stripe paid", "CRM sync"],
          "Mock trigger",
        ),
        enabled: true,
        createdAt: fuzzyTimestamp(`trg:created:${i}`),
      });
    }
  }
  fixturesSeeded = true;
}

export function handleCron(ctx) {
  const { method, url, parsedBody, res } = ctx;
  ensureCronFixtures();
  const cronJobs = getMockCronJobs();
  const webhookTriggers = getMockWebhookTriggers();

  if (method === "GET" && /^\/settings\/cron-jobs\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: cronJobs });
    return true;
  }
  if (method === "POST" && /^\/settings\/cron-jobs\/?$/.test(url)) {
    const created = {
      ...(parsedBody || {}),
      id: createMockId("cron"),
      createdAt: new Date().toISOString(),
    };
    cronJobs.unshift(created);
    json(res, 200, {
      success: true,
      data: created,
    });
    return true;
  }
  const cronItem = url.match(/^\/settings\/cron-jobs\/([^/?]+)\/?(\?.*)?$/);
  if (cronItem && (method === "PATCH" || method === "DELETE")) {
    const index = cronJobs.findIndex((entry) => entry.id === cronItem[1]);
    if (index >= 0 && method === "PATCH") {
      const { id: _ignoredId, ...patch } = parsedBody || {};
      cronJobs[index] = {
        ...cronJobs[index],
        ...patch,
      };
    }
    if (index >= 0 && method === "DELETE") {
      cronJobs.splice(index, 1);
    }
    json(res, 200, {
      success: true,
      data: { id: cronItem[1], deleted: method === "DELETE" },
    });
    return true;
  }

  if (
    method === "GET" &&
    /^\/settings\/webhooks-triggers\/?(\?.*)?$/.test(url)
  ) {
    json(res, 200, { success: true, data: webhookTriggers });
    return true;
  }
  if (method === "POST" && /^\/settings\/webhooks-triggers\/?$/.test(url)) {
    const created = {
      ...(parsedBody || {}),
      id: createMockId("trg"),
      createdAt: new Date().toISOString(),
    };
    webhookTriggers.unshift(created);
    json(res, 200, {
      success: true,
      data: created,
    });
    return true;
  }
  const trgItem = url.match(
    /^\/settings\/webhooks-triggers\/([^/?]+)\/?(\?.*)?$/,
  );
  if (trgItem && (method === "PATCH" || method === "DELETE")) {
    const index = webhookTriggers.findIndex((entry) => entry.id === trgItem[1]);
    if (index >= 0 && method === "PATCH") {
      const { id: _ignoredId, ...patch } = parsedBody || {};
      webhookTriggers[index] = {
        ...webhookTriggers[index],
        ...patch,
      };
    }
    if (index >= 0 && method === "DELETE") {
      webhookTriggers.splice(index, 1);
    }
    json(res, 200, {
      success: true,
      data: { id: trgItem[1], deleted: method === "DELETE" },
    });
    return true;
  }

  return false;
}

import assert from "node:assert/strict";
import test from "node:test";

import { buildProductDoctorReport } from "./amazon-qa-product.mjs";

const READY_STATUS = {
  health: {
    documents: 1779,
    chunks: 14597,
    embeddedChunks: 14597,
    vectorCoveragePercent: 100,
  },
  readiness: {
    level: "answer_ready_learning_processing",
    answerStatus: "ready",
    searchStatus: "ready",
    citationStatus: "ready",
    learningStatus: "processing",
  },
  sourceTreeDrain: {
    queuedJobs: 10,
    doneJobs: 20,
    failedJobs: 0,
  },
};

test("product doctor marks semantic Q&A ready while keeping Vercel boundary explicit", async () => {
  const restore = mockFetch({
    status: READY_STATUS,
    models: ["mxbai-embed-large:latest", "qwen2.5:3b"],
  });
  try {
    const report = await buildProductDoctorReport({
      baseUrl: "http://127.0.0.1:7790",
      ollamaEndpoint: "http://127.0.0.1:11434",
    });

    assert.equal(report.ok, true);
    assert.equal(report.service.answerStatus, "ready");
    assert.equal(report.service.embedded, 14597);
    assert.equal(report.service.learningNoteCount, 0);
    assert.equal(report.localAi.preflight.ok, true);
    assert.equal(report.deployment.vercelReady, false);
    assert.match(report.deployment.reason, /本地 SQLite/);
    assert.ok(report.warnings.some((item) => item.includes("来源树仍在后台深加工")));
  } finally {
    restore();
  }
});

test("product doctor fails the delivery gate when semantic vectors are incomplete", async () => {
  const restore = mockFetch({
    status: {
      ...READY_STATUS,
      health: { ...READY_STATUS.health, embeddedChunks: 100, vectorCoveragePercent: 0.7 },
      readiness: { ...READY_STATUS.readiness, answerStatus: "partial", searchStatus: "partial" },
    },
    models: ["mxbai-embed-large:latest", "qwen2.5:3b"],
  });
  try {
    const report = await buildProductDoctorReport({
      baseUrl: "http://127.0.0.1:7790",
      ollamaEndpoint: "http://127.0.0.1:11434",
    });

    assert.equal(report.ok, false);
    assert.ok(report.critical.some((item) => item.includes("语义索引未满")));
    assert.ok(report.critical.some((item) => item.includes("问答和引用未达到就绪")));
  } finally {
    restore();
  }
});

function mockFetch({ status, models }) {
  const original = globalThis.fetch;
  globalThis.fetch = async (url) => {
    const value = String(url);
    if (value.endsWith("/api/status")) return jsonResponse(status);
    if (value.endsWith("/api/tags")) {
      return jsonResponse({ models: models.map((name) => ({ name })) });
    }
    throw new Error(`Unexpected fetch: ${value}`);
  };
  return () => {
    globalThis.fetch = original;
  };
}

function jsonResponse(payload) {
  return {
    ok: true,
    status: 200,
    statusText: "OK",
    async json() {
      return payload;
    },
  };
}

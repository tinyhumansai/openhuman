import assert from "node:assert/strict";
import test from "node:test";

import { setCors } from "./http.mjs";

test("CORS allows every product identity header used by browser requests", () => {
  const headers = {};
  const response = {
    setHeader(name, value) {
      headers[name] = value;
    },
  };

  setCors(response);

  const allowedHeaders = headers["Access-Control-Allow-Headers"]
    .split(",")
    .map(header => header.trim().toLowerCase());
  assert.ok(allowedHeaders.includes("x-sdk-name"));
  assert.ok(allowedHeaders.includes("x-web-version"));
});

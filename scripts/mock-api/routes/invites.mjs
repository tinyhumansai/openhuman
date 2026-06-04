import { json } from "../http.mjs";
import { behavior } from "../state.mjs";

export function handleInvites(ctx) {
  const { method, url, res } = ctx;

  if (method === "POST" && /^\/invite\/redeem\/?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: { message: "Invite code redeemed successfully" },
    });
    return true;
  }
  if (method === "GET" && /^\/invite\/my-codes\/?(\?.*)?$/.test(url)) {
    const rawCodes = behavior().inviteCodes;
    let codes = [];
    if (typeof rawCodes === "string" && rawCodes.length > 0) {
      try {
        const parsed = JSON.parse(rawCodes);
        if (Array.isArray(parsed)) codes = parsed;
      } catch {
        codes = [];
      }
    }
    json(res, 200, { success: true, data: codes });
    return true;
  }
  if (
    method === "GET" &&
    /^\/invite\/status(?:\/[^/?]+)?\/?(\?.*)?$/.test(url)
  ) {
    json(res, 200, { success: true, data: { valid: true } });
    return true;
  }

  return false;
}

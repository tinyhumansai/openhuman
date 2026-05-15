import { json } from "../http.mjs";

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
    json(res, 200, { success: true, data: [] });
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

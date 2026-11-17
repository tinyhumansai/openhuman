// `pnpm debug web --script scripts/debug/web-scripts/pre-stream-failure.mjs`
// Fail the completion request (terminal 400) before any token streams and
// report what the UI shows. openhuman#5729.
export default async function preStreamFailure({
  page,
  mock,
  screenshot,
  log,
}) {
  // Sending before the socket is live is refused client-side, which would
  // look like "no feedback" for the wrong reason.
  await page.waitForFunction(
    () => {
      const s = window.__OPENHUMAN_STORE__?.getState?.().socket?.byUser ?? {};
      return Object.values(s).some((e) => e?.status === "connected");
    },
    null,
    { timeout: 30_000 },
  );
  mock.set("httpFaultRules", [
    {
      contains: "/chat/completions",
      mode: process.env.FAULT_MODE || "status",
      status: 400,
    },
  ]);
  const composer = page.getByRole("textbox", { name: "Message input" });
  await composer.click();
  await composer.pressSequentially("this turn dies before it streams");
  await page.getByRole("button", { name: "Send message" }).click();
  let elapsed = 0;
  for (const wait of [3, 7, 10, 15]) {
    await page.waitForTimeout(wait * 1000);
    elapsed += wait;
    const banner = await page.locator("[data-chat-send-error-code]").count();
    const bubbles = await page
      .locator('[data-testid="agent-message"]')
      .allInnerTexts();
    log(
      `t~${elapsed}s banner=${banner} agentBubbles=${JSON.stringify(bubbles)}`,
    );
    await screenshot(`t${elapsed}`);
  }
}

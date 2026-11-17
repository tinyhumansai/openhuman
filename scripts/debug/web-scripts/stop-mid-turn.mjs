// Example `pnpm debug web --script` scenario: stream a slow reasoning turn
// from the mock LLM, click the composer's Stop button mid-stream, and check
// the turn really ends (Stop disappears, no final answer lands).
//
//   pnpm debug web --script scripts/debug/web-scripts/stop-mid-turn.mjs
const SLOW_THINKING = [
  ...Array.from({ length: 30 }, (_, i) => ({ thinking: `step ${i}. `, delayMs: 1000 })),
  { text: "FINAL-ANSWER", delayMs: 10 },
  { finish: "stop" },
];

export default async function stopMidTurn({ page, mock, screenshot, log }) {
  mock.set("llmStreamScript", SLOW_THINKING);

  const composer = page.getByRole("textbox", { name: "Message input" });
  await composer.click();
  await composer.pressSequentially("think about this for a long time");
  await page.getByRole("button", { name: "Send message" }).click();

  const stop = page.getByTestId("stop-generation-button");
  await stop.waitFor({ state: "visible", timeout: 30_000 });
  await page.waitForTimeout(3_000); // let a few reasoning chunks stream
  await screenshot("before-stop");
  await stop.click();
  log("clicked Stop");

  await stop.waitFor({ state: "hidden", timeout: 10_000 });
  await page.waitForTimeout(3_000);
  await screenshot("after-stop");
  if (await page.getByText("FINAL-ANSWER").isVisible()) {
    throw new Error("the turn kept running after Stop: its final answer landed");
  }
  log("Stop ended the turn");
}

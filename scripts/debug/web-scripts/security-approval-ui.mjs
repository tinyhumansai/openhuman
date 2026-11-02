// Exercise the security settings, whole-tool grant list, policy diagnostics,
// and approval history against the throwaway core and mock backend.
export default async function securityApprovalUi({
  page,
  rpc,
  urls,
  log,
  screenshot,
}) {
  await page.goto(`${urls.app}/#/settings/security`);
  await page.getByText("Always-allowed tools", { exact: true }).waitFor();

  const rawGrants = await rpc("openhuman.approval_list_tool_trust");
  const grants = Array.isArray(rawGrants) ? rawGrants : rawGrants.result;
  if (!Array.isArray(grants)) {
    throw new Error("whole-tool grant RPC did not return a grant list");
  }
  await screenshot("security-tool-grants");

  await page.goto(`${urls.app}/#/settings/tool-policy-diagnostics`);
  await page.getByText("Active security policy", { exact: true }).waitFor();
  await screenshot("active-security-policy");

  await page.route("**/rpc", async (route) => {
    const request = route.request().postDataJSON();
    if (request?.method !== "openhuman.approval_list_recent_decisions") {
      await route.continue();
      return;
    }

    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: request.id,
        result: {
          result: [
            {
              request_id: "jev-history-fixture",
              tool_name: "filesystem.write_file",
              action_summary: "Write a fixture file",
              args_redacted: {},
              session_id: "fixture-session",
              created_at: "2026-01-01T00:00:00Z",
              expires_at: null,
              decided_at: "2026-01-01T00:00:01Z",
              decision: "approve_once",
              decided_by: "auto:jev",
              judge: {
                outcome: "allow",
                intent_millis: 950,
                reversible_millis: 820,
                exfiltrate_millis: 20,
                cached: true,
              },
            },
            {
              request_id: "policy-history-fixture",
              tool_name: "filesystem.write_file",
              action_summary: "Write a policy fixture",
              args_redacted: {},
              session_id: "fixture-session",
              created_at: "2026-01-01T00:00:02Z",
              expires_at: null,
              decided_at: "2026-01-01T00:00:03Z",
              decision: "approve_once",
              decided_by: "auto:policy",
              judge: {
                outcome: "allow",
                intent_millis: 950,
                reversible_millis: 820,
                exfiltrate_millis: 20,
                cached: true,
              },
            },
          ],
          logs: [],
        },
      }),
    });
  });

  await page.goto(`${urls.app}/#/settings/approval-history`);
  await page.getByTestId("approval-history-panel").waitFor();
  const autoRows = page.getByTestId("approval-history-auto");
  const jev = autoRows.nth(0);
  await jev.getByText("Jev", { exact: true }).waitFor();
  await jev.waitFor();
  await jev
    .getByTestId("approval-history-judge-scores")
    .getByText(/95%/)
    .waitFor();
  const policy = autoRows.nth(1);
  await policy.getByText("Policy", { exact: true }).waitFor();
  await policy.waitFor();
  if (await policy.getByTestId("approval-history-judge-scores").count()) {
    throw new Error("policy auto-decisions must not show Jev judge scores");
  }
  await screenshot("approval-history");
  log(
    "security grants, policy diagnostics, and Jev-vs-policy approval history rendered",
  );
}

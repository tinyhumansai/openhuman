import type { Locator, Page } from '@playwright/test';

/**
 * Locate text in the rendered assistant answer, excluding matching text kept
 * in the collapsed processing transcript for the same turn.
 */
export function agentMessageText(page: Page, text: string | RegExp): Locator {
  return page.getByTestId('agent-message').getByText(text).last();
}

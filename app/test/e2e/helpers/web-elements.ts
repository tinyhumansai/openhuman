import type { Locator, Page } from '@playwright/test';

import {
  clearChatComposer,
  composerText,
  replaceChatComposerText,
} from '../../playwright/helpers/chat-composer';

/** Keep browser selectors and platform element operations inside the helper layer. */
function webElement(locator: Locator) {
  return {
    click: () => locator.click(),
    press: (key: string) => locator.press(key),
    type: (text: string) => locator.pressSequentially(text),
    isVisible: () => locator.isVisible(),
    isEnabled: () => locator.isEnabled(),
    isEditable: () => locator.isEditable(),
    count: () => locator.count(),
    text: () => locator.textContent(),
    composerText: () => composerText(locator),
    clearComposer: () => clearChatComposer(locator),
    replaceComposerText: (text: string) => replaceChatComposerText(locator, text),
  };
}

export type WebTestElement = ReturnType<typeof webElement>;

/** Playwright counterpart to the native WebView element helpers. */
export function webElements(page: Page) {
  return {
    byTestId: (id: string) => webElement(page.getByTestId(id)),
    button: (name: string) => webElement(page.getByRole('button', { name, exact: true })),
    alert: (text: string) => webElement(page.getByRole('alert').filter({ hasText: text })),
  };
}

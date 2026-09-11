import { expect, type Locator, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * Real keyboard interaction with settings text fields.
 *
 * # Why a browser lane is required for this
 *
 * jsdom has no caret. `fireEvent.change(input, { target: { value } })` replaces
 * the whole value in one step, so a component that re-renders and resets the
 * selection looks identical to one that does not. Every settings test in
 * `app/src/**` — including the ones I wrote last round — is blind to it.
 *
 * W4 found exactly that defect in the chat composer
 * (`chat-composer-caret.spec.ts`): typing mid-string moves the caret to the end,
 * so the second keystroke lands in the wrong place. This spec asks whether any
 * settings text input shares it.
 *
 * The profile editor is the sharpest place to ask. Its ID field is
 * *programmatically rewritten* while you type the Name
 * (`ProfileEditorPage.tsx:125-128`), which is the exact shape — a controlled
 * input whose value is reassigned during render — that produces caret jumps.
 */

const NEW_PROFILE = '/#/settings/profiles/new';

async function openProfileEditor(page: Page) {
  await page.goto(NEW_PROFILE);
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByLabel('Name', { exact: true })).toBeVisible({ timeout: 30_000 });
}

/** Where the caret sits inside a text input, read from the live DOM. */
function caret(input: Locator): Promise<number | null> {
  return input.evaluate(el => (el as HTMLInputElement).selectionStart);
}

async function placeCaret(input: Locator, offset: number) {
  await input.evaluate((el, at) => {
    const field = el as HTMLInputElement;
    field.focus();
    field.setSelectionRange(at, at);
  }, offset);
}

test.describe('Settings forms — caret behaviour while typing', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-w1-forms');
    await openProfileEditor(page);
  });

  test('typing at the end leaves the caret at the end', async ({ page }) => {
    const name = page.getByLabel('Name', { exact: true });
    await name.click();
    await page.keyboard.type('Research');

    await expect(name).toHaveValue('Research');
    expect(await caret(name)).toBe('Research'.length);
  });

  // The W4 defect, asked of a settings field: type one character into the
  // middle and the caret must sit immediately after it, not jump to the end.
  test('typing mid-string leaves the caret after the inserted character', async ({ page }) => {
    const name = page.getByLabel('Name', { exact: true });
    await name.click();
    await page.keyboard.type('Reearch');

    await placeCaret(name, 2);
    await page.keyboard.type('s');

    await expect(name).toHaveValue('Research');
    expect(await caret(name)).toBe(3);
  });

  // The consequence that makes the chat-composer bug user-visible: if the caret
  // jumps on the first keystroke, the SECOND one lands at the end and the text
  // comes out scrambled. Two characters is the smallest test that shows it.
  test('two consecutive mid-string keystrokes both land where they were typed', async ({
    page,
  }) => {
    const name = page.getByLabel('Name', { exact: true });
    await name.click();
    await page.keyboard.type('Rearch');

    await placeCaret(name, 2);
    await page.keyboard.type('s');
    await page.keyboard.type('e');

    await expect(name).toHaveValue('Research');
    expect(await caret(name)).toBe(4);
  });

  test('backspace mid-string keeps the caret at the deletion point', async ({ page }) => {
    const name = page.getByLabel('Name', { exact: true });
    await name.click();
    await page.keyboard.type('Resxearch');

    await placeCaret(name, 4);
    await page.keyboard.press('Backspace');

    await expect(name).toHaveValue('Research');
    expect(await caret(name)).toBe(3);
  });

  // The ID field is rewritten from the Name on every keystroke until it is
  // touched, so it is the most likely place for a reset-driven caret jump.
  test('the ID field keeps its caret when edited directly', async ({ page }) => {
    const id = page.getByLabel('ID', { exact: true });
    await id.click();
    await page.keyboard.type('reearch-agent');

    await placeCaret(id, 2);
    await page.keyboard.type('s');

    await expect(id).toHaveValue('research-agent');
    expect(await caret(id)).toBe(3);
  });
});

test.describe('Settings forms — the Name/ID coupling, driven by keyboard', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-w1-forms-coupling');
    await openProfileEditor(page);
  });

  test('the ID auto-slugs from the Name as it is typed', async ({ page }) => {
    await page.getByLabel('Name', { exact: true }).click();
    await page.keyboard.type('My Research Agent');

    await expect(page.getByLabel('ID', { exact: true })).toHaveValue('my-research-agent');
  });

  // The `idTouched` latch, in a real browser. My jsdom test of this fires a
  // synthetic change event; this types, which is what a user does and what
  // exercises the per-keystroke re-render.
  test('editing the ID stops the Name from overwriting it', async ({ page }) => {
    const name = page.getByLabel('Name', { exact: true });
    const id = page.getByLabel('ID', { exact: true });

    await name.click();
    await page.keyboard.type('My Research');
    await expect(id).toHaveValue('my-research');

    await id.click();
    await id.press('ControlOrMeta+a');
    await page.keyboard.type('custom-id');

    await name.click();
    await page.keyboard.press('End');
    await page.keyboard.type(' Agent');

    await expect(name).toHaveValue('My Research Agent');
    await expect(id).toHaveValue('custom-id');
  });

  test('Tab moves from Name to the ID field', async ({ page }) => {
    const name = page.getByLabel('Name', { exact: true });
    await name.click();
    await page.keyboard.type('Tabbed');
    await page.keyboard.press('Tab');

    // Focus is what a keyboard user actually experiences; jsdom tests assert
    // values and never this.
    await expect(page.getByLabel('ID', { exact: true })).toBeFocused();
  });

  test('the Create button enables only once a usable id exists', async ({ page }) => {
    const create = page.getByRole('button', { name: 'Create' });
    await expect(create).toBeDisabled();

    // Punctuation-only slugs to '', so it must stay disabled.
    await page.getByLabel('Name', { exact: true }).click();
    await page.keyboard.type('!!!');
    await expect(create).toBeDisabled();

    await page.keyboard.type('Ok');
    await expect(create).toBeEnabled();
  });
});

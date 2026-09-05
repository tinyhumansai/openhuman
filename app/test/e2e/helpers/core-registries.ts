// @ts-nocheck
import { navigateViaHash, waitForHomePage } from './shared-flows';

type TabName = 'Agents' | 'Tools' | 'Connectors';

export async function openCoreRegistriesFromHome(): Promise<void> {
  await navigateViaHash('/home');
  await waitForHomePage(15_000);

  const opened = await browser.execute(() => {
    const button = Array.from(document.querySelectorAll('button')).find(candidate =>
      (candidate.textContent ?? '').includes('Core Registries')
    ) as HTMLButtonElement | undefined;
    button?.click();
    return Boolean(button);
  });
  expect(opened).toBe(true);
  await waitForCoreRegistriesPage();
}

export async function waitForCoreRegistriesPage(timeout = 20_000): Promise<void> {
  await browser.waitUntil(
    async () =>
      browser.execute(() => {
        const heading = document.querySelector('h1');
        return (
          heading?.textContent?.includes('Core Registries') === true &&
          document.body.textContent?.includes('Registry Views') === true
        );
      }),
    { timeout, interval: 250, timeoutMsg: 'Core Registries page did not finish loading' }
  );
}

export async function openRegistryTab(tab: TabName): Promise<void> {
  const opened = await browser.execute((target: string) => {
    const button = Array.from(document.querySelectorAll('[role="tab"]')).find(
      candidate => candidate.textContent?.trim() === target
    ) as HTMLButtonElement | undefined;
    button?.click();
    return Boolean(button);
  }, tab);
  expect(opened).toBe(true);
  await browser.waitUntil(
    async () =>
      browser.execute((target: string) => {
        const button = Array.from(document.querySelectorAll('[role="tab"]')).find(
          candidate => candidate.textContent?.trim() === target
        );
        return button?.getAttribute('aria-selected') === 'true';
      }, tab),
    { timeout: 10_000, interval: 250, timeoutMsg: `${tab} tab did not become active` }
  );
}

export async function clickCollectionRow(
  collectionTitle: string,
  rowText: string,
  requiredText: string[] = [],
  timeout = 20_000
): Promise<void> {
  await browser.waitUntil(
    async () =>
      browser.execute(
        ({ targetCollection, targetRow, required }) => {
          const section = Array.from(document.querySelectorAll('section')).find(
            candidate => candidate.querySelector('h2')?.textContent?.trim() === targetCollection
          );
          if (!section) return false;
          const button = Array.from(section.querySelectorAll('button')).find(candidate => {
            const text = candidate.textContent ?? '';
            return text.includes(targetRow) && required.every(value => text.includes(value));
          }) as HTMLButtonElement | undefined;
          button?.click();
          return Boolean(button);
        },
        { targetCollection: collectionTitle, targetRow: rowText, required: requiredText }
      ),
    {
      timeout,
      interval: 250,
      timeoutMsg: `Row "${rowText}" in ${collectionTitle} was not clickable`,
    }
  );
}

export async function clickActionButton(label: string, timeout = 10_000): Promise<void> {
  await browser.waitUntil(
    async () =>
      browser.execute((target: string) => {
        const button = Array.from(document.querySelectorAll('button')).find(
          candidate => candidate.textContent?.trim() === target
        ) as HTMLButtonElement | undefined;
        button?.click();
        return Boolean(button);
      }, label),
    { timeout, interval: 250, timeoutMsg: `Button "${label}" was not clickable` }
  );
}

export async function waitForDetailHeading(title: string, timeout = 20_000): Promise<void> {
  await browser.waitUntil(
    async () =>
      browser.execute((target: string) => {
        return Array.from(document.querySelectorAll('h2, h3')).some(
          heading => heading.textContent?.trim() === target
        );
      }, title),
    { timeout, interval: 250, timeoutMsg: `Detail heading "${title}" did not appear` }
  );
}

export async function waitForText(text: string, timeout = 20_000): Promise<void> {
  await browser.waitUntil(
    async () =>
      browser.execute(
        (target: string) => document.body.textContent?.includes(target) === true,
        text
      ),
    { timeout, interval: 250, timeoutMsg: `Text "${text}" did not appear` }
  );
}

export async function collectionSnapshot(): Promise<{
  loadMoreButtons: string[];
  tabStates: string[];
  text: string;
}> {
  return browser.execute(() => {
    const text = document.body.textContent ?? '';
    const loadMoreButtons = Array.from(document.querySelectorAll('button'))
      .map(button => button.textContent?.trim() ?? '')
      .filter(textValue => textValue.startsWith('Load more '));
    const tabStates = Array.from(document.querySelectorAll('[role="tab"]')).map(button => {
      const label = button.textContent?.trim() ?? '';
      const selected = button.getAttribute('aria-selected') === 'true' ? 'selected' : 'idle';
      return `${label}:${selected}`;
    });
    return { loadMoreButtons, tabStates, text };
  });
}

export async function detailSnapshot(): Promise<{
  buttons: string[];
  headings: string[];
  text: string;
}> {
  return browser.execute(() => {
    return {
      buttons: Array.from(document.querySelectorAll('button')).map(
        button => button.textContent?.trim() ?? ''
      ),
      headings: Array.from(document.querySelectorAll('h2, h3')).map(
        heading => heading.textContent?.trim() ?? ''
      ),
      text: document.body.textContent ?? '',
    };
  });
}

export async function loadMore(collectionLabel: string): Promise<void> {
  await clickActionButton(collectionLabel);
}

export async function installClipboardProbe(): Promise<void> {
  await browser.execute(() => {
    const probe = async (value: string) => {
      (window as typeof window & { __M224_LAST_CLIPBOARD__?: string }).__M224_LAST_CLIPBOARD__ =
        value;
    };
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: probe },
    });
    (window as typeof window & { __M224_LAST_CLIPBOARD__?: string }).__M224_LAST_CLIPBOARD__ = '';
  });
}

export async function readClipboardProbe(): Promise<string> {
  return browser.execute(() => {
    return (
      (window as typeof window & { __M224_LAST_CLIPBOARD__?: string }).__M224_LAST_CLIPBOARD__ ?? ''
    );
  });
}

/* global window, document, getComputedStyle, location */
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { URL } from 'node:url';

// Use the repository's shared browser element adapter, including in this CLI scenario.
createRequire(new URL('../../../../package.json', import.meta.url))('tsx/cjs');
const { browserElements } = createRequire(import.meta.url)('../helpers/element-helpers.ts');

export default async function uiPolish({ page, mock, screenshot, log }) {
  const ui = browserElements(page);
  const steps = [
    { content: 'Inspect the conversation', status: 'in_progress' },
    { content: 'Verify the next turn', status: 'pending' },
  ];
  mock.set(
    'llmKeywordRules',
    JSON.stringify([{ keyword: '', content: 'FIRST_CANARY: the checklist is complete.' }])
  );
  await page.waitForFunction(() => {
    const id = window.__OPENHUMAN_CORE_STATE__?.()?.snapshot?.auth?.userId;
    return (
      id && window.__OPENHUMAN_STORE__?.getState().socket?.byUser?.[id]?.status === 'connected'
    );
  });
  await page.waitForFunction(() => {
    const state = window.__OPENHUMAN_STORE__.getState().thread;
    return (
      state.selectedThreadId &&
      !state.isLoadingThreads &&
      !state.isLoadingMessages &&
      location.hash.endsWith(state.selectedThreadId)
    );
  });
  mock.set(
    'llmForcedResponses',
    JSON.stringify([
      {
        content: '',
        toolCalls: [
          { id: 'plan-start', name: 'todo', arguments: JSON.stringify({ todos: steps }) },
        ],
      },
      {
        content: '',
        toolCalls: [
          {
            id: 'plan-done',
            name: 'todo',
            arguments: JSON.stringify({
              todos: steps.map(step => ({ ...step, status: 'completed' })),
            }),
          },
        ],
      },
      {
        content:
          'FIRST_CANARY\n\n' +
          Array.from(
            { length: 14 },
            (_, i) => `Paragraph ${i}: a stable historical response.`
          ).join('\n\n'),
      },
    ])
  );
  await ui.textbox('Message input').fill('Execute the two-step checklist and report the result.');
  await ui.textbox('Message input').press('Enter');
  await page.waitForFunction(
    () =>
      document
        .querySelector('[data-slot="aui_assistant-message-content"]')
        ?.textContent?.includes('FIRST_CANARY'),
    undefined,
    { timeout: 60000 }
  );
  await ui.testId('stop-generation-button').waitFor({ state: 'hidden', timeout: 60000 });
  await page.waitForFunction(
    () => document.querySelector('[data-testid="todo-checklist"]')?.dataset.state === 'done'
  );
  await ui.testId('sidebar-nav-separator').waitFor({ state: 'visible' });
  assert.equal(await ui.slot('aui_thread-list-search').count(), 0);
  const geometry = await page.evaluate(() => {
    const card = document.querySelector('[data-testid="todo-checklist"]');
    const content = card.closest('[data-slot="aui_assistant-message-content"]');
    return {
      cardWidth: card.getBoundingClientRect().width,
      contentWidth:
        content.getBoundingClientRect().width -
        parseFloat(getComputedStyle(content).paddingLeft) -
        parseFloat(getComputedStyle(content).paddingRight),
      maxWidth: getComputedStyle(card).maxWidth,
      inComposer: !!card.closest('form'),
      state: card.dataset.state,
      blur: getComputedStyle(card).backdropFilter,
      opacity: getComputedStyle(card).opacity,
    };
  });
  assert.equal(geometry.maxWidth, 'none');
  assert.equal(geometry.inComposer, false);
  assert.equal(geometry.state, 'done');
  assert.equal(geometry.blur, 'none');
  assert.equal(geometry.opacity, '1');
  assert.ok(Math.abs(geometry.cardWidth - geometry.contentWidth) < 2);
  await page.evaluate(() => {
    window.__historyRoots = [...document.querySelectorAll('[data-role]')];
    window.__historyGroups = [...document.querySelectorAll('[data-slot="tool-group-root"]')];
    window.__finishedPlan = document.querySelector('[data-testid="todo-checklist"]');
    window.__finishedPlanParent = window.__finishedPlan.closest('[data-role]');
  });
  mock.set(
    'llmStreamScript',
    JSON.stringify([
      { thinking: 'Thinking about the next turn. ', delayMs: 1500 },
      { thinking: 'Keep the previous messages in order. ', delayMs: 1500 },
      { text: 'SECOND_CANARY', delayMs: 1000 },
      { text: ' The earlier response remains unchanged.', delayMs: 1000 },
      { finish: 'stop' },
    ])
  );
  await page.evaluate(() => {
    const viewport = document.querySelector('[data-slot="aui_thread-viewport"]');
    viewport.scrollTop = Math.max(0, viewport.scrollHeight - viewport.clientHeight - 200);
    window.__typingScroll = [];
    window.__typingScrollListener = () => window.__typingScroll.push(viewport.scrollTop);
    viewport.addEventListener('scroll', window.__typingScrollListener);
    window.__typingScroll.push(viewport.scrollTop);
  });
  await ui.textbox('Message input').pressSequentially('try now', { delay: 60 });
  const typingScroll = await page.evaluate(() => {
    const viewport = document.querySelector('[data-slot="aui_thread-viewport"]');
    window.__typingScroll.push(viewport.scrollTop);
    viewport.removeEventListener('scroll', window.__typingScrollListener);
    return window.__typingScroll;
  });
  assert.ok(
    Math.max(...typingScroll) - Math.min(...typingScroll) <= 2,
    'typing must not flicker or move the reading position'
  );
  await ui.textbox('Message input').press('Enter');
  await ui.testId('stop-generation-button').waitFor({ state: 'visible' });
  const loader = await page.evaluate(
    () => !!document.querySelector('[data-slot="aui_thread-list-item-running"]')
  );
  assert.equal(loader, true, 'a running thread must have a visible loader');
  for (let sample = 0; sample < 8; sample++) {
    const stable = await page.evaluate(() => {
      const roots = [...document.querySelectorAll('[data-role]')];
      return {
        roots: window.__historyRoots.every((root, i) => roots[i] === root),
        groups: window.__historyGroups.every(group => group.isConnected),
        plan:
          window.__finishedPlan.isConnected &&
          window.__finishedPlan.closest('[data-role]') === window.__finishedPlanParent,
        planState: window.__finishedPlan.dataset.state,
      };
    });
    assert.deepEqual(stable, { roots: true, groups: true, plan: true, planState: 'done' });
    // Sampling the running stream is intentionally paced; assertions are on DOM identity.
    await page.waitForTimeout(300);
  }
  await ui.testId('stop-generation-button').waitFor({ state: 'hidden', timeout: 60000 });
  await page.waitForFunction(() =>
    [...document.querySelectorAll('[data-slot="aui_assistant-message-content"]')].some(el =>
      el.textContent.includes('SECOND_CANARY')
    )
  );
  const messages = await page.evaluate(() =>
    [...document.querySelectorAll('[data-role]')].map(el => ({
      role: el.dataset.role,
      first: el.textContent.includes('FIRST_CANARY'),
      second: el.textContent.includes('SECOND_CANARY'),
      retry: el.dataset.role === 'user' && el.textContent.includes('try now'),
    }))
  );
  assert.deepEqual(
    messages.map(message => message.role),
    ['user', 'assistant', 'user', 'assistant']
  );
  assert.equal(messages[1].first, true);
  assert.equal(messages[2].retry, true);
  assert.equal(messages[3].second, true);
  await screenshot('next-turn-stable');
  log(
    'PASS: real assistant-ui send/stream/settle preserves message order, DOM identity, completed-plan attachment, widths, typing scroll stability, sidebar divider and running loader'
  );
}

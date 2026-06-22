import { describe, expect, it } from 'vitest';

import { parseMarkdownTable, splitAgentMessageIntoBubbles } from '../agentMessageBubbles';

describe('splitAgentMessageIntoBubbles', () => {
  it('returns a single bubble when there are no newlines', () => {
    expect(splitAgentMessageIntoBubbles('One line only.')).toEqual(['One line only.']);
  });

  it('returns no bubbles for empty or whitespace-only content', () => {
    expect(splitAgentMessageIntoBubbles('')).toEqual([]);
    expect(splitAgentMessageIntoBubbles(' \n\n ')).toEqual([]);
  });

  it('keeps single-paragraph newline-separated lines in one bubble', () => {
    expect(splitAgentMessageIntoBubbles('First line\nSecond line\nThird line')).toEqual([
      'First line\nSecond line\nThird line',
    ]);
  });

  it('ignores empty lines between bubbles', () => {
    expect(splitAgentMessageIntoBubbles('First line\n\nSecond line\n\n\nThird line')).toEqual([
      'First line',
      'Second line',
      'Third line',
    ]);
  });

  it('keeps fenced code blocks together as one bubble', () => {
    const content = 'Here is code\n```ts\nconst x = 1;\nconst y = 2;\n```\nDone';
    expect(splitAgentMessageIntoBubbles(content)).toEqual([
      'Here is code',
      '```ts\nconst x = 1;\nconst y = 2;\n```',
      'Done',
    ]);
  });

  it('normalizes windows newlines', () => {
    expect(splitAgentMessageIntoBubbles('First\r\nSecond\r\nThird')).toEqual([
      'First\nSecond\nThird',
    ]);
  });

  it('keeps markdown tables together as one segment', () => {
    const content =
      'Summary\n| Name | Value |\n| --- | --- |\n| Alpha | 1 |\n| Beta | 2 |\nNext step';
    expect(splitAgentMessageIntoBubbles(content)).toEqual([
      'Summary',
      '| Name | Value |\n| --- | --- |\n| Alpha | 1 |\n| Beta | 2 |',
      'Next step',
    ]);
  });

  it('keeps double-newline paragraphs in the same bubble', () => {
    const content = 'First line\nSecond line\n\nThird paragraph\nFourth line';
    expect(splitAgentMessageIntoBubbles(content)).toEqual([
      'First line\nSecond line',
      'Third paragraph\nFourth line',
    ]);
  });

  it('normalizes 3+ newlines down to double before splitting', () => {
    const content = 'One\n\n\n\nTwo\n\n\nThree';
    expect(splitAgentMessageIntoBubbles(content)).toEqual(['One', 'Two', 'Three']);
  });

  it('treats <hr> as a bubble break', () => {
    const content = 'First section\n<hr>\nSecond section\n\n<hr />\nThird section';
    expect(splitAgentMessageIntoBubbles(content)).toEqual([
      'First section',
      'Second section',
      'Third section',
    ]);
  });

  it('never returns an hr-only bubble', () => {
    expect(splitAgentMessageIntoBubbles('<hr>')).toEqual([]);
    expect(splitAgentMessageIntoBubbles('Before\n\n<hr />\n\nAfter')).toEqual(['Before', 'After']);
  });

  it('never returns a markdown thematic-break-only bubble', () => {
    expect(splitAgentMessageIntoBubbles('---')).toEqual([]);
    expect(splitAgentMessageIntoBubbles('***')).toEqual([]);
    expect(splitAgentMessageIntoBubbles('___')).toEqual([]);
    expect(splitAgentMessageIntoBubbles('Before\n\n---\n\nAfter')).toEqual(['Before', 'After']);
  });

  // Issue #3807: a heading and its body must never land in separate bubbles.
  it('keeps an ATX heading together with its body in one bubble', () => {
    expect(splitAgentMessageIntoBubbles('## 📅 Calendar\n\n- 9am standup')).toEqual([
      '## 📅 Calendar\n\n- 9am standup',
    ]);
  });

  it('keeps a bold-line heading together with its body in one bubble', () => {
    expect(splitAgentMessageIntoBubbles('**Tasks**\n\n- Ship the PR')).toEqual([
      '**Tasks**\n\n- Ship the PR',
    ]);
  });

  it('renders a multi-section morning briefing as one bubble per section', () => {
    const briefing = [
      'Good morning! Here is what today looks like.',
      '## 📅 Calendar',
      '- 9:00 Standup\n- 14:00 Design review',
      '## ✅ Tasks',
      '- Finish the proposal (due today)',
      '## 📧 Emails',
      '2 unread threads from key contacts',
    ].join('\n\n');

    const bubbles = splitAgentMessageIntoBubbles(briefing);

    expect(bubbles).toEqual([
      'Good morning! Here is what today looks like.',
      '## 📅 Calendar\n\n- 9:00 Standup\n- 14:00 Design review',
      '## ✅ Tasks\n\n- Finish the proposal (due today)',
      '## 📧 Emails\n\n2 unread threads from key contacts',
    ]);

    // No bubble is a heading with no body, and no body bubble lacks its heading.
    for (const bubble of bubbles.slice(1)) {
      const lines = bubble.split('\n').filter(line => line.trim().length > 0);
      expect(lines.length).toBeGreaterThan(1);
      expect(lines[0].startsWith('#')).toBe(true);
    }
  });

  it('absorbs trailing closing lines into the final section bubble', () => {
    const briefing = '## 📧 Emails\n\n2 unread threads\n\nHave a great day! ☀️';
    expect(splitAgentMessageIntoBubbles(briefing)).toEqual([
      '## 📧 Emails\n\n2 unread threads\n\nHave a great day! ☀️',
    ]);
  });

  it('keeps a table out of the heading bubble so the table renderer still fires', () => {
    // A table folded behind a heading would not sit at the bubble start and
    // would render as raw pipe text, so it stays its own segment.
    const content = '## 📅 Calendar\n\n| Time | Event |\n| --- | --- |\n| 9:00 | Standup |';
    expect(splitAgentMessageIntoBubbles(content)).toEqual([
      '## 📅 Calendar',
      '| Time | Event |\n| --- | --- |\n| 9:00 | Standup |',
    ]);
  });

  it('still groups body paragraphs that follow a table under their heading', () => {
    const content =
      '## 📅 Calendar\n\n| Time | Event |\n| --- | --- |\n| 9:00 | Standup |\n\n## ✅ Tasks\n\n- Ship the PR';
    expect(splitAgentMessageIntoBubbles(content)).toEqual([
      '## 📅 Calendar',
      '| Time | Event |\n| --- | --- |\n| 9:00 | Standup |',
      '## ✅ Tasks\n\n- Ship the PR',
    ]);
  });

  it('folds a trailing body-less heading into the previous bubble', () => {
    const content = '## 📅 Calendar\n\n- 9:00 Standup\n\n## ✅ Tasks';
    expect(splitAgentMessageIntoBubbles(content)).toEqual([
      '## 📅 Calendar\n\n- 9:00 Standup\n\n## ✅ Tasks',
    ]);
  });

  it('does not treat inline bold emphasis with trailing prose as a heading', () => {
    // Only a fully-bold line is a heading; emphasis followed by prose stays a
    // plain paragraph so ordinary content is not misclassified.
    const content = '**Heads up:** the meeting moved\n\nSee you at 3pm';
    expect(splitAgentMessageIntoBubbles(content)).toEqual([
      '**Heads up:** the meeting moved',
      'See you at 3pm',
    ]);
  });
});

describe('parseMarkdownTable', () => {
  it('parses a markdown table into headers and rows', () => {
    expect(
      parseMarkdownTable('| Name | Value |\n| --- | --- |\n| Alpha | 1 |\n| Beta | 2 |')
    ).toEqual({
      headers: ['Name', 'Value'],
      rows: [
        ['Alpha', '1'],
        ['Beta', '2'],
      ],
    });
  });

  it('returns null for non-table content', () => {
    expect(parseMarkdownTable('Just a normal message')).toBeNull();
  });
});

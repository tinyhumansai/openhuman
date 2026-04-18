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

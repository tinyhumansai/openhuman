import { cleanup, fireEvent, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  type MeetCallTranscriptLine,
  parseTranscriptLine,
} from '../../../services/meetCallService';
import { renderWithProviders } from '../../../test/test-utils';
import TranscriptViewer from '../TranscriptViewer';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const lineWithPrefix: MeetCallTranscriptLine = {
  role: 'participant',
  content: '[1:23] [Alice] Hello there!',
};

const lineWithoutPrefix: MeetCallTranscriptLine = {
  role: 'assistant',
  content: 'How can I help you?',
};

// ── parseTranscriptLine unit tests ──────────────────────────────────────────

describe('parseTranscriptLine', () => {
  it('parses a line with [MM:SS] [Name] prefix', () => {
    const result = parseTranscriptLine(lineWithPrefix);
    expect(result.timestamp).toBe('1:23');
    expect(result.speaker).toBe('Alice');
    expect(result.text).toBe('Hello there!');
    expect(result.role).toBe('participant');
  });

  it('returns null timestamp and speaker when prefix is absent', () => {
    const result = parseTranscriptLine(lineWithoutPrefix);
    expect(result.timestamp).toBeNull();
    expect(result.speaker).toBeNull();
    expect(result.text).toBe('How can I help you?');
    expect(result.role).toBe('assistant');
  });

  it('handles partial brackets — no match, returns full content', () => {
    const line: MeetCallTranscriptLine = {
      role: 'participant',
      content: '[1:23] missing second bracket',
    };
    const result = parseTranscriptLine(line);
    expect(result.timestamp).toBeNull();
    expect(result.speaker).toBeNull();
    expect(result.text).toBe('[1:23] missing second bracket');
  });

  it('handles malformed content gracefully', () => {
    const line: MeetCallTranscriptLine = { role: 'participant', content: 'just plain text' };
    const result = parseTranscriptLine(line);
    expect(result.timestamp).toBeNull();
    expect(result.speaker).toBeNull();
    expect(result.text).toBe('just plain text');
  });
});

// ── TranscriptViewer component tests ────────────────────────────────────────

describe('TranscriptViewer', () => {
  it('renders speaker label and timestamp when prefix present', () => {
    renderWithProviders(<TranscriptViewer lines={[lineWithPrefix]} />);
    expect(screen.getByText('1:23')).toBeInTheDocument();
    expect(screen.getByText('Alice:')).toBeInTheDocument();
    expect(screen.getByText('Hello there!')).toBeInTheDocument();
  });

  it('renders plain content when no prefix', () => {
    renderWithProviders(<TranscriptViewer lines={[lineWithoutPrefix]} />);
    expect(screen.getByText('How can I help you?')).toBeInTheDocument();
    // No timestamp or speaker
    expect(screen.queryByText(':')).toBeNull();
  });

  it('applies primary color class to assistant lines', () => {
    const { container } = renderWithProviders(<TranscriptViewer lines={[lineWithoutPrefix]} />);
    const p = container.querySelector('p.text-primary-600');
    expect(p).not.toBeNull();
  });

  it('applies secondary color class to non-assistant lines', () => {
    const { container } = renderWithProviders(<TranscriptViewer lines={[lineWithPrefix]} />);
    const p = container.querySelector('p.text-content-secondary');
    expect(p).not.toBeNull();
  });

  it('copy button calls navigator.clipboard.writeText with plain text', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText },
      writable: true,
      configurable: true,
    });

    renderWithProviders(<TranscriptViewer lines={[lineWithPrefix]} />);
    fireEvent.click(screen.getByText('Copy'));

    // Wait for async clipboard write
    await vi.waitFor(() => {
      expect(writeText).toHaveBeenCalledWith('1:23 Alice: Hello there!');
    });
  });

  it('download button creates a blob and triggers download', () => {
    const createObjectURL = vi.fn().mockReturnValue('blob:test');
    const revokeObjectURL = vi.fn();
    Object.defineProperty(URL, 'createObjectURL', {
      value: createObjectURL,
      writable: true,
      configurable: true,
    });
    Object.defineProperty(URL, 'revokeObjectURL', {
      value: revokeObjectURL,
      writable: true,
      configurable: true,
    });

    // Save original before mocking to avoid infinite recursion
    const originalCreateElement = document.createElement.bind(document);
    const clickSpy = vi.fn();
    const createElementSpy = vi
      .spyOn(document, 'createElement')
      .mockImplementation((tag: string) => {
        const el = originalCreateElement(tag);
        if (tag === 'a') {
          el.click = clickSpy;
        }
        return el;
      });

    renderWithProviders(<TranscriptViewer lines={[lineWithPrefix]} />);
    fireEvent.click(screen.getByText('Download'));

    expect(createObjectURL).toHaveBeenCalled();
    expect(clickSpy).toHaveBeenCalled();
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:test');

    createElementSpy.mockRestore();
  });
});

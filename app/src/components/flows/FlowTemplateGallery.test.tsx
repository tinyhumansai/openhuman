/**
 * FlowTemplateGallery — the curated-template picker. Asserts the bundled
 * templates render as selectable cards, clicking one calls `onSelect` with
 * that template, and the whole grid disables while a create is in flight.
 * `useT()` falls back to the bundled English map with no provider mounted
 * (same as the sibling flows tests).
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { FLOW_TEMPLATES } from '../../lib/flows/templates';
import FlowTemplateGallery from './FlowTemplateGallery';

describe('FlowTemplateGallery', () => {
  it('renders one card per bundled template', () => {
    render(<FlowTemplateGallery onSelect={vi.fn()} />);
    expect(screen.getByTestId('flow-template-gallery')).toBeInTheDocument();
    for (const template of FLOW_TEMPLATES) {
      expect(screen.getByTestId(`flow-template-${template.id}`)).toBeInTheDocument();
    }
  });

  it('calls onSelect with the clicked template', () => {
    const onSelect = vi.fn();
    render(<FlowTemplateGallery onSelect={onSelect} />);
    const first = FLOW_TEMPLATES[0];
    fireEvent.click(screen.getByTestId(`flow-template-${first.id}`));
    expect(onSelect).toHaveBeenCalledWith(first);
  });

  it('disables every card while a template is being created', () => {
    render(<FlowTemplateGallery onSelect={vi.fn()} busyId={FLOW_TEMPLATES[0].id} />);
    for (const template of FLOW_TEMPLATES) {
      expect(screen.getByTestId(`flow-template-${template.id}`)).toBeDisabled();
    }
  });
});

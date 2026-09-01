/**
 * ProfileEditorPage — the save payload, id stickiness, and the failure path.
 *
 * `ProfileEditorPage.test.tsx` (sibling) covers the happy path: slug derivation,
 * hydration, not-found, the allowlist chips and the toggle rows. It stops at
 * `expect(sent.id)` / `expect(sent.name)`, so the rest of the object
 * `handleSubmit` builds is unasserted — and that object is what reaches the RPC
 * layer. This file covers the normalisation in `handleSubmit`
 * (`ProfileEditorPage.tsx:137-166`), the `idTouched` latch, and what happens
 * when the upsert rejects.
 *
 * Deliberately a separate file rather than an edit to the sibling: three of us
 * are working in this directory.
 */
import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { agentProfilesApi } from '../../../../services/api/agentProfilesApi';
import agentProfilesReducer from '../../../../store/agentProfileSlice';
import type { AgentProfile } from '../../../../types/agentProfile';
import ProfileEditorPage from '../ProfileEditorPage';

vi.mock('../../../../services/api/agentProfilesApi', () => ({
  agentProfilesApi: { list: vi.fn(), select: vi.fn(), upsert: vi.fn(), delete: vi.fn() },
}));

const mockNavigate = vi.fn();
vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => mockNavigate };
});

const mockUpsert = vi.mocked(agentProfilesApi.upsert);

function profile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: 'writer',
    name: 'Writer',
    description: 'Drafts copy.',
    agentId: 'orchestrator',
    builtIn: false,
    ...overrides,
  };
}

function renderAt(path: string, profiles: AgentProfile[] = []) {
  const store = configureStore({
    reducer: { agentProfiles: agentProfilesReducer },
    preloadedState: {
      agentProfiles: { profiles, activeProfileId: 'default', status: 'idle' as const, error: null },
    },
  });
  return render(
    <Provider store={store}>
      <MemoryRouter initialEntries={[path]}>
        <Routes>
          <Route path="/settings/profiles/new" element={<ProfileEditorPage />} />
          <Route path="/settings/profiles/edit/:id" element={<ProfileEditorPage />} />
        </Routes>
      </MemoryRouter>
    </Provider>
  );
}

const nameField = () => screen.getByLabelText('Name');
const idField = () => screen.getByLabelText('ID') as HTMLInputElement;
const submit = (label: string) => screen.getByText(label).closest('button')!;

async function sentPayload() {
  await waitFor(() => expect(mockUpsert).toHaveBeenCalled());
  return mockUpsert.mock.calls[0][0];
}

describe('ProfileEditorPage — id stickiness', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockUpsert.mockResolvedValue({ profiles: [], activeProfileId: 'default' });
  });

  // `handleName` re-slugs the id only while the user has not touched it
  // (`ProfileEditorPage.tsx:125-128`). Without the latch, typing the rest of a
  // name after choosing an id silently overwrites the chosen id.
  it('stops re-slugging the id once the user edits it', () => {
    renderAt('/settings/profiles/new');

    fireEvent.change(nameField(), { target: { value: 'My Research' } });
    expect(idField().value).toBe('my-research');

    fireEvent.change(idField(), { target: { value: 'custom-id' } });
    fireEvent.change(nameField(), { target: { value: 'My Research Assistant' } });

    expect(idField().value).toBe('custom-id');
  });

  it('keeps re-slugging while the id is untouched', () => {
    renderAt('/settings/profiles/new');

    fireEvent.change(nameField(), { target: { value: 'One' } });
    expect(idField().value).toBe('one');
    fireEvent.change(nameField(), { target: { value: 'One Two' } });
    expect(idField().value).toBe('one-two');
  });

  it('submits the typed id rather than the slug of the name', async () => {
    renderAt('/settings/profiles/new');

    fireEvent.change(nameField(), { target: { value: 'My Research' } });
    fireEvent.change(idField(), { target: { value: '  custom-id  ' } });
    fireEvent.click(submit('Create'));

    expect((await sentPayload()).id).toBe('custom-id');
  });

  it('falls back to the slug of the name when the id box is cleared', async () => {
    renderAt('/settings/profiles/new');

    fireEvent.change(nameField(), { target: { value: 'My Research' } });
    fireEvent.change(idField(), { target: { value: '' } });
    // The latch is set, so the id no longer tracks the name — but `resolvedId`
    // still falls back to the slug rather than submitting an empty id.
    fireEvent.click(submit('Create'));

    expect((await sentPayload()).id).toBe('my-research');
  });

  it('does not offer an editable id in edit mode', () => {
    renderAt('/settings/profiles/edit/writer', [profile()]);
    expect(screen.queryByLabelText('ID')).not.toBeInTheDocument();
  });
});

describe('ProfileEditorPage — save payload normalisation', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockUpsert.mockResolvedValue({ profiles: [], activeProfileId: 'default' });
  });

  it('trims the name and description', async () => {
    renderAt('/settings/profiles/new');

    fireEvent.change(nameField(), { target: { value: '  Spaced Name  ' } });
    fireEvent.change(screen.getByLabelText('Description'), {
      target: { value: '  padded description  ' },
    });
    fireEvent.click(submit('Create'));

    const sent = await sentPayload();
    expect(sent.name).toBe('Spaced Name');
    expect(sent.description).toBe('padded description');
  });

  // `name: name.trim() || id` — a whitespace-only name must not reach the RPC
  // layer as an empty string; the id stands in for it.
  it('falls back to the id when the name is only whitespace', async () => {
    renderAt('/settings/profiles/new');

    fireEvent.change(nameField(), { target: { value: 'Temp' } });
    fireEvent.change(idField(), { target: { value: 'kept-id' } });
    fireEvent.change(nameField(), { target: { value: '   ' } });
    fireEvent.click(submit('Create'));

    const sent = await sentPayload();
    expect(sent.name).toBe('kept-id');
  });

  it('sends null rather than an empty string for the optional text fields', async () => {
    renderAt('/settings/profiles/new');

    fireEvent.change(nameField(), { target: { value: 'Blank Optionals' } });
    fireEvent.click(submit('Create'));

    const sent = await sentPayload();
    expect(sent.modelOverride).toBeNull();
    expect(sent.systemPromptSuffix).toBeNull();
    expect(sent.soulMd).toBeNull();
    expect(sent.temperature).toBeNull();
  });

  it('defaults a blank agent id to orchestrator', async () => {
    renderAt('/settings/profiles/new');

    fireEvent.change(nameField(), { target: { value: 'Blank Agent' } });
    const agent = screen.getByLabelText('Base agent');
    fireEvent.change(agent, { target: { value: '   ' } });
    fireEvent.click(submit('Create'));

    expect((await sentPayload()).agentId).toBe('orchestrator');
  });

  it('parses a numeric temperature', async () => {
    renderAt('/settings/profiles/new');

    fireEvent.change(nameField(), { target: { value: 'Warm' } });
    fireEvent.change(screen.getByLabelText('Temperature'), { target: { value: '0.7' } });
    fireEvent.click(submit('Create'));

    expect((await sentPayload()).temperature).toBe(0.7);
  });

  // `Number('abc')` is NaN, and NaN would serialise as `null` in JSON anyway —
  // but only after passing through the payload as NaN. The `Number.isFinite`
  // guard (`ProfileEditorPage.tsx:154`) is what keeps a non-numeric entry from
  // becoming a NaN temperature.
  it.each(['abc', '--', 'Infinity'])(
    'sends null for a non-finite temperature entry %s',
    async value => {
      renderAt('/settings/profiles/new');

      fireEvent.change(nameField(), { target: { value: 'Bad Temp' } });
      fireEvent.change(screen.getByLabelText('Temperature'), { target: { value } });
      fireEvent.click(submit('Create'));

      const sent = await sentPayload();
      expect(sent.temperature).toBeNull();
      expect(Number.isNaN(sent.temperature as unknown as number)).toBe(false);
    }
  );

  it('preserves builtIn when editing a built-in profile', async () => {
    renderAt('/settings/profiles/edit/writer', [profile({ builtIn: true })]);

    fireEvent.change(nameField(), { target: { value: 'Renamed Builtin' } });
    fireEvent.click(submit('Save'));

    const sent = await sentPayload();
    expect(sent.builtIn).toBe(true);
    expect(sent.id).toBe('writer');
  });

  it('marks a newly created profile as not built-in', async () => {
    renderAt('/settings/profiles/new');

    fireEvent.change(nameField(), { target: { value: 'Mine' } });
    fireEvent.click(submit('Create'));

    expect((await sentPayload()).builtIn).toBe(false);
  });
});

describe('ProfileEditorPage — failure path', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockUpsert.mockResolvedValue({ profiles: [], activeProfileId: 'default' });
  });

  it('surfaces an alert, stays on the page, and re-enables the button', async () => {
    mockUpsert.mockRejectedValueOnce(new Error('backend refused the profile'));
    renderAt('/settings/profiles/new');

    fireEvent.change(nameField(), { target: { value: 'Doomed' } });
    const create = submit('Create');
    fireEvent.click(create);

    expect(await screen.findByRole('alert')).toBeInTheDocument();
    // Navigating away on a failed save would lose the user's unsaved form.
    expect(mockNavigate).not.toHaveBeenCalled();
    await waitFor(() => expect(create).not.toBeDisabled());
  });

  // Was pinned as a known defect (#5900): the alert read "[object Object]"
  // because `dispatch(thunk).unwrap()` rejects with Redux Toolkit's
  // SerializedError — a plain object, not an `Error` — and the old
  // `err instanceof Error ? err.message : String(err)` guard stringified it.
  // Fixed by routing through `lib/errorMessage`; the assertion is inverted
  // rather than deleted, so a regression re-surfaces here.
  it('renders the failure message, not [object Object]', async () => {
    mockUpsert.mockRejectedValueOnce(new Error('backend refused the profile'));
    renderAt('/settings/profiles/new');

    fireEvent.change(nameField(), { target: { value: 'Doomed' } });
    fireEvent.click(submit('Create'));

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('backend refused the profile');
    expect(alert).not.toHaveTextContent('[object Object]');
  });

  // The shape that actually reaches this catch in production: a thunk that
  // threw is rethrown by `.unwrap()` as a SerializedError, never an `Error`.
  it('renders the message from a SerializedError-shaped rejection', async () => {
    mockUpsert.mockRejectedValueOnce({
      name: 'Error',
      message: 'profile id already taken',
      stack: 'at …',
    });
    renderAt('/settings/profiles/new');

    fireEvent.change(nameField(), { target: { value: 'Doomed' } });
    fireEvent.click(submit('Create'));

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('profile id already taken');
    expect(alert).not.toHaveTextContent('[object Object]');
  });

  it('clears a previous error when the next save succeeds', async () => {
    mockUpsert.mockRejectedValueOnce(new Error('transient failure'));
    renderAt('/settings/profiles/new');

    fireEvent.change(nameField(), { target: { value: 'Retry' } });
    fireEvent.click(submit('Create'));
    expect(await screen.findByRole('alert')).toBeInTheDocument();

    fireEvent.click(submit('Create'));
    await waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument());
    expect(mockNavigate).toHaveBeenCalledWith('/settings/profiles');
  });

  it('navigates back to the list only after a successful save', async () => {
    renderAt('/settings/profiles/new');

    fireEvent.change(nameField(), { target: { value: 'Fine' } });
    expect(mockNavigate).not.toHaveBeenCalled();
    fireEvent.click(submit('Create'));

    await waitFor(() => expect(mockNavigate).toHaveBeenCalledWith('/settings/profiles'));
  });

  it('does not show not-found while the profile list is still empty', () => {
    // `notFound` is only set once a list has arrived
    // (`ProfileEditorPage.tsx:99-102`); flagging it during load would blame the
    // user for a race.
    renderAt('/settings/profiles/edit/missing', []);
    expect(screen.queryByText(/not found/i)).not.toBeInTheDocument();
  });
});

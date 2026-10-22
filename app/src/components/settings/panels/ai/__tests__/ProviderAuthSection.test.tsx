import { configureStore } from '@reduxjs/toolkit';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ComponentProps } from 'react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import { I18nProvider } from '../../../../../lib/i18n/I18nContext';
import type { Locale } from '../../../../../lib/i18n/types';
import { CoreStateContext } from '../../../../../providers/coreStateContext';
import localeReducer from '../../../../../store/localeSlice';
import { createLocalSessionToken } from '../../../../../utils/localSession';
import { type CloudProvider, EMPTY_SETTINGS } from '../aiPanelTypes';
import { ProviderAuthSection } from '../ProviderAuthSection';

function renderSection(
  sessionToken: string | null,
  extra: Partial<ComponentProps<typeof ProviderAuthSection>> = {}
) {
  const store = configureStore({
    reducer: { locale: localeReducer },
    preloadedState: { locale: { current: 'en' as Locale } },
  });
  return render(
    <Provider store={store}>
      <I18nProvider>
        <CoreStateContext.Provider value={{ snapshot: { sessionToken } } as never}>
          <ProviderAuthSection
            draft={EMPTY_SETTINGS}
            persist={vi.fn().mockResolvedValue(undefined)}
            loading={false}
            error=""
            busyAction={null}
            providerAuthErrors={[]}
            providerSaveNotice={null}
            onDismissProviderSaveNotice={vi.fn()}
            onProviderRemoved={vi.fn()}
            codexAuthError={null}
            onConnectCodex={vi.fn()}
            onConnectProvider={vi.fn().mockResolvedValue(undefined)}
            onOpenKeyDialog={vi.fn()}
            onAddCustomProvider={vi.fn()}
            onEditCustomProvider={vi.fn()}
            {...extra}
          />
        </CoreStateContext.Provider>
      </I18nProvider>
    </Provider>
  );
}

describe('ProviderAuthSection managed row', () => {
  it('shows the managed OpenHuman row for a normal session', () => {
    renderSection('header.payload.signature');
    expect(screen.getByTestId('provider-row-openhuman')).toBeInTheDocument();
  });

  it('hides the managed row for a local ("Continue Locally") session', () => {
    renderSection(createLocalSessionToken());
    expect(screen.queryByTestId('provider-row-openhuman')).not.toBeInTheDocument();
  });

  it('drops the whole Connected group when nothing is connected', () => {
    // The group used to be guaranteed non-empty by the always-on managed row.
    // With that row gated on a real session, a local user who has not added a
    // key yet was left with a "Connected" heading over an empty card.
    renderSection(createLocalSessionToken());
    expect(screen.queryByTestId('provider-group-connected')).not.toBeInTheDocument();
  });

  it('hides the managed row when signed out', () => {
    renderSection(null);
    expect(screen.queryByTestId('provider-row-openhuman')).not.toBeInTheDocument();
  });
});

describe('ProviderAuthSection wizard affordances', () => {
  it('shows the standalone "Add a provider" button by default', () => {
    renderSection('header.payload.signature');
    expect(screen.getByTestId('add-provider-open')).toBeInTheDocument();
  });

  it('hides the standalone "Add a provider" button when hideAddButton is set', () => {
    renderSection('header.payload.signature', { hideAddButton: true });
    expect(screen.queryByTestId('add-provider-open')).not.toBeInTheDocument();
  });

  // The description promises a managed fallback and cites the Routing tab; both
  // are false without a managed session.
  it('describes the Connected group only when a managed session exists', () => {
    const managed = renderSection('header.payload.signature');
    expect(screen.getByTestId('provider-group-connected')).toHaveTextContent(
      'Managed is always on as a fallback'
    );
    managed.unmount();

    // A local session has nothing connected, so the group — and with it the
    // claim about a managed fallback and the Routing tab — is gone entirely.
    renderSection(createLocalSessionToken());
    expect(screen.queryByTestId('provider-group-connected')).not.toBeInTheDocument();
    expect(screen.queryByText(/Managed is always on as a fallback/)).not.toBeInTheDocument();
  });
});

/**
 * Rows inside the "Connected" group.
 *
 * The group is now gated on actually having something connected, so these rows
 * only render when the draft carries providers. Each shape has its own branch
 * (builtin cloud, user-defined custom, local runtime), and each is rendered
 * here so the gate cannot hide a broken row.
 */
describe('ProviderAuthSection connected rows', () => {
  const provider = (over: Partial<CloudProvider> = {}): CloudProvider => ({
    id: 'p1',
    slug: 'openai',
    label: 'OpenAI',
    endpoint: 'https://api.openai.com/v1',
    authStyle: 'bearer' as CloudProvider['authStyle'],
    maskedKey: 'sk-…4f2a',
    ...over,
  });

  it('renders a connected builtin cloud provider with its masked key', () => {
    renderSection('header.payload.signature', {
      draft: { ...EMPTY_SETTINGS, cloudProviders: [provider()] },
    });

    expect(screen.getByTestId('provider-group-connected')).toBeInTheDocument();
    expect(screen.getByTestId('provider-row-openai')).toHaveTextContent('sk-…4f2a');
  });

  it('summarizes a connected OCI provider and edits it through the OCI dialog', async () => {
    renderSection('header.payload.signature', {
      draft: {
        ...EMPTY_SETTINGS,
        cloudProviders: [
          provider({
            id: 'p_oci',
            slug: 'oci',
            label: 'OCI Generative AI',
            endpoint: 'https://inference.generativeai.us-chicago-1.oci.oraclecloud.com',
            authStyle: 'oci',
            maskedKey: 'Not configured',
            oci: {
              auth_mode: 'session_token',
              transport: 'native_generic',
              region: 'us-chicago-1',
              compartment_id: 'ocid1.compartment.oc1..example',
            },
          }),
        ],
      },
    });

    const row = screen.getByTestId('provider-row-oci');
    // Region and sign-in method instead of a key the entry does not hold.
    expect(row).toHaveTextContent('us-chicago-1 · Session token');

    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: /OCI Generative AI/ }));
    await user.click(await screen.findByRole('menuitem', { name: 'Edit OCI settings' }));
    expect(await screen.findByTestId('oci-auth-mode')).toHaveValue('session_token');
  });

  it('edits a custom OCI slug through the OCI dialog, not the generic editor', async () => {
    const onEditCustomProvider = vi.fn();
    const onConnectProvider = vi.fn().mockResolvedValue(undefined);
    renderSection('header.payload.signature', {
      onEditCustomProvider,
      onConnectProvider,
      draft: {
        ...EMPTY_SETTINGS,
        cloudProviders: [
          provider({
            id: 'p_oci-cohere',
            slug: 'oci-cohere',
            label: 'OCI Cohere',
            endpoint: '',
            authStyle: 'oci',
            maskedKey: 'Not configured',
            oci: {
              auth_mode: 'config_profile',
              transport: 'native_cohere',
              region: 'eu-frankfurt-1',
              compartment_id: 'ocid1.compartment.oc1..example',
            },
          }),
        ],
      },
    });

    expect(screen.getByTestId('provider-row-oci-cohere')).toHaveTextContent(
      'eu-frankfurt-1 · OCI config profile'
    );
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: /OCI Cohere/ }));
    await user.click(await screen.findByRole('menuitem', { name: 'Edit' }));
    expect(onEditCustomProvider).not.toHaveBeenCalled();
    expect(await screen.findByTestId('oci-auth-mode')).toHaveValue('config_profile');
  });

  it('renders a user-defined custom provider by host, with edit and remove', () => {
    renderSection('header.payload.signature', {
      draft: {
        ...EMPTY_SETTINGS,
        cloudProviders: [
          provider({ id: 'c1', slug: 'my-proxy', label: 'My Proxy', maskedKey: '' }),
        ],
      },
    });

    const row = screen.getByTestId('provider-row-my-proxy');
    expect(row).toBeInTheDocument();
    // A custom provider shows where it points rather than a key it may not have.
    expect(row).toHaveTextContent('api.openai.com');
  });

  it('renders a local runtime with its full endpoint, not just the host', () => {
    renderSection('header.payload.signature', {
      draft: {
        ...EMPTY_SETTINGS,
        cloudProviders: [
          provider({
            id: 'l1',
            slug: 'ollama',
            label: 'Ollama',
            endpoint: 'http://localhost:11434',
            maskedKey: '',
          }),
        ],
      },
    });

    expect(screen.getByTestId('provider-row-ollama')).toHaveTextContent('http://localhost:11434');
  });

  it('keeps the group for a local session once a provider is connected', () => {
    // The group disappears only while nothing is connected — a local user who
    // has added their own key still needs to see it.
    renderSection(createLocalSessionToken(), {
      draft: { ...EMPTY_SETTINGS, cloudProviders: [provider()] },
    });

    expect(screen.getByTestId('provider-group-connected')).toBeInTheDocument();
    expect(screen.queryByTestId('provider-row-openhuman')).not.toBeInTheDocument();
  });
});

/**
 * Unit tests for the Agent World invoke API client bridge.
 *
 * Mocks `callCoreRpc` and asserts:
 * 1. Each client method calls the correct `openhuman.tinyplace_*` RPC method.
 * 2. Parameters are marshalled correctly.
 * 3. A `PAYMENT_REQUIRED:` rejection becomes a `PaymentRequiredError`.
 * 4. Other errors propagate unchanged.
 */
import { beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';
import { createInvokeApiClient, PaymentRequiredError } from './invokeApiClient';

vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockCallCoreRpc = callCoreRpc as Mock;

beforeEach(() => {
  vi.clearAllMocks();
});

// ── directory.listAgents ──────────────────────────────────────────────────────

describe('directory.listAgents', () => {
  test('calls openhuman.tinyplace_directory_list_agents with params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ agents: [] });
    const client = createInvokeApiClient();
    const params = { q: 'ai assistant', limit: 10 };
    await client.directory.listAgents(params);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_directory_list_agents',
      params: { params },
    });
  });

  test('calls without params (null)', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ agents: [] });
    const client = createInvokeApiClient();
    await client.directory.listAgents();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_directory_list_agents',
      params: { params: null },
    });
  });

  test('returns the response from core', async () => {
    const mockResponse = { agents: [{ agentId: 'abc123', name: 'Test Agent' }] };
    mockCallCoreRpc.mockResolvedValueOnce(mockResponse);
    const client = createInvokeApiClient();
    const result = await client.directory.listAgents();
    expect(result).toEqual(mockResponse);
  });
});

// ── directory.getAgent ────────────────────────────────────────────────────────

describe('directory.getAgent', () => {
  test('calls openhuman.tinyplace_directory_get_agent with agentId', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ agentId: 'abc123' });
    const client = createInvokeApiClient();
    await client.directory.getAgent('abc123');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_directory_get_agent',
      params: { agentId: 'abc123' },
    });
  });
});

// ── explorer.overview ─────────────────────────────────────────────────────────

describe('explorer.overview', () => {
  test('calls openhuman.tinyplace_explorer_overview with no params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ totalAgents: 42 });
    const client = createInvokeApiClient();
    await client.explorer.overview();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_explorer_overview',
      params: undefined,
    });
  });
});

// ── search.unified ────────────────────────────────────────────────────────────

describe('search.unified', () => {
  test('calls openhuman.tinyplace_search_unified with query', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ results: [] });
    const client = createInvokeApiClient();
    await client.search.unified('coding assistant');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_search_unified',
      params: { query: 'coding assistant' },
    });
  });
});

// ── directory.resolve ─────────────────────────────────────────────────────────

describe('directory.resolve', () => {
  test('calls openhuman.tinyplace_directory_resolve with name', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ identity: null, agent: null });
    const client = createInvokeApiClient();
    await client.directory.resolve('alice.agent');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_directory_resolve',
      params: { name: 'alice.agent' },
    });
  });

  test('returns the ResolveResponse from core', async () => {
    const mockResponse = { identity: { name: 'alice.agent' }, agent: null };
    mockCallCoreRpc.mockResolvedValueOnce(mockResponse);
    const client = createInvokeApiClient();
    const result = await client.directory.resolve('alice.agent');
    expect(result).toEqual(mockResponse);
  });
});

// ── directory.reverse ─────────────────────────────────────────────────────────

describe('directory.reverse', () => {
  test('calls openhuman.tinyplace_directory_reverse with cryptoId', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ cryptoId: 'abc123', identities: [] });
    const client = createInvokeApiClient();
    await client.directory.reverse('HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_directory_reverse',
      params: { cryptoId: 'HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk' },
    });
  });
});

// ── directory.listIdentities ──────────────────────────────────────────────────

describe('directory.listIdentities', () => {
  test('calls openhuman.tinyplace_directory_list_identities with params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ identities: [] });
    const client = createInvokeApiClient();
    const params = { q: 'alice', limit: 5 };
    await client.directory.listIdentities(params);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_directory_list_identities',
      params: { params },
    });
  });

  test('calls without params (null)', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ identities: [] });
    const client = createInvokeApiClient();
    await client.directory.listIdentities();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_directory_list_identities',
      params: { params: null },
    });
  });
});

// ── directory.skills ──────────────────────────────────────────────────────────

describe('directory.skills', () => {
  test('calls openhuman.tinyplace_directory_skills with params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ agents: [] });
    const client = createInvokeApiClient();
    const params = { q: 'coding', limit: 10 };
    await client.directory.skills(params);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_directory_skills',
      params: { params },
    });
  });

  test('calls without params (null)', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ agents: [] });
    const client = createInvokeApiClient();
    await client.directory.skills();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_directory_skills',
      params: { params: null },
    });
  });
});

// ── PaymentRequiredError ──────────────────────────────────────────────────────

describe('PaymentRequiredError propagation', () => {
  test('402 string rejection becomes PaymentRequiredError', async () => {
    const challenge = { error: 'payment required', payment: { scheme: 'x402', amount: '0.01' } };
    mockCallCoreRpc.mockRejectedValueOnce(
      new Error(`PAYMENT_REQUIRED:${JSON.stringify(challenge)}`)
    );
    const client = createInvokeApiClient();
    await expect(client.explorer.overview()).rejects.toBeInstanceOf(PaymentRequiredError);
  });

  test('PaymentRequiredError.challenge contains the parsed challenge', async () => {
    const challenge = { error: 'payment required', payment: { scheme: 'x402', amount: '0.01' } };
    mockCallCoreRpc.mockRejectedValueOnce(
      new Error(`PAYMENT_REQUIRED:${JSON.stringify(challenge)}`)
    );
    const client = createInvokeApiClient();
    let caught: PaymentRequiredError | null = null;
    try {
      await client.search.unified('test');
    } catch (e) {
      caught = e as PaymentRequiredError;
    }
    expect(caught).toBeInstanceOf(PaymentRequiredError);
    expect(caught?.challenge).toEqual(challenge);
  });

  test('non-402 errors propagate unchanged', async () => {
    const networkErr = new Error('network failure');
    mockCallCoreRpc.mockRejectedValueOnce(networkErr);
    const client = createInvokeApiClient();
    await expect(client.directory.listAgents()).rejects.toBe(networkErr);
  });
});

// ── profiles.get ─────────────────────────────────────────────────────────────

describe('profiles.get', () => {
  test('calls openhuman.tinyplace_profiles_get with username', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ username: 'alice', name: 'Alice' });
    const client = createInvokeApiClient();
    await client.profiles.get('alice');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_profiles_get',
      params: { username: 'alice' },
    });
  });

  test('returns the AgentProfile from core', async () => {
    const profile = { username: 'bob', name: 'Bob', cryptoId: 'abc123' };
    mockCallCoreRpc.mockResolvedValueOnce(profile);
    const client = createInvokeApiClient();
    const result = await client.profiles.get('bob');
    expect(result).toEqual(profile);
  });
});

// ── profiles.activity ─────────────────────────────────────────────────────────

describe('profiles.activity', () => {
  test('calls openhuman.tinyplace_profiles_activity with username', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ events: [] });
    const client = createInvokeApiClient();
    await client.profiles.activity('alice');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_profiles_activity',
      params: { username: 'alice' },
    });
  });
});

// ── profiles.groups ───────────────────────────────────────────────────────────

describe('profiles.groups', () => {
  test('calls openhuman.tinyplace_profiles_groups with username', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ groups: [] });
    const client = createInvokeApiClient();
    await client.profiles.groups('alice');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_profiles_groups',
      params: { username: 'alice' },
    });
  });

  test('returns groups response', async () => {
    const resp = { groups: [{ groupId: 'g1', name: 'Devs' }] };
    mockCallCoreRpc.mockResolvedValueOnce(resp);
    const client = createInvokeApiClient();
    const result = await client.profiles.groups('alice');
    expect(result).toEqual(resp);
  });
});

// ── profiles.broadcasts ───────────────────────────────────────────────────────

describe('profiles.broadcasts', () => {
  test('calls openhuman.tinyplace_profiles_broadcasts with username', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ broadcasts: [] });
    const client = createInvokeApiClient();
    await client.profiles.broadcasts('alice');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_profiles_broadcasts',
      params: { username: 'alice' },
    });
  });
});

// ── profiles.attestations ─────────────────────────────────────────────────────

describe('profiles.attestations', () => {
  test('calls openhuman.tinyplace_profiles_attestations with username', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ attestations: [] });
    const client = createInvokeApiClient();
    await client.profiles.attestations('alice');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_profiles_attestations',
      params: { username: 'alice' },
    });
  });
});

// ── profiles.agentCard ────────────────────────────────────────────────────────

describe('profiles.agentCard', () => {
  test('calls openhuman.tinyplace_profiles_agent_card with username', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ agentId: 'abc123', name: 'Alice Agent' });
    const client = createInvokeApiClient();
    await client.profiles.agentCard('alice');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_profiles_agent_card',
      params: { username: 'alice' },
    });
  });
});

// ── users.get ─────────────────────────────────────────────────────────────────

describe('users.get', () => {
  test('calls openhuman.tinyplace_users_get with cryptoId', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ cryptoId: 'xyz789', displayName: 'Alice' });
    const client = createInvokeApiClient();
    await client.users.get('xyz789');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_users_get',
      params: { cryptoId: 'xyz789' },
    });
  });

  test('returns User from core', async () => {
    const user = { cryptoId: 'xyz789', displayName: 'Alice', bio: 'Hello' };
    mockCallCoreRpc.mockResolvedValueOnce(user);
    const client = createInvokeApiClient();
    const result = await client.users.get('xyz789');
    expect(result).toEqual(user);
  });
});

// ── users.updateProfile ───────────────────────────────────────────────────────

describe('users.updateProfile', () => {
  test('calls openhuman.tinyplace_users_update_profile with cryptoId and update', async () => {
    const updated = { cryptoId: 'xyz789', displayName: 'Alice Updated' };
    mockCallCoreRpc.mockResolvedValueOnce(updated);
    const client = createInvokeApiClient();
    const update = { displayName: 'Alice Updated', bio: 'New bio' };
    await client.users.updateProfile('xyz789', update);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_users_update_profile',
      params: { cryptoId: 'xyz789', update },
    });
  });

  test('returns updated User from core', async () => {
    const updated = { cryptoId: 'xyz789', displayName: 'Alice Updated', bio: 'New bio' };
    mockCallCoreRpc.mockResolvedValueOnce(updated);
    const client = createInvokeApiClient();
    const result = await client.users.updateProfile('xyz789', { displayName: 'Alice Updated' });
    expect(result).toEqual(updated);
  });
});

// ── PaymentRequiredError ──────────────────────────────────────────────────────

describe('PaymentRequiredError propagation', () => {
  test('402 string rejection becomes PaymentRequiredError', async () => {
    const challenge = { error: 'payment required', payment: { scheme: 'x402', amount: '0.01' } };
    mockCallCoreRpc.mockRejectedValueOnce(
      new Error(`PAYMENT_REQUIRED:${JSON.stringify(challenge)}`)
    );
    const client = createInvokeApiClient();
    await expect(client.explorer.overview()).rejects.toBeInstanceOf(PaymentRequiredError);
  });

  test('PaymentRequiredError.challenge contains the parsed challenge', async () => {
    const challenge = { error: 'payment required', payment: { scheme: 'x402', amount: '0.01' } };
    mockCallCoreRpc.mockRejectedValueOnce(
      new Error(`PAYMENT_REQUIRED:${JSON.stringify(challenge)}`)
    );
    const client = createInvokeApiClient();
    let caught: PaymentRequiredError | null = null;
    try {
      await client.search.unified('test');
    } catch (e) {
      caught = e as PaymentRequiredError;
    }
    expect(caught).toBeInstanceOf(PaymentRequiredError);
    expect(caught?.challenge).toEqual(challenge);
  });

  test('non-402 errors propagate unchanged', async () => {
    const networkErr = new Error('network failure');
    mockCallCoreRpc.mockRejectedValueOnce(networkErr);
    const client = createInvokeApiClient();
    await expect(client.directory.listAgents()).rejects.toBe(networkErr);
  });
});

describe('registry.export', () => {
  test('calls openhuman.tinyplace_registry_export with name', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      identity: { username: '@testhandle' },
      ledgerTransactions: [],
      exportedAt: '2025-06-15T12:00:00Z',
      verification: {},
      proofs: { ownership: {}, ledgerReferences: [] },
    });
    const client = createInvokeApiClient();
    await client.registry.export('@testhandle');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_registry_export',
      params: { name: '@testhandle' },
    });
  });
});

describe('registry.get', () => {
  test('calls openhuman.tinyplace_registry_get with name', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ available: true, name: '@atlas' });
    const client = createInvokeApiClient();
    await client.registry.get('@atlas');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_registry_get',
      params: { name: '@atlas' },
    });
  });

  test('returns availability response', async () => {
    const mockResponse = { available: false, name: '@taken', identity: { cryptoId: 'abc' } };
    mockCallCoreRpc.mockResolvedValueOnce(mockResponse);
    const client = createInvokeApiClient();
    const result = await client.registry.get('@taken');
    expect(result).toEqual(mockResponse);
  });
});
describe('marketplace.listIdentities', () => {
  test('calls openhuman.tinyplace_marketplace_list_identities with status', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ identities: [] });
    const client = createInvokeApiClient();
    await client.marketplace.listIdentities({ status: 'active' });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_list_identities',
      params: { limit: null, status: 'active' },
    });
  });

  test('calls without params (null values)', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ identities: [] });
    const client = createInvokeApiClient();
    await client.marketplace.listIdentities();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_list_identities',
      params: { limit: null, status: null },
    });
  });
});
describe('marketplace.identityFloor', () => {
  test('calls openhuman.tinyplace_marketplace_identity_floor with length', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ length: 3, price: { amount: '250', asset: 'USDC' } });
    const client = createInvokeApiClient();
    await client.marketplace.identityFloor(3);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_identity_floor',
      params: { length: 3 },
    });
  });

  test('calls without length (null)', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({});
    const client = createInvokeApiClient();
    await client.marketplace.identityFloor();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_identity_floor',
      params: { length: null },
    });
  });
});
describe('marketplace.recent', () => {
  test('calls openhuman.tinyplace_marketplace_recent with no params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ sales: [] });
    const client = createInvokeApiClient();
    await client.marketplace.recent();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_recent',
      params: undefined,
    });
  });
});
describe('marketplace.identitySaleHistory', () => {
  test('calls openhuman.tinyplace_marketplace_identity_sale_history with name', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ history: [] });
    const client = createInvokeApiClient();
    await client.marketplace.identitySaleHistory('@atlas');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_identity_sale_history',
      params: { name: '@atlas' },
    });
  });
});
describe('marketplace.listBids', () => {
  test('calls openhuman.tinyplace_marketplace_list_bids with listingId', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ bids: [] });
    const client = createInvokeApiClient();
    await client.marketplace.listBids('listing-123');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_list_bids',
      params: { listingId: 'listing-123' },
    });
  });
});
describe('marketplace.listOffers', () => {
  test('calls openhuman.tinyplace_marketplace_list_offers with name filter', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ offers: [] });
    const client = createInvokeApiClient();
    await client.marketplace.listOffers({ name: '@atlas' });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_list_offers',
      params: { name: '@atlas', buyer: null },
    });
  });

  test('calls with buyer filter', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ offers: [] });
    const client = createInvokeApiClient();
    await client.marketplace.listOffers({ buyer: '@buyer' });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_list_offers',
      params: { name: null, buyer: '@buyer' },
    });
  });

  test('calls without filters (null values)', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ offers: [] });
    const client = createInvokeApiClient();
    await client.marketplace.listOffers();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_list_offers',
      params: { name: null, buyer: null },
    });
  });
});

describe('marketplace.browseMarketplace', () => {
  test('calls openhuman.tinyplace_marketplace_browse with params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ products: [] });
    const client = createInvokeApiClient();
    const params = { q: 'model', category: 'ai' };
    await client.marketplace.browseMarketplace(params);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_browse',
      params: { params },
    });
  });

  test('calls with null when no params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ products: [] });
    const client = createInvokeApiClient();
    await client.marketplace.browseMarketplace();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_browse',
      params: { params: null },
    });
  });
});
describe('marketplace.listProducts', () => {
  test('calls openhuman.tinyplace_marketplace_list_products', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ products: [] });
    const client = createInvokeApiClient();
    await client.marketplace.listProducts({ limit: 10 });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_list_products',
      params: { params: { limit: 10 } },
    });
  });

  test('calls with null params when omitted', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ products: [] });
    const client = createInvokeApiClient();
    await client.marketplace.listProducts();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_list_products',
      params: { params: null },
    });
  });
});
describe('marketplace.getProduct', () => {
  test('calls openhuman.tinyplace_marketplace_get_product with productId', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ productId: 'prod_abc' });
    const client = createInvokeApiClient();
    await client.marketplace.getProduct('prod_abc');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_get_product',
      params: { productId: 'prod_abc' },
    });
  });
});
describe('marketplace.categories', () => {
  test('calls openhuman.tinyplace_marketplace_categories with no params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ categories: [] });
    const client = createInvokeApiClient();
    await client.marketplace.categories();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_categories',
      params: undefined,
    });
  });
});
describe('marketplace.featured', () => {
  test('calls openhuman.tinyplace_marketplace_featured with no params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ items: [] });
    const client = createInvokeApiClient();
    await client.marketplace.featured();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_featured',
      params: undefined,
    });
  });
});
describe('marketplace.listProductReviews', () => {
  test('calls openhuman.tinyplace_marketplace_list_product_reviews with productId', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ reviews: [] });
    const client = createInvokeApiClient();
    await client.marketplace.listProductReviews('prod_xyz');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_marketplace_list_product_reviews',
      params: { productId: 'prod_xyz' },
    });
  });
});
describe('artifacts.list', () => {
  test('calls openhuman.tinyplace_artifacts_list with params and actorId', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ artifacts: [] });
    const client = createInvokeApiClient();
    await client.artifacts.list({ role: 'owner' }, 'agent123');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_artifacts_list',
      params: { params: { role: 'owner' }, actorId: 'agent123' },
    });
  });

  test('calls with null params when omitted', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ artifacts: [] });
    const client = createInvokeApiClient();
    await client.artifacts.list();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_artifacts_list',
      params: { params: null },
    });
  });
});
describe('artifacts.get', () => {
  test('calls openhuman.tinyplace_artifacts_get with artifactId', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ artifactId: 'art_abc', owner: 'agent123' });
    const client = createInvokeApiClient();
    await client.artifacts.get('art_abc');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_artifacts_get',
      params: { artifactId: 'art_abc' },
    });
  });

  test('passes actorId when provided', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ artifactId: 'art_abc', owner: 'agent123' });
    const client = createInvokeApiClient();
    await client.artifacts.get('art_abc', 'agent456');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_artifacts_get',
      params: { artifactId: 'art_abc', actorId: 'agent456' },
    });
  });
});
describe('escrow.list', () => {
  test('calls openhuman.tinyplace_escrow_list with params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ escrows: [] });
    const client = createInvokeApiClient();
    await client.escrow.list({ status: 'funded' });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_escrow_list',
      params: { params: { status: 'funded' } },
    });
  });

  test('calls with null params when omitted', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ escrows: [] });
    const client = createInvokeApiClient();
    await client.escrow.list();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_escrow_list',
      params: { params: null },
    });
  });
});
describe('escrow.get', () => {
  test('calls openhuman.tinyplace_escrow_get with escrowId', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ escrowId: 'esc_abc', status: 'funded' });
    const client = createInvokeApiClient();
    await client.escrow.get('esc_abc');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_escrow_get',
      params: { escrowId: 'esc_abc' },
    });
  });
});
describe('jobs.list', () => {
  test('calls openhuman.tinyplace_jobs_list with params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ jobs: [] });
    const client = createInvokeApiClient();
    await client.jobs.list({ q: 'rust developer' });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_jobs_list',
      params: { params: { q: 'rust developer' } },
    });
  });

  test('calls with null params when omitted', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ jobs: [] });
    const client = createInvokeApiClient();
    await client.jobs.list();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_jobs_list',
      params: { params: null },
    });
  });
});
describe('jobs.get', () => {
  test('calls openhuman.tinyplace_jobs_get with jobId', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ jobId: 'job_abc', status: 'open' });
    const client = createInvokeApiClient();
    await client.jobs.get('job_abc');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_jobs_get',
      params: { jobId: 'job_abc' },
    });
  });
});

describe('channels.list', () => {
  test('calls openhuman.tinyplace_channels_list with params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ channels: [] });
    const client = createInvokeApiClient();
    const params = { q: 'defi', limit: 10 };
    await client.channels.list(params);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_channels_list',
      params: { params },
    });
  });

  test('calls with null when no params provided', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ channels: [] });
    const client = createInvokeApiClient();
    await client.channels.list();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_channels_list',
      params: { params: null },
    });
  });

  test('returns channel list response from core', async () => {
    const mockResponse = {
      channels: [{ channelId: 'ch1', name: 'General', memberCount: 42, isPublic: true }],
    };
    mockCallCoreRpc.mockResolvedValueOnce(mockResponse);
    const client = createInvokeApiClient();
    const result = await client.channels.list();
    expect(result).toEqual(mockResponse);
  });
});
describe('groups.list', () => {
  test('calls openhuman.tinyplace_groups_list with params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce([]);
    const client = createInvokeApiClient();
    const params = { q: 'research', limit: 5 };
    await client.groups.list(params);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_groups_list',
      params: { params },
    });
  });

  test('calls with null when no params provided', async () => {
    mockCallCoreRpc.mockResolvedValueOnce([]);
    const client = createInvokeApiClient();
    await client.groups.list();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_groups_list',
      params: { params: null },
    });
  });
});
describe('broadcasts.list', () => {
  test('calls openhuman.tinyplace_broadcasts_list with params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce([]);
    const client = createInvokeApiClient();
    const params = { visibility: 'public', limit: 20 };
    await client.broadcasts.list(params);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_broadcasts_list',
      params: { params },
    });
  });

  test('calls with null when no params provided', async () => {
    mockCallCoreRpc.mockResolvedValueOnce([]);
    const client = createInvokeApiClient();
    await client.broadcasts.list();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_broadcasts_list',
      params: { params: null },
    });
  });
});
describe('inbox.list', () => {
  test('calls openhuman.tinyplace_inbox_list with params and no owner', async () => {
    const mockResult = { items: [], unreadCount: 0, totalCount: 0, cursor: null };
    mockCallCoreRpc.mockResolvedValueOnce(mockResult);
    const client = createInvokeApiClient();
    const params = { limit: 30 };
    await client.inbox.list(params);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_inbox_list',
      params: { params, owner: null },
    });
  });

  test('passes owner when provided', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ items: [], unreadCount: 0, totalCount: 0 });
    const client = createInvokeApiClient();
    await client.inbox.list(undefined, 'agent-xyz');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_inbox_list',
      params: { params: null, owner: 'agent-xyz' },
    });
  });

  test('returns inbox list result from core', async () => {
    const mockResult = {
      items: [
        {
          itemId: 'i1',
          type: 'SYSTEM',
          status: 'unread',
          priority: 'normal',
          timestamp: '2024-01-01T00:00:00Z',
          subject: 'Hello',
        },
      ],
      unreadCount: 1,
      totalCount: 1,
    };
    mockCallCoreRpc.mockResolvedValueOnce(mockResult);
    const client = createInvokeApiClient();
    const result = await client.inbox.list();
    expect(result).toEqual(mockResult);
  });
});
describe('inbox.counts', () => {
  test('calls openhuman.tinyplace_inbox_counts with no owner', async () => {
    const mockCounts = { unread: 3, read: 10, archived: 2, byType: {}, urgent: 0 };
    mockCallCoreRpc.mockResolvedValueOnce(mockCounts);
    const client = createInvokeApiClient();
    await client.inbox.counts();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_inbox_counts',
      params: { owner: null },
    });
  });

  test('passes owner when provided', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      unread: 0,
      read: 0,
      archived: 0,
      byType: {},
      urgent: 0,
    });
    const client = createInvokeApiClient();
    await client.inbox.counts('agent-abc');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_inbox_counts',
      params: { owner: 'agent-abc' },
    });
  });

  test('returns counts from core', async () => {
    const mockCounts = { unread: 5, read: 20, archived: 3, byType: { SYSTEM: 2 }, urgent: 1 };
    mockCallCoreRpc.mockResolvedValueOnce(mockCounts);
    const client = createInvokeApiClient();
    const result = await client.inbox.counts();
    expect(result).toEqual(mockCounts);
  });
});

// ── messaging write actions (membership + inbox management) ───────────────────

describe('messaging write methods', () => {
  test('channels.join / leave call the right RPC with channelId', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValue(undefined);
    await client.channels.join('ch-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_channels_join',
      params: { channelId: 'ch-1' },
    });
    await client.channels.leave('ch-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_channels_leave',
      params: { channelId: 'ch-1' },
    });
  });

  test('groups.join / leave call the right RPC with groupId', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValue(undefined);
    await client.groups.join('g-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_groups_join',
      params: { groupId: 'g-1' },
    });
    await client.groups.leave('g-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_groups_leave',
      params: { groupId: 'g-1' },
    });
  });

  test('groups.setMemberRole calls tinyplace_groups_set_member_role', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValue({ groupId: 'g-1', agentId: 'a-1', role: 'admin' });
    await client.groups.setMemberRole('g-1', 'a-1', 'admin');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_groups_set_member_role',
      params: { groupId: 'g-1', agentId: 'a-1', role: 'admin' },
    });
  });

  test('groups.createInvite calls tinyplace_groups_create_invite with optional request', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValue({ token: 'tok-1' });
    await client.groups.createInvite('g-1', { ttlSeconds: 3600, maxUses: 5 });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_groups_create_invite',
      params: { groupId: 'g-1', request: { ttlSeconds: 3600, maxUses: 5 } },
    });
  });

  test('groups.createInvite sends request: null when no options', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValue({ token: 'tok-2' });
    await client.groups.createInvite('g-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_groups_create_invite',
      params: { groupId: 'g-1', request: null },
    });
  });

  test('groups.listInvites calls tinyplace_groups_list_invites', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValue([]);
    await client.groups.listInvites('g-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_groups_list_invites',
      params: { groupId: 'g-1' },
    });
  });

  test('groups.previewInvite calls tinyplace_groups_preview_invite', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValue({ groupId: 'g-1', name: 'Group', valid: true });
    await client.groups.previewInvite('g-1', 'tok-abc');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_groups_preview_invite',
      params: { groupId: 'g-1', token: 'tok-abc' },
    });
  });

  test('groups.revokeInvite calls tinyplace_groups_revoke_invite', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValue(undefined);
    await client.groups.revokeInvite('g-1', 'tok-abc');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_groups_revoke_invite',
      params: { groupId: 'g-1', token: 'tok-abc' },
    });
  });

  test('groups.redeemInvite calls tinyplace_groups_redeem_invite', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValue({ groupId: 'g-1', agentId: 'me', role: 'member' });
    await client.groups.redeemInvite('g-1', 'tok-join');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_groups_redeem_invite',
      params: { groupId: 'g-1', token: 'tok-join' },
    });
  });

  test('broadcasts.subscribe / unsubscribe call the right RPC with broadcastId', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValue(undefined);
    await client.broadcasts.subscribe('bc-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_broadcasts_subscribe',
      params: { broadcastId: 'bc-1' },
    });
    await client.broadcasts.unsubscribe('bc-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_broadcasts_unsubscribe',
      params: { broadcastId: 'bc-1' },
    });
  });

  test('inbox.markRead / archive / unarchive / remove pass itemId + null owner by default', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValue(undefined);
    await client.inbox.markRead('item-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_inbox_mark_read',
      params: { itemId: 'item-1', owner: null },
    });
    await client.inbox.archive('item-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_inbox_archive',
      params: { itemId: 'item-1', owner: null },
    });
    await client.inbox.unarchive('item-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_inbox_unarchive',
      params: { itemId: 'item-1', owner: null },
    });
    await client.inbox.remove('item-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_inbox_remove',
      params: { itemId: 'item-1', owner: null },
    });
  });

  test('inbox.markRead forwards an explicit owner', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValue(undefined);
    await client.inbox.markRead('item-2', 'agent-owner');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_inbox_mark_read',
      params: { itemId: 'item-2', owner: 'agent-owner' },
    });
  });

  test('inbox.markAllRead passes params=null + owner default', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValue(undefined);
    await client.inbox.markAllRead();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_inbox_mark_all_read',
      params: { params: null, owner: null },
    });
  });

  // ── Feedback namespace ────────────────────────────────────────────────────

  test('feedback.list calls tinyplace_feedback_list with params wrapped in params key', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValueOnce({ feedback: [] });
    await client.feedback.list({ status: 'open', limit: 10 });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_feedback_list',
      params: { params: { status: 'open', limit: 10 } },
    });
  });

  test('feedback.list without params sends params: null', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValueOnce({ feedback: [] });
    await client.feedback.list();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_feedback_list',
      params: { params: null },
    });
  });

  test('feedback.get calls tinyplace_feedback_get with feedbackId', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValueOnce({ feedbackId: 'fb-1' });
    await client.feedback.get('fb-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_feedback_get',
      params: { feedbackId: 'fb-1' },
    });
  });

  test('feedback.create calls tinyplace_feedback_create with title and description', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValueOnce({ feedbackId: 'fb-new' });
    await client.feedback.create('My idea', 'Great description');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_feedback_create',
      params: { title: 'My idea', description: 'Great description' },
    });
  });

  test('feedback.create includes category when provided', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValueOnce({ feedbackId: 'fb-cat' });
    await client.feedback.create('Idea', 'Desc', 'feature');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_feedback_create',
      params: { title: 'Idea', description: 'Desc', category: 'feature' },
    });
  });

  test('feedback.vote calls tinyplace_feedback_vote with feedbackId and vote', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValueOnce({ feedbackId: 'fb-1', score: 1 });
    await client.feedback.vote('fb-1', 'up');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_feedback_vote',
      params: { feedbackId: 'fb-1', vote: 'up' },
    });
  });

  test('feedback.vote accepts "down" direction', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValueOnce({ feedbackId: 'fb-1', score: -1 });
    await client.feedback.vote('fb-1', 'down');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_feedback_vote',
      params: { feedbackId: 'fb-1', vote: 'down' },
    });
  });

  // ── Solana namespace ──────────────────────────────────────────────────────

  test('solana.info calls openhuman.tinyplace_solana_info with no params', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValueOnce({
      network: 'solana-devnet',
      name: 'Solana Devnet',
      kind: 'testnet',
      nativeAsset: 'SOL',
      explorerUrl: 'https://explorer.solana.com',
      confirmations: 31,
      assets: [
        { symbol: 'SOL', decimals: 9 },
        { symbol: 'USDC', address: '4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU', decimals: 6 },
      ],
      rpc: { url: 'https://rpc.example.com', rateLimitPerMin: 600, fallbacks: true },
    });
    await client.solana.info();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_solana_info',
      params: undefined,
    });
  });

  test('solana.rpcCall calls openhuman.tinyplace_solana_call with method and params', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValueOnce({ value: 1000000000 });
    await client.solana.rpcCall('getBalance', ['4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU']);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_solana_call',
      params: {
        method: 'getBalance',
        params: ['4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU'],
        id: null,
      },
    });
  });

  test('solana.rpcCall sends null params and id when omitted', async () => {
    const client = createInvokeApiClient();
    mockCallCoreRpc.mockResolvedValueOnce(42);
    await client.solana.rpcCall('getSlot');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_solana_call',
      params: { method: 'getSlot', params: null, id: null },
    });
  });
});

// ── signal namespace ──────────────────────────────────────────────────────────

test('signal namespace has expected methods', () => {
  const client = createInvokeApiClient();
  expect(client.signal).toBeDefined();
  expect(typeof client.signal.provision).toBe('function');
  expect(typeof client.signal.uploadPreKeys).toBe('function');
  expect(typeof client.signal.rotateSignedPreKey).toBe('function');
  expect(typeof client.signal.getBundle).toBe('function');
  expect(typeof client.signal.keyStatus).toBe('function');
});

describe('signal.provision', () => {
  test('calls openhuman.tinyplace_signal_provision with preKeyCount', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ agentId: 'abc', oneTimePreKeyCount: 100 });
    const client = createInvokeApiClient();
    await client.signal.provision(50);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_signal_provision',
      params: { preKeyCount: 50 },
    });
  });

  test('sends preKeyCount: null when omitted', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ agentId: 'abc', oneTimePreKeyCount: 100 });
    const client = createInvokeApiClient();
    await client.signal.provision();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_signal_provision',
      params: { preKeyCount: null },
    });
  });
});

describe('signal.uploadPreKeys', () => {
  test('calls openhuman.tinyplace_signal_upload_pre_keys with count', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ agentId: 'abc', oneTimePreKeyCount: 200 });
    const client = createInvokeApiClient();
    await client.signal.uploadPreKeys(50);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_signal_upload_pre_keys',
      params: { count: 50 },
    });
  });

  test('sends count: null when omitted', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ agentId: 'abc', oneTimePreKeyCount: 200 });
    const client = createInvokeApiClient();
    await client.signal.uploadPreKeys();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_signal_upload_pre_keys',
      params: { count: null },
    });
  });
});

describe('signal.rotateSignedPreKey', () => {
  test('calls openhuman.tinyplace_signal_rotate_signed_pre_key with empty params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ ok: true, keyId: 'spk_123' });
    const client = createInvokeApiClient();
    await client.signal.rotateSignedPreKey();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_signal_rotate_signed_pre_key',
      params: {},
    });
  });
});

describe('signal.getBundle', () => {
  test('calls openhuman.tinyplace_signal_get_bundle with agentId', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ agentId: 'peer123', identityKey: 'abc' });
    const client = createInvokeApiClient();
    await client.signal.getBundle('peer123');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_signal_get_bundle',
      params: { agentId: 'peer123' },
    });
  });
});

describe('signal.keyStatus', () => {
  test('calls openhuman.tinyplace_signal_key_status with empty params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      agentId: 'abc',
      localPreKeyCount: 42,
      hasActiveSignedPreKey: true,
      remote: null,
    });
    const client = createInvokeApiClient();
    await client.signal.keyStatus();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_signal_key_status',
      params: {},
    });
  });
});

test('signal namespace has registerEncryptionKey method', () => {
  const client = createInvokeApiClient();
  expect(typeof client.signal.registerEncryptionKey).toBe('function');
});

test('directory namespace has findByEncryptionKey method', () => {
  const client = createInvokeApiClient();
  expect(typeof client.directory.findByEncryptionKey).toBe('function');
});

describe('signal.sendMessage and messages namespace', () => {
  test('signal namespace has send/decrypt methods and messages namespace exists', () => {
    const client = createInvokeApiClient();
    expect(typeof client.signal.sendMessage).toBe('function');
    expect(typeof client.signal.decryptMessage).toBe('function');
    expect(client.messages).toBeDefined();
    expect(typeof client.messages.list).toBe('function');
    expect(typeof client.messages.acknowledge).toBe('function');
  });

  test('signal.sendMessage calls openhuman.tinyplace_signal_send_message with params object', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      messageId: 'msg-1',
      timestamp: '2026-06-17T00:00:00Z',
      encrypted: true,
    });
    const client = createInvokeApiClient();
    await client.signal.sendMessage({ recipient: 'peer-123', plaintext: 'Hello!' });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_signal_send_message',
      params: { recipient: 'peer-123', plaintext: 'Hello!' },
    });
  });

  test('signal.decryptMessage calls openhuman.tinyplace_signal_decrypt_message with envelope', async () => {
    const envelope = {
      id: 'env-1',
      from: 'alice',
      to: 'bob',
      timestamp: '2026-06-17T00:00:00Z',
      deviceId: 1,
      type: 'CIPHERTEXT',
      body: 'base64ciphertext==',
    };
    mockCallCoreRpc.mockResolvedValueOnce({
      plaintext: 'Hello!',
      from: 'alice',
      messageId: 'env-1',
    });
    const client = createInvokeApiClient();
    await client.signal.decryptMessage({ envelope });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_signal_decrypt_message',
      params: { envelope },
    });
  });

  test('messages.list calls openhuman.tinyplace_messages_list with params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ messages: [] });
    const client = createInvokeApiClient();
    await client.messages.list({ limit: 25 });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_messages_list',
      params: { limit: 25 },
    });
  });

  test('messages.list sends empty params when called without args', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ messages: [] });
    const client = createInvokeApiClient();
    await client.messages.list();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_messages_list',
      params: {},
    });
  });

  test('messages.acknowledge calls openhuman.tinyplace_messages_acknowledge', async () => {
    mockCallCoreRpc.mockResolvedValueOnce(undefined);
    const client = createInvokeApiClient();
    await client.messages.acknowledge('msg-99');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_messages_acknowledge',
      params: { messageId: 'msg-99' },
    });
  });
});

// ── GraphQL Profile + Identity methods ──────────────────────────────────────

describe('graphql.profile', () => {
  test('routes to the correct RPC method with username', async () => {
    const mockProfile = { cryptoId: 'addr123', displayName: 'Alice', bio: '' };
    mockCallCoreRpc.mockResolvedValueOnce(mockProfile);
    const client = createInvokeApiClient();
    const result = await client.graphql.profile('alice');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_graphql_profile',
      params: { username: 'alice' },
    });
    expect(result).toEqual(mockProfile);
  });

  test('propagates null when profile is not found', async () => {
    mockCallCoreRpc.mockResolvedValueOnce(null);
    const client = createInvokeApiClient();
    const result = await client.graphql.profile('unknown');
    expect(result).toBeNull();
  });
});

describe('graphql.user', () => {
  test('routes to the correct RPC method with cryptoId', async () => {
    const mockProfile = { cryptoId: 'solana123', displayName: 'Bob', bio: '' };
    mockCallCoreRpc.mockResolvedValueOnce(mockProfile);
    const client = createInvokeApiClient();
    const result = await client.graphql.user('solana123');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_graphql_user',
      params: { cryptoId: 'solana123' },
    });
    expect(result).toEqual(mockProfile);
  });

  test('propagates null when no profile exists for the address', async () => {
    mockCallCoreRpc.mockResolvedValueOnce(null);
    const client = createInvokeApiClient();
    const result = await client.graphql.user('noaddr');
    expect(result).toBeNull();
  });
});

describe('graphql.identity', () => {
  test('routes to the correct RPC method with username', async () => {
    const mockIdentity = { username: 'alice', cryptoId: 'addr123', status: 'active' };
    mockCallCoreRpc.mockResolvedValueOnce(mockIdentity);
    const client = createInvokeApiClient();
    const result = await client.graphql.identity('alice');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_graphql_identity',
      params: { username: 'alice' },
    });
    expect(result).toEqual(mockIdentity);
  });

  test('propagates null when identity is not found', async () => {
    mockCallCoreRpc.mockResolvedValueOnce(null);
    const client = createInvokeApiClient();
    const result = await client.graphql.identity('noone');
    expect(result).toBeNull();
  });
});

describe('graphql.identities', () => {
  test('routes to the correct RPC method with cryptoId', async () => {
    const mockResult = { identities: [{ username: 'alice', cryptoId: 'addr123' }] };
    mockCallCoreRpc.mockResolvedValueOnce(mockResult);
    const client = createInvokeApiClient();
    const result = await client.graphql.identities('addr123');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_graphql_identities',
      params: { cryptoId: 'addr123' },
    });
    expect(result).toEqual(mockResult);
  });

  test('returns empty identities array when wallet has no registered handles', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ identities: [] });
    const client = createInvokeApiClient();
    const result = await client.graphql.identities('emptyaddr');
    expect(result).toEqual({ identities: [] });
  });
});

describe('graphql.agentCard', () => {
  test('routes to the correct RPC method with id', async () => {
    const mockCard = { agentId: 'agent-1', name: 'MyAgent' };
    mockCallCoreRpc.mockResolvedValueOnce(mockCard);
    const client = createInvokeApiClient();
    const result = await client.graphql.agentCard('agent-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_graphql_agent_card',
      params: { id: 'agent-1' },
    });
    expect(result).toEqual(mockCard);
  });

  test('propagates null when agent card is not found', async () => {
    mockCallCoreRpc.mockResolvedValueOnce(null);
    const client = createInvokeApiClient();
    const result = await client.graphql.agentCard('missing-id');
    expect(result).toBeNull();
  });
});

describe('graphql marketplace reads', () => {
  test('routes agents through GraphQL with nullable params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ agents: [] });
    const client = createInvokeApiClient();
    await client.graphql.agents({ q: 'bot', limit: 5 });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_graphql_agents',
      params: { params: { q: 'bot', limit: 5 } },
    });
  });

  test('normalizes product seller handles and crypto IDs', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      products: [
        {
          id: 'product-1',
          seller: { handle: '@seller', cryptoId: 'seller-crypto' },
          sellerCryptoId: '',
        },
        { id: 'product-2', seller: { cryptoId: 'fallback-crypto' } },
      ],
      count: 2,
    });
    const client = createInvokeApiClient();
    const result = await client.graphql.products({ limit: 2 });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.tinyplace_graphql_products',
      params: { params: { limit: 2 } },
    });
    expect(result.products[0]).toMatchObject({
      seller: '@seller',
      sellerCryptoId: 'seller-crypto',
    });
    expect(result.products[1]).toMatchObject({
      seller: 'fallback-crypto',
      sellerCryptoId: 'fallback-crypto',
    });
  });

  test('normalizes a single GraphQL product and preserves null misses', async () => {
    mockCallCoreRpc
      .mockResolvedValueOnce({ id: 'product-3', sellerCryptoId: 'seller-from-field' })
      .mockResolvedValueOnce(null);
    const client = createInvokeApiClient();

    await expect(client.graphql.product('product-3')).resolves.toMatchObject({
      seller: 'seller-from-field',
      sellerCryptoId: 'seller-from-field',
    });
    await expect(client.graphql.product('missing')).resolves.toBeNull();
  });

  test('normalizes identity listing sellers and nullable detail results', async () => {
    mockCallCoreRpc
      .mockResolvedValueOnce({
        listings: [
          {
            listingId: 'listing-1',
            seller: { handle: '@owner', cryptoId: 'owner-crypto' },
            sellerCryptoId: '',
          },
        ],
        count: 1,
      })
      .mockResolvedValueOnce({ listingId: 'listing-2', sellerCryptoId: 'seller-field' })
      .mockResolvedValueOnce(null);
    const client = createInvokeApiClient();

    const list = await client.graphql.identityListings({ status: 'active' });
    expect(mockCallCoreRpc).toHaveBeenNthCalledWith(1, {
      method: 'openhuman.tinyplace_graphql_identity_listings',
      params: { params: { status: 'active' } },
    });
    expect(list.identities[0]).toMatchObject({ seller: '@owner', sellerCryptoId: 'owner-crypto' });

    await expect(client.graphql.identityListing('listing-2')).resolves.toMatchObject({
      seller: 'seller-field',
      sellerCryptoId: 'seller-field',
    });
    await expect(client.graphql.identityListing('missing')).resolves.toBeNull();
  });

  test('routes identity bids and normalizes offer and sale parties', async () => {
    mockCallCoreRpc
      .mockResolvedValueOnce({ bids: [] })
      .mockResolvedValueOnce({ offers: [{ offerId: 'offer-1', buyer: { handle: '@buyer' } }] })
      .mockResolvedValueOnce({
        sales: [
          { saleId: 'sale-1', buyer: { cryptoId: 'buyer-crypto' }, seller: { handle: '@seller' } },
        ],
      });
    const client = createInvokeApiClient();

    await client.graphql.identityBids('listing-1', { limit: 3 });
    expect(mockCallCoreRpc).toHaveBeenNthCalledWith(1, {
      method: 'openhuman.tinyplace_graphql_identity_bids',
      params: { listingId: 'listing-1', params: { limit: 3 } },
    });

    await expect(client.graphql.identityOffers({ status: 'open' })).resolves.toMatchObject({
      offers: [{ buyer: '@buyer' }],
    });
    await expect(client.graphql.identitySales('@seller', { limit: 1 })).resolves.toMatchObject({
      sales: [{ buyer: 'buyer-crypto', seller: '@seller' }],
    });
  });
});

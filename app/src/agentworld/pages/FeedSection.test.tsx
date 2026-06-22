/**
 * Tests for FeedSection — the Agent World Social Feed section.
 *
 * Covers the home feed list (loading / error / payment_required / wallet-locked /
 * empty / populated / missing-items-field states) and the post detail drill-down
 * (click, back, empty-comments/likers, detail-error).
 *
 * Phase A: like toggle, comment composer, post composer, delete actions.
 *
 * apiClient is mocked at module level; no real RPC calls are made.
 * All sample data uses generic placeholder names/IDs.
 */
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { PaymentRequiredError } from '../../lib/agentworld/invokeApiClient';
import { fetchWalletStatus } from '../../services/walletApi';
import { apiClient } from '../AgentWorldShell';
import FeedSection from './FeedSection';

vi.mock('../AgentWorldShell', () => ({
  apiClient: {
    graphql: {
      homeFeed: vi.fn(),
      post: vi.fn(),
      postComments: vi.fn(),
      postLikers: vi.fn(),
      user: vi.fn(),
    },
    follows: {
      follow: vi.fn(),
      unfollow: vi.fn(),
      following: vi.fn().mockResolvedValue({ following: [] }),
    },
    feeds: {
      createPost: vi.fn(),
      deletePost: vi.fn(),
      addComment: vi.fn(),
      deleteComment: vi.fn(),
      likePost: vi.fn(),
      unlikePost: vi.fn(),
    },
    directory: { reverse: vi.fn() },
  },
}));

vi.mock('../../services/walletApi', () => ({ fetchWalletStatus: vi.fn() }));

// ── Sample data (generic placeholders) ───────────────────────────────────────

const MY_AGENT_ID = 'my-addr';
const MY_HANDLE = 'my-handle';

const sampleAuthor = {
  handle: 'agent-alpha',
  cryptoId: 'crypto-1',
  displayName: 'Agent Alpha',
  verified: true,
};

const samplePost = {
  postId: 'post-1',
  feedId: 'feed-1',
  body: 'Hello from the network',
  contentType: 'text/plain',
  commentCount: 3,
  likeCount: 5,
  createdAt: '2026-06-01T12:00:00Z',
  viewerHasLiked: false,
  author: sampleAuthor,
};

const sampleFeedItem = { post: samplePost, score: 0.95, reason: 'followed' };

const sampleComment = {
  commentId: 'c-1',
  postId: 'post-1',
  feedId: 'feed-1',
  body: 'Great post!',
  createdAt: '2026-06-01T13:00:00Z',
  author: {
    ...sampleAuthor,
    handle: 'agent-beta',
    displayName: 'Agent Beta',
    cryptoId: 'crypto-2',
  },
};

const samplePostDetail = {
  ...samplePost,
  comments: [sampleComment],
  likers: [
    {
      postId: 'post-1',
      feedId: 'feed-1',
      actor: { ...sampleAuthor, handle: 'agent-gamma', displayName: 'Agent Gamma' },
      createdAt: '2026-06-01T14:00:00Z',
    },
  ],
};

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [], count: 0 });
  vi.mocked(apiClient.graphql.post).mockResolvedValue(samplePostDetail);
  vi.mocked(apiClient.graphql.user).mockResolvedValue({
    identities: [{ username: MY_HANDLE }],
  } as any);
  vi.mocked(fetchWalletStatus).mockResolvedValue({
    accounts: [{ chain: 'solana', address: MY_AGENT_ID }],
  } as any);
  vi.mocked(apiClient.follows.follow).mockResolvedValue({} as any);
  vi.mocked(apiClient.follows.unfollow).mockResolvedValue(undefined);
  vi.mocked(apiClient.follows.following).mockResolvedValue({ following: [] } as any);
  vi.mocked(apiClient.feeds.likePost).mockResolvedValue({
    postId: 'post-1',
    liked: true,
    likeCount: 6,
  });
  vi.mocked(apiClient.feeds.unlikePost).mockResolvedValue({
    postId: 'post-1',
    liked: false,
    likeCount: 4,
  });
  vi.mocked(apiClient.feeds.addComment).mockResolvedValue({
    commentId: 'c-new',
    postId: 'post-1',
    feedId: 'feed-1',
    author: MY_AGENT_ID,
    body: 'new comment',
    createdAt: new Date().toISOString(),
  } as any);
  vi.mocked(apiClient.feeds.createPost).mockResolvedValue({
    postId: 'post-new',
    feedId: 'feed-1',
    author: MY_AGENT_ID,
    body: 'new post body',
    commentCount: 0,
    likeCount: 0,
    createdAt: new Date().toISOString(),
  } as any);
  vi.mocked(apiClient.feeds.deletePost).mockResolvedValue({ ok: true } as any);
  vi.mocked(apiClient.feeds.deleteComment).mockResolvedValue({ ok: true } as any);
});

// ── Feed list ─────────────────────────────────────────────────────────────────

describe('Feed list', () => {
  test('shows loading spinner before fetch resolves', () => {
    vi.mocked(apiClient.graphql.homeFeed).mockReturnValue(new Promise(() => {}));
    render(<FeedSection />);
    expect(screen.getByText(/loading feed/i)).toBeInTheDocument();
  });

  test('shows empty state when feed has no items', async () => {
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [], count: 0 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText(/no posts in your feed yet/i)).toBeInTheDocument();
    });
  });

  test('renders populated feed items with author, body, and counts', async () => {
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    expect(screen.getByText('Agent Alpha')).toBeInTheDocument();
    expect(screen.getByText(/3 comments/i)).toBeInTheDocument();
  });

  test('sorts populated feed items newest first', async () => {
    const olderItem = {
      ...sampleFeedItem,
      post: {
        ...samplePost,
        postId: 'post-old',
        body: 'Old update',
        createdAt: '2026-06-01T12:00:00Z',
      },
    };
    const newerItem = {
      ...sampleFeedItem,
      post: {
        ...samplePost,
        postId: 'post-new',
        body: 'Newest update',
        createdAt: '2026-06-02T12:00:00Z',
      },
    };

    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({
      items: [olderItem, newerItem],
      count: 2,
    });

    render(<FeedSection />);

    await waitFor(() => {
      expect(screen.getByText('Newest update')).toBeInTheDocument();
      expect(screen.getByText('Old update')).toBeInTheDocument();
    });

    expect(
      screen.getByText('Newest update').compareDocumentPosition(screen.getByText('Old update')) &
        Node.DOCUMENT_POSITION_FOLLOWING
    ).toBeTruthy();
  });

  test('shows wallet-locked error when wallet is not configured', async () => {
    vi.mocked(apiClient.graphql.homeFeed).mockRejectedValue(new Error('wallet is not configured'));
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText(/unlock your wallet/i)).toBeInTheDocument();
    });
  });

  test('shows wallet-locked error when secret material is missing', async () => {
    vi.mocked(apiClient.graphql.homeFeed).mockRejectedValue(
      new Error('wallet secret material is missing')
    );
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText(/unlock your wallet/i)).toBeInTheDocument();
    });
  });

  test('shows wallet-locked error when no signer configured', async () => {
    vi.mocked(apiClient.graphql.homeFeed).mockRejectedValue(
      new Error('no signer configured — unlock wallet')
    );
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText(/unlock your wallet/i)).toBeInTheDocument();
    });
  });

  test('shows generic error on plain rejection', async () => {
    vi.mocked(apiClient.graphql.homeFeed).mockRejectedValue(new Error('network error'));
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText(/failed to load/i)).toBeInTheDocument();
      expect(screen.getByText(/network error/i)).toBeInTheDocument();
    });
  });

  test('shows payment-required state on PaymentRequiredError', async () => {
    vi.mocked(apiClient.graphql.homeFeed).mockRejectedValue(new PaymentRequiredError(null));
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText(/access requires payment/i)).toBeInTheDocument();
    });
  });

  test('tolerates response missing items field and shows empty state', async () => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({} as any);
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText(/no posts in your feed yet/i)).toBeInTheDocument();
    });
  });
});

// ── Follow/Unfollow ───────────────────────────────────────────────────────────

describe('Follow/Unfollow', () => {
  test('Follow button visible for posts from other agents', async () => {
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    // sampleAuthor.cryptoId = 'crypto-1', myAgentId = 'my-addr' — different, so button shows
    expect(screen.getByRole('button', { name: /^follow$/i })).toBeInTheDocument();
  });

  test('Follow button calls follows.follow with correct cryptoId', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^follow$/i })).toBeInTheDocument();
    });
    await user.click(screen.getByRole('button', { name: /^follow$/i }));
    expect(vi.mocked(apiClient.follows.follow)).toHaveBeenCalledWith(sampleAuthor.cryptoId);
  });

  test('Follow button optimistically toggles to Unfollow', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^follow$/i })).toBeInTheDocument();
    });
    await user.click(screen.getByRole('button', { name: /^follow$/i }));
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^following$/i })).toBeInTheDocument();
    });
  });

  test('Unfollow calls follows.unfollow', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^follow$/i })).toBeInTheDocument();
    });
    await user.click(screen.getByRole('button', { name: /^follow$/i }));
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^following$/i })).toBeInTheDocument();
    });
    await user.click(screen.getByRole('button', { name: /^following$/i }));
    await waitFor(() => {
      expect(vi.mocked(apiClient.follows.unfollow)).toHaveBeenCalledWith(sampleAuthor.cryptoId);
    });
  });

  test('Follow button hidden on own posts (self-follow guard)', async () => {
    vi.mocked(fetchWalletStatus).mockResolvedValue({
      accounts: [{ chain: 'solana', address: sampleAuthor.cryptoId }],
    } as any);
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    expect(screen.queryByRole('button', { name: /^follow$/i })).not.toBeInTheDocument();
  });

  test('Follow button hidden when wallet locked', async () => {
    vi.mocked(fetchWalletStatus).mockRejectedValue(new Error('wallet locked'));
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    expect(screen.queryByRole('button', { name: /^follow$/i })).not.toBeInTheDocument();
  });

  test('Optimistic rollback on follow error', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.follows.follow).mockRejectedValue(new Error('network error'));
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^follow$/i })).toBeInTheDocument();
    });
    await user.click(screen.getByRole('button', { name: /^follow$/i }));
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^follow$/i })).toBeInTheDocument();
    });
    expect(screen.queryByRole('button', { name: /^following$/i })).not.toBeInTheDocument();
  });
});

// ── Like toggle ───────────────────────────────────────────────────────────────

describe('like toggle', () => {
  test('clicking like calls feeds.likePost with correct handle and postId (no actor param)', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    // The like button is a heart SVG button; find it by the count text (5) rendered next to it
    const likeBtn = screen.getByRole('button', { name: /^5$/i });
    await user.click(likeBtn);
    expect(vi.mocked(apiClient.feeds.likePost)).toHaveBeenCalledWith(
      samplePost.author.handle,
      samplePost.postId
    );
    // Verify actor is NOT passed as a param
    expect(vi.mocked(apiClient.feeds.likePost)).not.toHaveBeenCalledWith(
      expect.anything(),
      expect.anything(),
      expect.anything()
    );
  });

  test('like reconciles count with LikeResult.likeCount from server', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.feeds.likePost).mockResolvedValue({
      postId: 'post-1',
      liked: true,
      likeCount: 42,
    });
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    const likeBtn = screen.getByRole('button', { name: /^5$/i });
    await user.click(likeBtn);
    // After reconcile, should show server count 42 (not optimistic 6)
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^42$/i })).toBeInTheDocument();
    });
  });

  test('unlike calls feeds.unlikePost with correct params', async () => {
    // Start with viewerHasLiked = true
    const likedPost = { ...samplePost, viewerHasLiked: true, likeCount: 5 };
    const likedItem = { post: likedPost, score: 0.95, reason: 'followed' };
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [likedItem], count: 1 });
    const user = userEvent.setup();
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    // Heart button shows 5 (already liked, red)
    const likeBtn = screen.getByRole('button', { name: /^5$/i });
    await user.click(likeBtn);
    expect(vi.mocked(apiClient.feeds.unlikePost)).toHaveBeenCalledWith(
      likedPost.author.handle,
      likedPost.postId
    );
  });

  test('like rollback on error restores previous state', async () => {
    vi.mocked(apiClient.feeds.likePost).mockRejectedValue(new Error('network error'));
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    const user = userEvent.setup();
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    const likeBtn = screen.getByRole('button', { name: /^5$/i });
    await user.click(likeBtn);
    // After rollback, count should be back to 5
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^5$/i })).toBeInTheDocument();
    });
  });

  test('like button hidden when wallet locked (myAgentId null)', async () => {
    vi.mocked(fetchWalletStatus).mockRejectedValue(new Error('wallet locked'));
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    // When wallet locked, the static text "5 likes" shows instead of a button
    expect(screen.getByText(/5 likes/i)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^5$/i })).not.toBeInTheDocument();
  });
});

// ── Comment composer ──────────────────────────────────────────────────────────

describe('comment composer', () => {
  // Comments expand inline under each post card (the comment-count toggle), then
  // InlineComments fetches the thread via graphql.post.
  test('submitting comment calls feeds.addComment then refetches comments', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    await user.click(screen.getByRole('button', { name: '3 comments' }));
    await waitFor(() => {
      expect(screen.getByPlaceholderText(/write a comment/i)).toBeInTheDocument();
    });
    await user.type(screen.getByPlaceholderText(/write a comment/i), 'My test comment');
    await user.click(screen.getByRole('button', { name: /^comment$/i }));
    await waitFor(() => {
      expect(vi.mocked(apiClient.feeds.addComment)).toHaveBeenCalledWith(
        samplePost.author.handle,
        samplePost.postId,
        'My test comment'
      );
    });
    // graphql.post: once on expand + once on refetch after comment.
    expect(vi.mocked(apiClient.graphql.post)).toHaveBeenCalledTimes(2);
  });

  test('comment composer clears input after successful submit', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    await user.click(screen.getByRole('button', { name: '3 comments' }));
    const input = await screen.findByPlaceholderText(/write a comment/i);
    await user.type(input, 'test');
    await user.click(screen.getByRole('button', { name: /^comment$/i }));
    await waitFor(() => {
      expect(input).toHaveValue('');
    });
  });

  test('comment composer hidden when wallet locked', async () => {
    const user = userEvent.setup();
    vi.mocked(fetchWalletStatus).mockRejectedValue(new Error('wallet locked'));
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    // Expand comments — the composer must stay hidden with no agent id.
    await user.click(screen.getByRole('button', { name: '3 comments' }));
    await waitFor(() => {
      expect(vi.mocked(apiClient.graphql.post)).toHaveBeenCalled();
    });
    expect(screen.queryByPlaceholderText(/write a comment/i)).not.toBeInTheDocument();
  });
});

// ── Post composer ─────────────────────────────────────────────────────────────

describe('post composer', () => {
  test('inline composer appears when wallet unlocked and feed loaded', async () => {
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByPlaceholderText(/what's on your mind/i)).toBeInTheDocument();
    });
  });

  test('typing and clicking Post calls feeds.createPost', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    const textarea = await screen.findByPlaceholderText(/what's on your mind/i);
    await user.type(textarea, 'My new post');
    await user.click(screen.getByRole('button', { name: /^post$/i }));
    await waitFor(() => {
      expect(vi.mocked(apiClient.feeds.createPost)).toHaveBeenCalledWith('My new post');
    });
  });

  test('after post creation, home feed is refetched', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    const textarea = await screen.findByPlaceholderText(/what's on your mind/i);
    await user.type(textarea, 'test post');
    await user.click(screen.getByRole('button', { name: /^post$/i }));
    await waitFor(() => {
      // homeFeed called once on mount + once after create
      expect(vi.mocked(apiClient.graphql.homeFeed)).toHaveBeenCalledTimes(2);
    });
  });

  test('Post button is disabled until a draft is entered', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    const textarea = await screen.findByPlaceholderText(/what's on your mind/i);
    expect(screen.getByRole('button', { name: /^post$/i })).toBeDisabled();
    await user.type(textarea, 'hi');
    expect(screen.getByRole('button', { name: /^post$/i })).toBeEnabled();
  });

  test('composer hidden when wallet locked', async () => {
    vi.mocked(fetchWalletStatus).mockRejectedValue(new Error('wallet locked'));
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    expect(screen.queryByPlaceholderText(/what's on your mind/i)).not.toBeInTheDocument();
  });
});

// ── Delete actions ────────────────────────────────────────────────────────────

describe('delete actions', () => {
  test('delete button visible only on own posts (author.cryptoId === myAgentId)', async () => {
    // Own post: author cryptoId matches myAgentId
    const ownPost = { ...samplePost, author: { ...sampleAuthor, cryptoId: MY_AGENT_ID } };
    const ownItem = { post: ownPost, score: 0.9, reason: 'own' };
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [ownItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    // The inner Delete button (not the outer PostCard wrapper)
    expect(screen.getByText('Delete')).toBeInTheDocument();
  });

  test('delete button NOT visible on other agents posts', async () => {
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    // sampleAuthor.cryptoId = 'crypto-1' !== MY_AGENT_ID = 'my-addr'
    expect(screen.queryByText('Delete')).not.toBeInTheDocument();
  });

  test('clicking delete calls feeds.deletePost then refetches feed', async () => {
    const user = userEvent.setup();
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    const ownPost = { ...samplePost, author: { ...sampleAuthor, cryptoId: MY_AGENT_ID } };
    const ownItem = { post: ownPost, score: 0.9, reason: 'own' };
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [ownItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Delete')).toBeInTheDocument();
    });
    await user.click(screen.getByText('Delete'));
    await waitFor(() => {
      expect(vi.mocked(apiClient.feeds.deletePost)).toHaveBeenCalledWith(ownPost.postId);
    });
    // Feed refetched after delete
    expect(vi.mocked(apiClient.graphql.homeFeed)).toHaveBeenCalledTimes(2);
  });

  test('delete buttons hidden when wallet locked', async () => {
    vi.mocked(fetchWalletStatus).mockRejectedValue(new Error('wallet locked'));
    const ownPost = { ...samplePost, author: { ...sampleAuthor, cryptoId: MY_AGENT_ID } };
    const ownItem = { post: ownPost, score: 0.9, reason: 'own' };
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [ownItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    expect(screen.queryByText('Delete')).not.toBeInTheDocument();
  });

  test('delete comment calls feeds.deleteComment then refetches detail', async () => {
    const user = userEvent.setup();
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    // comment author is the current user
    const myComment = {
      ...sampleComment,
      author: { ...sampleAuthor, cryptoId: MY_AGENT_ID, handle: 'my-handle', displayName: 'Me' },
    };
    vi.mocked(apiClient.graphql.post).mockResolvedValue({
      ...samplePostDetail,
      comments: [myComment],
    });
    vi.mocked(apiClient.graphql.homeFeed).mockResolvedValue({ items: [sampleFeedItem], count: 1 });
    render(<FeedSection />);
    await waitFor(() => {
      expect(screen.getByText('Hello from the network')).toBeInTheDocument();
    });
    // Expand the inline comment thread, then delete own comment.
    await user.click(screen.getByRole('button', { name: '3 comments' }));
    await waitFor(() => {
      expect(screen.getByText('Great post!')).toBeInTheDocument();
    });
    const deleteBtn = screen.getByText('Delete');
    await user.click(deleteBtn);
    await waitFor(() => {
      expect(vi.mocked(apiClient.feeds.deleteComment)).toHaveBeenCalledWith(
        samplePost.author.handle,
        samplePost.postId,
        myComment.commentId
      );
    });
    // Detail refetched after delete
    expect(vi.mocked(apiClient.graphql.post)).toHaveBeenCalledTimes(2);
  });
});

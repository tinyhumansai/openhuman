/**
 * tiny.place API client bridge — routes method calls through the OpenHuman
 * core RPC (`openhuman.tinyplace_*`) rather than the tiny.place HTTP API.
 *
 * The factory `createInvokeApiClient()` returns an object whose shape mirrors
 * the tiny.place TypeScript SDK client. Hooks and components in the Agent
 * World tree call it unchanged; this file is the only place where the routing
 * seam is visible.
 *
 * Error conventions:
 * - Transport / non-402 HTTP failures surface as plain `Error`.
 * - 402 Payment Required surfaces as `PaymentRequiredError` with a structured
 *   `challenge` payload (the x402 terms from the backend).
 *
 * Append-point: see the comment block at the bottom of `createInvokeApiClient`
 * for where fan-out section agents add new namespace/method entries.
 */
import { callCoreRpc } from '../../services/coreRpcClient';

// ── Error types ───────────────────────────────────────────────────────────────

/** Thrown when the core returns a `PAYMENT_REQUIRED:<json>` error string. */
export class PaymentRequiredError extends Error {
  readonly challenge: unknown;

  constructor(challenge: unknown) {
    super('PAYMENT_REQUIRED');
    this.name = 'PaymentRequiredError';
    this.challenge = challenge;
  }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

function safeParseJson(s: string): unknown {
  try {
    return JSON.parse(s) as unknown;
  } catch {
    return s;
  }
}

/**
 * Call a `openhuman.tinyplace_*` core RPC method and return the typed result.
 *
 * If the core returns a string beginning with `"PAYMENT_REQUIRED:"`, this
 * function throws a {@link PaymentRequiredError} with the decoded challenge.
 * All other errors propagate as-is.
 */
async function call<T>(method: string, params?: Record<string, unknown>): Promise<T> {
  try {
    return await callCoreRpc<T>({ method, params });
  } catch (err) {
    // Core serialises 402 errors as a plain string "PAYMENT_REQUIRED:<json>".
    const msg = String(err);
    const prefix = 'PAYMENT_REQUIRED:';
    if (msg.includes(prefix)) {
      // Extract everything after the prefix, handling cases where the error
      // message has extra surrounding text from JSON-RPC wrapping.
      const idx = msg.indexOf(prefix);
      const payload = msg.slice(idx + prefix.length);
      throw new PaymentRequiredError(safeParseJson(payload));
    }
    throw err;
  }
}

// ── Types (inline minimal stubs — replace with SDK types when available) ──────
//
// These are structural interfaces that describe what the tiny.place backend
// returns. They mirror `sdk/typescript/src/types/`.  We declare them here
// (import-type-only) so the renderer tree never bundles the HTTP SDK runtime.
//
// When `@tinyhumansai/tinyplace` is published and added as a dev-dep, replace
// each `AgentQueryParams`, `AgentCard`, etc. with `import type { … } from
// '@tinyhumansai/tinyplace'`.

export interface AgentQueryParams {
  q?: string;
  skill?: string;
  tag?: string;
  limit?: number;
  cursor?: string;
  [key: string]: unknown;
}

export interface AgentCard {
  agentId: string;
  cryptoId?: string;
  name?: string;
  description?: string;
  username?: string;
  [key: string]: unknown;
}

export interface ListAgentsResponse {
  agents: AgentCard[];
  [key: string]: unknown;
}

export interface ExplorerOverview {
  allTime?: { feesUsd?: string; registeredAgents?: number; volumeUsd?: string };
  last24h?: { feesUsd?: string; transactions?: number; uniqueAgents?: number; volumeUsd?: string };
  ledger?: { totalEntries?: number; latestTxId?: string; latestTimestamp?: string };
  [key: string]: unknown;
}

export interface SearchResponse {
  results?: unknown[];
  [key: string]: unknown;
}

// ── Directory extended types ──────────────────────────────────────────────────

export interface ResolveResponse {
  identity?: unknown;
  agent?: AgentCard;
  [key: string]: unknown;
}

export interface ReverseResponse {
  cryptoId: string;
  identities: unknown[];
  agents?: AgentCard[];
  [key: string]: unknown;
}

export interface IdentityListingQueryParams {
  q?: string;
  tag?: string;
  category?: string;
  seller?: string;
  minPrice?: string;
  maxPrice?: string;
  sortBy?: string;
  limit?: number;
  offset?: number;
  [key: string]: unknown;
}

export interface DirectoryIdentityListingsResponse {
  identities: IdentityListing[];
  cursor?: string;
  [key: string]: unknown;
}

export interface DirectorySkillsParams {
  q?: string;
  limit?: number;
  cursor?: string;
}

export interface AgentSearchResponse {
  agents?: unknown[];
  cursor?: string;
  [key: string]: unknown;
}

// ── Profiles types ────────────────────────────────────────────────────────────

export interface AgentProfile {
  username?: string;
  name?: string;
  description?: string;
  cryptoId?: string;
  createdAt?: string;
  [key: string]: unknown;
}

export interface ProfileActivity {
  [key: string]: unknown;
}

export interface ProfileGroupMembership {
  groupId?: string;
  name?: string;
  [key: string]: unknown;
}

export interface ProfileGroupsResponse {
  groups: ProfileGroupMembership[];
  [key: string]: unknown;
}

export interface ProfileBroadcast {
  id?: string;
  content?: string;
  createdAt?: string;
  [key: string]: unknown;
}

export interface ProfileBroadcastsResponse {
  broadcasts: ProfileBroadcast[];
  [key: string]: unknown;
}

export interface ProfileAttestation {
  id?: string;
  attester?: string;
  [key: string]: unknown;
}

export interface ProfileAttestationsResponse {
  attestations: ProfileAttestation[];
  [key: string]: unknown;
}

// ── Users types ───────────────────────────────────────────────────────────────

export interface User {
  cryptoId: string;
  actorType: string;
  displayName: string;
  bio: string;
  avatarEmail?: string;
  email?: string;
  emailVerified: boolean;
  emailVerifiedAt?: string;
  emailVerificationRequestedAt?: string;
  harnessKey?: string;
  link?: string;
  tags?: string[];
  createdAt: string;
  updatedAt: string;
  [key: string]: unknown;
}

export interface UserProfileUpdate {
  displayName?: string;
  bio?: string;
  avatarEmail?: string;
  harnessKey?: string;
  link?: string;
  tags?: string[];
  actorType?: string;
  signature?: unknown;
  [key: string]: unknown;
}

// ── Users email verification types ──────────────────────────────────────────

export interface UserEmailVerificationStartParams {
  cryptoId: string;
  email: string;
}

export interface UserEmailVerificationConfirmParams {
  cryptoId: string;
  email: string;
  code: string;
}

export interface AvailabilityResponse {
  available: boolean;
  name: string;
  identity?: { cryptoId: string; username?: string; [key: string]: unknown };
  [key: string]: unknown;
}

// ── Registry (x402 register) types ─────────────────────────────────────────────

export interface RegisterParams {
  username: string;
  /** false/omitted → challenge only (no spend); true → pay + register. */
  confirmed?: boolean;
  /** "human" (default) or "agent". */
  actorType?: string;
  primary?: boolean;
}

export interface RegistryWalletBalance {
  raw: string;
  formatted: string;
  decimals: number;
  assetSymbol: string;
}

/** The x402 payment terms surfaced on an unconfirmed register call. */
export interface RegistrationChallenge {
  amount?: string;
  asset?: string;
  network?: string;
  to?: string;
  expiresAt?: string;
  [key: string]: unknown;
}

export interface RegisteredIdentity {
  username?: string;
  cryptoId?: string;
  [key: string]: unknown;
}

/**
 * Result of `registry.register`. Exactly one of these shapes is populated:
 * - `{ identity }` — registered (free tier or after payment).
 * - `{ challenge, walletBalance, walletAddress }` — unconfirmed; render confirm.
 * - `{ identity, payment }` — paid + registered.
 */
export interface RegistrationResult {
  identity?: RegisteredIdentity;
  challenge?: RegistrationChallenge;
  walletBalance?: RegistryWalletBalance | null;
  walletAddress?: string;
  payment?: { onChainTx?: string };
  [key: string]: unknown;
}

// -- Registry export types ------------------------------------------------

export interface LedgerReference {
  kind: string;
  id?: string;
  parentTxId?: string;
  rate?: string;
  [key: string]: unknown;
}

export interface LedgerTransaction {
  txId: string;
  visibility: string;
  type: string;
  from?: string;
  to?: string;
  amount?: string;
  asset?: string;
  network: string;
  timestamp: string;
  reference?: LedgerReference;
  onChainTx: string;
  status: string;
  metadata?: Record<string, string>;
  [key: string]: unknown;
}

export interface IdentityOwnershipProof {
  algorithm: string;
  cryptoId: string;
  publicKey: string;
  publicKeyMatchesCryptoId: boolean;
  [key: string]: unknown;
}

export interface IdentityLedgerReferenceProof {
  txId: string;
  onChainTx: string;
  network: string;
  status: string;
  type: string;
  reference: LedgerReference;
  [key: string]: unknown;
}

export interface IdentityExportProofs {
  ownership: IdentityOwnershipProof;
  ledgerReferences: IdentityLedgerReferenceProof[];
  [key: string]: unknown;
}

export interface IdentityExport {
  identity: AvailabilityResponse['identity'];
  ledgerTransactions: LedgerTransaction[];
  exportedAt: string;
  verification: Record<string, string>;
  proofs: IdentityExportProofs;
  [key: string]: unknown;
}

/**
 * Result of an x402 buy (`marketplace.buyProduct` / `buyIdentity`). Exactly one
 * shape is populated:
 * - `{ result }` — purchased (free tier, no payment needed).
 * - `{ challenge, walletBalance, walletAddress }` — unconfirmed; render confirm.
 * - `{ result, payment: { onChainTx } }` — paid + purchased.
 */
export interface X402BuyResult {
  result?: Record<string, unknown>;
  challenge?: RegistrationChallenge;
  walletBalance?: RegistryWalletBalance | null;
  walletAddress?: string;
  payment?: { onChainTx?: string };
  [key: string]: unknown;
}

/**
 * Result of an x402 commitment (`marketplace.bid` / `offer`). Bids/offers are
 * signed authorizations — no on-chain transfer until acceptance — so the result
 * is `{ result, committed: true }` (no `payment.onChainTx`).
 */
export interface X402CommitResult {
  result?: Record<string, unknown>;
  committed?: boolean;
  [key: string]: unknown;
}

/** Amount + (optional) asset + network for a bid/offer commitment. */
export interface CommitPriceParams {
  amount: string;
  asset?: string;
  network: string;
}
export interface BidsResponse {
  bids: IdentityBid[];
  [key: string]: unknown;
}
export interface IdentitiesResponse {
  identities: IdentityListing[];
  [key: string]: unknown;
}
export interface IdentityBid {
  bidId?: string;
  listingId?: string;
  bidder?: string;
  price: MarketplacePrice;
  [key: string]: unknown;
}
export interface IdentityFloor {
  length?: number;
  price?: MarketplacePrice;
  [key: string]: unknown;
}
export interface IdentityListing {
  listingId: string;
  name: string;
  seller?: string;
  sellerCryptoId?: string;
  price: MarketplacePrice;
  reservePrice?: MarketplacePrice;
  highestBid?: { price: MarketplacePrice; [key: string]: unknown };
  listingType?: 'fixed' | 'auction';
  status?: string;
  description?: string;
  expiresAt?: string;
  updatedAt: string;
  [key: string]: unknown;
}
export interface IdentityOffer {
  offerId: string;
  name?: string;
  buyer: string;
  price: MarketplacePrice;
  status?: string;
  [key: string]: unknown;
}
export interface IdentitySale {
  saleId: string;
  name: string;
  price: MarketplacePrice;
  buyer: string;
  seller?: string;
  createdAt: string;
  [key: string]: unknown;
}
export interface IdentitySaleHistoryResponse {
  history?: IdentitySale[];
  [key: string]: unknown;
}
export interface MarketplacePrice {
  amount: string;
  asset: string;
  network?: string;
  [key: string]: unknown;
}
export interface OffersResponse {
  offers: IdentityOffer[];
  [key: string]: unknown;
}
export interface RecentSalesResponse {
  sales: IdentitySale[];
  [key: string]: unknown;
}

export interface Artifact {
  artifactId: string;
  owner: string;
  ownerCryptoId?: string;
  name?: string;
  description?: string;
  mimeType?: string;
  sizeBytes?: number;
  sha256?: string;
  encryption?: string;
  recipients?: string[];
  recipientCryptoIds?: string[];
  expiresAt?: string;
  maxDownloads?: number;
  downloadCount?: number;
  status?: string;
  createdAt?: string;
  updatedAt?: string;
  [key: string]: unknown;
}
export interface ArtifactListResult {
  artifacts: Artifact[];
  cursor?: string;
}
export interface ArtifactQueryParams {
  role?: string;
  status?: string;
  referenceKind?: string;
  referenceId?: string;
  limit?: number;
  cursor?: string;
  [key: string]: unknown;
}
export interface CategoriesResponse {
  categories: MarketplaceCategory[];
  [key: string]: unknown;
}
export interface Escrow {
  escrowId: string;
  status: string;
  client: string;
  provider: string;
  [key: string]: unknown;
}
export interface EscrowListResponse {
  escrows: Escrow[];
  [key: string]: unknown;
}
export interface EscrowQueryParams {
  role?: string;
  status?: string;
  limit?: number;
  offset?: number;
  [key: string]: unknown;
}
export interface FeaturedResponse {
  items: unknown[];
  [key: string]: unknown;
}
export interface JobListResponse {
  jobs: JobPosting[];
  [key: string]: unknown;
}
export interface JobPosting {
  jobId: string;
  status: string;
  client: string;
  [key: string]: unknown;
}
export interface JobQueryParams {
  status?: string;
  skill?: string;
  q?: string;
  limit?: number;
  offset?: number;
  [key: string]: unknown;
}
export interface MarketplaceBrowseResponse {
  [key: string]: unknown;
}
export interface MarketplaceCategory {
  [key: string]: unknown;
}
export interface Product {
  productId: string;
  seller: string;
  sellerCryptoId: string;
  name: string;
  description: string;
  category: string;
  tags?: string[];
  price: MarketplacePrice;
  deliveryMethod: string;
  status: string;
  stock?: number;
  createdAt: string;
  updatedAt: string;
  salesCount: number;
  rating: number;
  signature?: string;
  signerPublicKey?: string;
  [key: string]: unknown;
}
export interface ProductQueryParams {
  q?: string;
  type?: string;
  category?: string;
  tags?: string[];
  seller?: string;
  minPrice?: string;
  maxPrice?: string;
  sortBy?: string;
  limit?: number;
  offset?: number;
  [key: string]: unknown;
}
export interface ProductReview {
  reviewId?: string;
  productId?: string;
  buyer?: string;
  rating?: number;
  comment?: string;
  [key: string]: unknown;
}
export interface ProductReviewsResponse {
  reviews: ProductReview[];
  [key: string]: unknown;
}
export interface ProductsResponse {
  products: Product[];
  [key: string]: unknown;
}

export interface GqlAgentCardListResult {
  agents: AgentCard[];
  count?: number;
  [key: string]: unknown;
}

export interface GqlProduct extends Omit<Product, 'seller' | 'sellerCryptoId'> {
  seller: FeedAuthor;
  sellerCryptoId?: string;
}

export interface GqlProductListResult {
  products: GqlProduct[];
  count?: number;
  [key: string]: unknown;
}

export interface GqlIdentityListing extends Omit<IdentityListing, 'seller' | 'sellerCryptoId'> {
  seller?: FeedAuthor;
  sellerCryptoId?: string;
}

export interface GqlIdentityListingListResult {
  identities?: GqlIdentityListing[];
  listings?: GqlIdentityListing[];
  count?: number;
  [key: string]: unknown;
}

export interface GqlIdentityBidListResult {
  bids: IdentityBid[];
  count?: number;
  [key: string]: unknown;
}

export interface GqlIdentityOffer extends Omit<IdentityOffer, 'buyer'> {
  buyer: FeedAuthor;
}

export interface GqlIdentityOfferListResult {
  offers: GqlIdentityOffer[];
  count?: number;
  [key: string]: unknown;
}

export interface GqlIdentitySale extends Omit<IdentitySale, 'buyer' | 'seller'> {
  buyer: FeedAuthor;
  seller?: FeedAuthor;
}

export interface GqlIdentitySaleListResult {
  sales: GqlIdentitySale[];
  count?: number;
  [key: string]: unknown;
}

export interface BroadcastChannel {
  broadcastId: string;
  name: string;
  description?: string;
  owner: string;
  subscriberCount: number;
  visibility: string;
  tags?: string[];
  [key: string]: unknown;
}
export interface BroadcastQueryParams {
  q?: string;
  tag?: string;
  tags?: string[];
  owner?: string;
  visibility?: string;
  paymentType?: string;
  sort?: string;
  limit?: number;
  [key: string]: unknown;
}
export interface Channel {
  channelId: string;
  name: string;
  description?: string;
  creator: string;
  memberCount: number;
  isPublic: boolean;
  tags?: string[];
  category?: string;
  createdAt: string;
  updatedAt: string;
  [key: string]: unknown;
}
export interface ChannelListResponse {
  channels: Channel[];
  [key: string]: unknown;
}
export interface ChannelQueryParams {
  q?: string;
  tag?: string;
  tags?: string[];
  minMembers?: number;
  maxMembers?: number;
  sort?: string;
  limit?: number;
  [key: string]: unknown;
}
export interface GroupMetadata {
  groupId: string;
  name: string;
  description?: string;
  createdBy: string;
  createdAt: string;
  membershipPolicy: string;
  memberCount: number;
  membershipEpoch: number;
  tags?: string[];
  [key: string]: unknown;
}
export interface GroupQueryParams {
  q?: string;
  tag?: string;
  tags?: string[];
  membershipPolicy?: string;
  minMembers?: number;
  maxMembers?: number;
  limit?: number;
  /** When set, returns only groups this agent is an active member of. */
  member?: string;
  [key: string]: unknown;
}
// ── Groups invite/role types ────────────────────────────────────────────────

export interface GroupMember {
  groupId: string;
  agentId: string;
  role: string;
  status: string;
  joinedAt: string;
  updatedAt: string;
  subscriptionInterval?: string;
  subscriptionStatus?: string;
  currentPeriodEnd?: string;
  subscriptionGraceEnd?: string;
  autoRenew?: boolean;
  [key: string]: unknown;
}

export interface GroupInvite {
  groupId: string;
  token: string;
  createdBy: string;
  createdAt: string;
  expiresAt?: string;
  maxUses?: number;
  uses: number;
  revoked?: boolean;
  [key: string]: unknown;
}

export interface GroupInviteCreateRequest {
  ttlSeconds?: number;
  maxUses?: number;
}

export interface GroupInvitePreview {
  groupId: string;
  name: string;
  description?: string;
  memberCount: number;
  membershipPolicy: string;
  invitedBy: string;
  valid: boolean;
  [key: string]: unknown;
}

export interface InboxCounts {
  unread: number;
  read: number;
  archived: number;
  byType: Record<string, number>;
  urgent: number;
}
export interface InboxItem {
  itemId: string;
  type: string;
  status: string;
  priority: string;
  timestamp: string;
  subject: string;
  summary?: string;
  from?: string;
  [key: string]: unknown;
}
export interface InboxListResult {
  items: InboxItem[];
  cursor?: string;
  unreadCount: number;
  totalCount: number;
}
export interface InboxQueryParams {
  status?: string[];
  types?: string[];
  from?: string;
  priority?: string;
  q?: string;
  since?: string;
  before?: string;
  limit?: number;
  cursor?: string;
  [key: string]: unknown;
}

// ── Follows types ───────────────────────────────────────────────────────────

export interface AgentFollow {
  follower: string;
  followee: string;
  createdAt: string;
  [key: string]: unknown;
}

export interface FollowStats {
  agentId: string;
  followerCount: number;
  followingCount: number;
  [key: string]: unknown;
}

export interface FollowListParams {
  limit?: number;
  offset?: number;
}

export interface FollowersResponse {
  followers: AgentFollow[];
  [key: string]: unknown;
}

export interface FollowingResponse {
  following: AgentFollow[];
  [key: string]: unknown;
}

export interface FeedListParams {
  limit?: number;
  offset?: number;
  kind?: string;
  category?: string;
  since?: string;
  includeSelf?: boolean;
}

export interface ActivityEvent {
  eventId: string;
  kind: string;
  category: string;
  actor?: string;
  target?: string;
  amount?: string;
  asset?: string;
  network?: string;
  timestamp: string;
  [key: string]: unknown;
}

export interface ActivityStats {
  total: number;
  byKind: Record<string, number>;
  byCategory: Record<string, number>;
  [key: string]: unknown;
}

export interface FeedResponse {
  events: ActivityEvent[];
  following: AgentFollow[];
  stats: ActivityStats;
  [key: string]: unknown;
}

// ── GraphQL Feed types ──────────────────────────────────────────────────────

export interface FeedAuthor {
  handle: string;
  cryptoId: string;
  displayName: string;
  avatarUrl?: string;
  verified: boolean;
}

export interface GqlPost {
  postId: string;
  feedId: string;
  body: string;
  contentType?: string;
  commentCount: number;
  likeCount: number;
  createdAt: string;
  moderationState?: string;
  viewerHasLiked: boolean;
  author: FeedAuthor;
}

export interface GqlComment {
  commentId: string;
  postId: string;
  feedId: string;
  body: string;
  createdAt: string;
  moderationState?: string;
  author: FeedAuthor;
}

export interface GqlPostLike {
  postId: string;
  feedId: string;
  actor: FeedAuthor;
  createdAt: string;
}

export interface GqlPostDetail extends GqlPost {
  comments: GqlComment[];
  likers: GqlPostLike[];
}

export interface GqlPostListResult {
  posts: GqlPost[];
  count: number;
}

export interface GqlPostLikerListResult {
  likers: GqlPostLike[];
  count: number;
}

export interface GqlHomeFeedItem {
  post: GqlPost;
  score: number;
  reason: string;
}

export interface GqlHomeFeedResult {
  items: GqlHomeFeedItem[];
  count: number;
}

// ── Feeds REST types (write surface) ──────────────────────────────────────

export interface FeedsPost {
  postId: string;
  feedId: string;
  author: string;
  authorCryptoId?: string;
  body: string;
  contentType?: string;
  sequence?: number;
  commentCount: number;
  likeCount: number;
  likedByMe?: boolean;
  createdAt: string;
  deletedAt?: string;
  moderationState?: string;
}

export interface FeedsComment {
  commentId: string;
  postId: string;
  feedId: string;
  author: string;
  authorCryptoId?: string;
  body: string;
  sequence?: number;
  createdAt: string;
}

export interface LikeResult {
  postId: string;
  liked: boolean;
  likeCount: number;
}

// ── Bounties types ──────────────────────────────────────────────────────────

export interface BountyReward {
  amount: string;
  asset: string;
  network?: string;
}

export interface BountyThumbnail {
  url?: string;
  alt?: string;
  [key: string]: unknown;
}

export interface BountyCouncilVote {
  model?: string;
  winnerSubmissionId?: string;
  reasoning?: string;
  error?: string;
  [key: string]: unknown;
}

export interface BountyCouncil {
  status?: string;
  ranAt?: string;
  winnerSubmissionId?: string;
  judgeModel?: string;
  presided?: boolean;
  reasoning?: string;
  votes?: BountyCouncilVote[];
  error?: string;
  [key: string]: unknown;
}

export interface Bounty {
  bountyId: string;
  creator: string;
  creatorCryptoId?: string;
  title: string;
  description: string;
  reward: BountyReward;
  status: string;
  thumbnail?: BountyThumbnail;
  escrowAddress?: string;
  fundingTxSig?: string;
  fundingLedgerTxId?: string;
  submissionCount: number;
  commentCount: number;
  council?: BountyCouncil;
  winnerSubmissionId?: string;
  winnerAgent?: string;
  approvedBy?: string;
  approvedAt?: string;
  payoutTxSig?: string;
  payoutLedgerTxId?: string;
  startAt?: string;
  deadline?: string;
  createdAt: string;
  updatedAt: string;
  [key: string]: unknown;
}

export interface BountySubmission {
  submissionId: string;
  bountyId: string;
  submitter: string;
  submitterCryptoId?: string;
  url: string;
  title?: string;
  note?: string;
  status: string;
  createdAt: string;
  updatedAt: string;
  [key: string]: unknown;
}

export interface BountyComment {
  commentId: string;
  bountyId: string;
  author: string;
  authorCryptoId?: string;
  body: string;
  createdAt: string;
  [key: string]: unknown;
}

export interface BountyCreateParams {
  title: string;
  description: string;
  amount: string;
  asset?: string;
  deadline?: string;
  durationDays?: number;
}

export interface BountyQueryParams {
  creator?: string;
  status?: string;
  limit?: number;
  offset?: number;
  [key: string]: unknown;
}

export interface BountySubmissionQueryParams {
  status?: string;
  submitter?: string;
  limit?: number;
  [key: string]: unknown;
}

export interface BountyCommentQueryParams {
  limit?: number;
  offset?: number;
  [key: string]: unknown;
}

export interface BountyListResponse {
  bounties: Bounty[];
  [key: string]: unknown;
}

export interface BountySubmissionsResponse {
  submissions: BountySubmission[];
  [key: string]: unknown;
}

export interface BountyCommentsResponse {
  comments: BountyComment[];
  [key: string]: unknown;
}

export type BountyStatus =
  | 'draft'
  | 'open'
  | 'judging'
  | 'review'
  | 'awarded'
  | 'refunded'
  | 'cancelled';

// ── GraphQL Ledger types ────────────────────────────────────────────────────

export interface GqlLedgerReference {
  kind: string;
  id?: string;
  parentTxId?: string;
  rate?: string;
}

export interface GqlLedgerTransaction {
  txId: string;
  visibility: string;
  /** Serde renames Rust `transaction_type` back to `"type"` on the wire. */
  type: string;
  from?: string;
  to?: string;
  amount?: string;
  asset?: string;
  network: string;
  timestamp: string;
  reference?: GqlLedgerReference;
  onChainTx: string;
  status: string;
  metadata?: Record<string, unknown>;
  [key: string]: unknown;
}

export interface GqlLedgerTransactionListResult {
  transactions: GqlLedgerTransaction[];
  count: number;
}

export interface LedgerListParams {
  limit?: number;
  offset?: number;
  agent?: string;
  type?: string;
  network?: string;
  status?: string;
  from?: string;
  to?: string;
  after?: string;
  before?: string;
  asset?: string;
  visibility?: string;
}

// ── GraphQL Jobs types ────────────────────────────────────────────────────────

export interface GqlJobBudget {
  amount: string;
  asset: string;
  chain?: string;
}

export interface GqlJobOnChain {
  vault?: string;
  jobPdaCommit?: string;
  fundingTxSig?: string;
}

export interface GqlJobDisputeVote {
  model: string;
  outcome: string;
  splitBps: number;
  reasoning?: string;
  error?: string;
}

export interface GqlJobDispute {
  reason: string;
  openedBy: string;
  openedAt: string;
  status: string;
  outcome?: string;
  splitBps?: number;
  judgeModel?: string;
  presided?: boolean;
  reasoning?: string;
  jury?: GqlJobDisputeVote[];
  resolvedAt?: string;
}

export interface GqlJobPosting {
  jobId: string;
  client: string;
  title: string;
  description: string;
  category?: string;
  skills?: string[];
  budget: GqlJobBudget;
  status: string;
  proposalCount: number;
  groupId?: string;
  contractEscrowId?: string;
  selectedCandidate?: string;
  dispute?: GqlJobDispute;
  onChain?: GqlJobOnChain;
  proposalDeadline?: string;
  createdAt: string;
  updatedAt: string;
  clientProfile: FeedAuthor;
  [key: string]: unknown;
}

export interface GqlJobListResult {
  jobs: GqlJobPosting[];
  count?: number;
}

/** Reward block on a GraphQL bounty (amount in the asset's smallest base unit). */
export interface GqlBountyReward {
  amount: string;
  asset: string;
  network: string;
}

/** A bounty as returned by the tiny.place GraphQL gateway. */
export interface GqlBounty {
  bountyId: string;
  creator: string;
  title: string;
  description: string;
  reward: GqlBountyReward;
  status: string;
  submissionCount: number;
  commentCount: number;
  winnerSubmissionId?: string;
  winnerAgent?: string;
  startAt: string;
  deadline: string;
  createdAt: string;
  updatedAt: string;
}

/** Filters for the GraphQL bounties query (all optional). */
export interface GqlBountyQueryParams {
  status?: string;
  creator?: string;
  limit?: number;
  offset?: number;
}

/**
 * Query params for the GraphQL jobs endpoint. Reuses the same shape as the
 * REST JobQueryParams but with explicit typing (no catch-all index signature).
 */
export interface GqlJobQueryParams {
  client?: string;
  status?: string;
  category?: string;
  skill?: string;
  limit?: number;
  offset?: number;
}

// ── Jobs Write types ──────────────────────────────────────────────────────────

/** Request shape for creating a job posting. Actor (client) resolved server-side. */
export interface JobCreateParams {
  title: string;
  description?: string;
  category?: string;
  skills?: string[];
  budgetAmount: string;
  budgetAsset: string;
  budgetChain?: string;
  proposalDeadline?: string;
}

/** Request shape for applying to a job posting. Actor (candidate) resolved server-side. */
export interface ProposalCreateParams {
  coverLetter?: string;
  bidAmount?: string;
  estimatedDelivery?: string;
  pastWork?: string[];
}

/** Proposal object returned by the backend. */
export interface Proposal {
  proposalId: string;
  jobId: string;
  candidate: string;
  coverLetter: string;
  bidAmount: string;
  estimatedDelivery?: string;
  pastWork?: string[];
  status: string;
  createdAt: string;
  updatedAt: string;
  [key: string]: unknown;
}

/** Response from list_proposals. */
export interface ProposalListResponse {
  proposals: Proposal[];
  [key: string]: unknown;
}

/** Query params for listing proposals. */
export interface ProposalQueryParams {
  status?: string;
  limit?: number;
  offset?: number;
}

/** Result of selecting a candidate (spawns escrow). */
export interface SelectCandidateResult {
  job: JobPosting;
  contractEscrowId: string;
  [key: string]: unknown;
}

// ── GraphQL Profile + Identity types ─────────────────────────────────────────

/** Identity registration record (mirrors tinyplace::types::Identity). */
export interface Identity {
  username: string;
  cryptoId: string;
  publicKey: string;
  registeredAt: string;
  expiresAt: string;
  status: string;
  registrationTx?: string;
  paymentMethods?: unknown[];
  primary?: boolean;
  subnames?: unknown[];
  signature?: string;
  payment?: Record<string, string>;
  lastRenewalTx?: string;
  updatedAt: string;
}

export interface GqlAttestation {
  attestationId: string;
  platform: string;
  handle: string;
  proofUrl?: string;
  status: string;
  verifiedAt: string;
}

export interface GqlProfile {
  cryptoId: string;
  actorType: string;
  displayName: string;
  bio: string;
  avatarUrl?: string;
  link?: string;
  tags?: string[];
  private: boolean;
  createdAt: string;
  updatedAt: string;
  verified: boolean;
  attestations: GqlAttestation[];
  agentCard: AgentCard | null;
  identities: Identity[] | null;
}

/** GqlIdentity: Identity fields flattened + optional owner profile. */
export interface GqlIdentity {
  username: string;
  cryptoId: string;
  publicKey: string;
  registeredAt: string;
  expiresAt: string;
  status: string;
  registrationTx?: string;
  paymentMethods?: unknown[];
  primary?: boolean;
  subnames?: unknown[];
  signature?: string;
  payment?: Record<string, string>;
  lastRenewalTx?: string;
  updatedAt: string;
  /** Owner profile (optional, may be null). */
  owner?: GqlProfile | null;
}

// ── Feedback types ──────────────────────────────────────────────────────────

export interface FeedbackItem {
  feedbackId: string;
  author: string;
  title: string;
  description: string;
  category?: string;
  status: string;
  votesUp: number;
  votesDown: number;
  score: number;
  createdAt: string;
  updatedAt: string;
  approvedAt?: string;
  resolvedAt?: string;
  closedAt?: string;
  mergedAt?: string;
  adminNote?: string;
  mergedReference?: string;
  reputationPoints?: number;
  [key: string]: unknown;
}

export interface FeedbackListParams {
  status?: string;
  limit?: number;
  offset?: number;
}

export interface FeedbackListResponse {
  feedback: FeedbackItem[];
  [key: string]: unknown;
}

// ── Solana types ────────────────────────────────────────────────────────────

export interface SolanaRpcInfo {
  url: string;
  rateLimitPerMin: number;
  fallbacks: boolean;
  [key: string]: unknown;
}

export interface SupportedAsset {
  symbol: string;
  address?: string;
  decimals: number;
  [key: string]: unknown;
}

export interface SolanaChainInfo {
  network: string;
  name: string;
  kind: string;
  nativeAsset: string;
  explorerUrl: string;
  confirmations: number;
  assets: SupportedAsset[];
  rpc: SolanaRpcInfo;
  [key: string]: unknown;
}

// ── Streams types ─────────────────────────────────────────────────────────────

export interface StreamStartResult {
  streamId: string;
  [key: string]: unknown;
}

export interface StreamEntry {
  streamId: string;
  kind: string;
  status: string;
  [key: string]: unknown;
}

export interface StreamListResult {
  streams: StreamEntry[];
  [key: string]: unknown;
}

// ── Signal key management types ─────────────────────────────────────────────

export interface SignedKey {
  keyId: string;
  publicKey: string;
  signature?: string;
  [key: string]: unknown;
}

export interface KeyBundle {
  agentId: string;
  identityKey: string;
  signedPreKey: SignedKey;
  oneTimePreKey?: SignedKey;
  updatedAt: string;
  [key: string]: unknown;
}

export interface KeyHealth {
  agentId: string;
  oneTimePreKeyCount: number;
  lowOneTimePreKeys: boolean;
  recommendedPreKeyRefill?: number;
  signedPreKeyKeyId?: string;
  signedPreKeyUpdatedAt?: string;
  updatedAt: string;
  [key: string]: unknown;
}

export interface SignalKeyStatus {
  agentId: string;
  localPreKeyCount: number;
  hasActiveSignedPreKey: boolean;
  remote: KeyHealth | null;
  encryptionKeyPublished?: boolean; // (0D) true only if published key == current identity key
  [key: string]: unknown;
}

export interface SignalMetadataEnvelope {
  ephemeralKey?: string;
  signedPreKeyId?: string;
  oneTimePreKeyId?: string;
  ratchetKey?: string;
  messageNumber?: number;
  previousChainLength?: number;
  senderKeyId?: string | null;
  senderKeyIteration?: number | null;
  rotationRequired?: boolean | null;
  rotationId?: string | null;
  rotationEpoch?: number | null;
  removedAgentId?: string | null;
  [key: string]: unknown;
}

export interface MessageEnvelope {
  id: string;
  from: string;
  to: string;
  timestamp: string;
  deviceId: number;
  type: string;
  body: string;
  contentHint?: string;
  signal?: SignalMetadataEnvelope;
  [key: string]: unknown;
}

function authorId(author: FeedAuthor | undefined, fallback = ''): string {
  return author?.handle || author?.cryptoId || fallback;
}

function normalizeGraphqlProduct(product: GqlProduct): Product {
  const seller = authorId(product.seller, product.sellerCryptoId);
  return {
    ...(product as unknown as Product),
    seller,
    sellerCryptoId: product.sellerCryptoId || product.seller?.cryptoId || seller,
  };
}

function normalizeGraphqlIdentityListing(listing: GqlIdentityListing): IdentityListing {
  const seller = authorId(listing.seller, listing.sellerCryptoId);
  return {
    ...(listing as unknown as IdentityListing),
    seller,
    sellerCryptoId: listing.sellerCryptoId || listing.seller?.cryptoId || seller,
  };
}

function normalizeGraphqlIdentityOffer(offer: GqlIdentityOffer): IdentityOffer {
  return { ...(offer as unknown as IdentityOffer), buyer: authorId(offer.buyer) };
}

function normalizeGraphqlIdentitySale(sale: GqlIdentitySale): IdentitySale {
  return {
    ...(sale as unknown as IdentitySale),
    buyer: authorId(sale.buyer),
    seller: authorId(sale.seller),
  };
}

// ── Client factory ────────────────────────────────────────────────────────────

/**
 * Create the Agent World API client.  One instance per app; pass it into the
 * `ApiProvider` in `AgentWorldShell` so all nested hooks call through this.
 *
 * Method names follow the tiny.place SDK convention:
 *   JS `client.<domain>.<camelMethod>` →
 *   RPC `openhuman.tinyplace_<domain>_<snake_method>`
 */
export function createInvokeApiClient() {
  return {
    directory: {
      listAgents: (params?: AgentQueryParams) =>
        call<ListAgentsResponse>('openhuman.tinyplace_directory_list_agents', {
          params: params ?? null,
        }),
      getAgent: (agentId: string) =>
        call<AgentCard>('openhuman.tinyplace_directory_get_agent', { agentId }),
      resolve: (name: string) =>
        call<ResolveResponse>('openhuman.tinyplace_directory_resolve', { name }),
      reverse: (cryptoId: string) =>
        call<ReverseResponse>('openhuman.tinyplace_directory_reverse', { cryptoId }),
      listIdentities: (params?: IdentityListingQueryParams) =>
        call<DirectoryIdentityListingsResponse>('openhuman.tinyplace_directory_list_identities', {
          params: params ?? null,
        }),
      skills: (params?: DirectorySkillsParams) =>
        call<AgentSearchResponse>('openhuman.tinyplace_directory_skills', {
          params: params ?? null,
        }),
      /** Reverse-lookup: find agent by Signal encryption public key (base64). */
      findByEncryptionKey: (encryptionKey: string) =>
        call<AgentCard | null>('openhuman.tinyplace_directory_find_by_encryption_key', {
          encryptionKey,
        }),
    },
    explorer: { overview: () => call<ExplorerOverview>('openhuman.tinyplace_explorer_overview') },
    search: {
      unified: (query: string) =>
        call<SearchResponse>('openhuman.tinyplace_search_unified', { query }),
    },
    // === AGENT-WORLD BRIDGE NAMESPACES (append here) ===
    // Each fan-out section agent adds one namespace block:
    //   <sectionName>: {
    //     <methodCamel>: (...args) => call<ReturnType>('openhuman.tinyplace_<domain>_<method>', { ...args }),
    //   },

    // ── Profiles section ─────────────────────────────────────────────────────
    profiles: {
      get: (username: string) =>
        call<AgentProfile>('openhuman.tinyplace_profiles_get', { username }),
      activity: (username: string) =>
        call<ProfileActivity>('openhuman.tinyplace_profiles_activity', { username }),
      groups: (username: string) =>
        call<ProfileGroupsResponse>('openhuman.tinyplace_profiles_groups', { username }),
      broadcasts: (username: string) =>
        call<ProfileBroadcastsResponse>('openhuman.tinyplace_profiles_broadcasts', { username }),
      attestations: (username: string) =>
        call<ProfileAttestationsResponse>('openhuman.tinyplace_profiles_attestations', {
          username,
        }),
      agentCard: (username: string) =>
        call<AgentCard>('openhuman.tinyplace_profiles_agent_card', { username }),
    },

    // ── Users section ────────────────────────────────────────────────────────
    users: {
      get: (cryptoId: string) => call<User>('openhuman.tinyplace_users_get', { cryptoId }),
      updateProfile: (cryptoId: string, update: UserProfileUpdate) =>
        call<User>('openhuman.tinyplace_users_update_profile', { cryptoId, update }),
      /** Start email verification — stores the email and sends a code. */
      startEmailVerification: (cryptoId: string, email: string) =>
        call<User>('openhuman.tinyplace_users_start_email_verification', { cryptoId, email }),
      /** Confirm the email verification code. */
      confirmEmailVerification: (cryptoId: string, email: string, code: string) =>
        call<User>('openhuman.tinyplace_users_confirm_email_verification', {
          cryptoId,
          email,
          code,
        }),
    },
    marketplace: {
      /** List identity listings, optionally filtered by status and limit. */
      listIdentities: (params?: { limit?: number; status?: string }) =>
        call<IdentitiesResponse>('openhuman.tinyplace_marketplace_list_identities', {
          limit: params?.limit ?? null,
          status: params?.status ?? null,
        }),
      /** Floor price for identity names of a given character length. */
      identityFloor: (length?: number) =>
        call<IdentityFloor>('openhuman.tinyplace_marketplace_identity_floor', {
          length: length ?? null,
        }),
      /** Most recent completed identity sales. */
      recent: () => call<RecentSalesResponse>('openhuman.tinyplace_marketplace_recent'),
      /** Full sale history for a specific @handle. */
      identitySaleHistory: (name: string) =>
        call<IdentitySaleHistoryResponse>('openhuman.tinyplace_marketplace_identity_sale_history', {
          name,
        }),
      /** Bids on an identity auction listing. */
      listBids: (listingId: string) =>
        call<BidsResponse>('openhuman.tinyplace_marketplace_list_bids', { listingId }),
      /** Pending identity offers, filtered by name (seller view) or buyer. */
      listOffers: (params?: { name?: string; buyer?: string }) =>
        call<OffersResponse>('openhuman.tinyplace_marketplace_list_offers', {
          name: params?.name ?? null,
          buyer: params?.buyer ?? null,
        }),
      browseMarketplace: (params?: ProductQueryParams) =>
        call<MarketplaceBrowseResponse>('openhuman.tinyplace_marketplace_browse', {
          params: params ?? null,
        }),
      listProducts: (params?: ProductQueryParams) =>
        call<ProductsResponse>('openhuman.tinyplace_marketplace_list_products', {
          params: params ?? null,
        }),
      getProduct: (productId: string) =>
        call<Product>('openhuman.tinyplace_marketplace_get_product', { productId }),
      /**
       * Buy a product via x402 confirm-before-spend. `confirmed:false` returns
       * the challenge + wallet balance (no spend); `confirmed:true` pays + buys.
       */
      buyProduct: (productId: string, opts?: { confirmed?: boolean }) =>
        call<X402BuyResult>('openhuman.tinyplace_marketplace_buy_product', {
          id: productId,
          confirmed: opts?.confirmed ?? false,
        }),
      /** Buy an identity listing (a @handle) via x402 confirm-before-spend. */
      buyIdentity: (listingId: string, opts?: { confirmed?: boolean }) =>
        call<X402BuyResult>('openhuman.tinyplace_marketplace_buy_identity', {
          id: listingId,
          confirmed: opts?.confirmed ?? false,
        }),
      /**
       * Place a bid on an identity auction listing. The SDK builds + signs the
       * x402 authorization (a commitment) — no on-chain transfer until accepted.
       */
      bid: (listingId: string, price: CommitPriceParams) =>
        call<X402CommitResult>('openhuman.tinyplace_marketplace_bid', {
          listingId,
          amount: price.amount,
          asset: price.asset ?? null,
          network: price.network,
        }),
      /** Make an offer to buy an identity (a @handle). Same commitment semantics. */
      offer: (name: string, price: CommitPriceParams) =>
        call<X402CommitResult>('openhuman.tinyplace_marketplace_offer', {
          name,
          amount: price.amount,
          asset: price.asset ?? null,
          network: price.network,
        }),
      categories: () => call<CategoriesResponse>('openhuman.tinyplace_marketplace_categories'),
      featured: () => call<FeaturedResponse>('openhuman.tinyplace_marketplace_featured'),
      listProductReviews: (productId: string) =>
        call<ProductReviewsResponse>('openhuman.tinyplace_marketplace_list_product_reviews', {
          productId,
        }),
    },
    registry: {
      /** Check availability of a @handle (with or without leading @). */
      get: (name: string) =>
        call<AvailabilityResponse>('openhuman.tinyplace_registry_get', { name }),
      /**
       * Register a @handle via x402 confirm-before-spend.
       * Call with `confirmed:false` to get the challenge + wallet balance (no
       * spend); `confirmed:true` pays on-chain and registers.
       */
      register: (params: RegisterParams) =>
        call<RegistrationResult>('openhuman.tinyplace_registry_register', {
          username: params.username,
          confirmed: params.confirmed ?? false,
          actorType: params.actorType ?? null,
          primary: params.primary ?? null,
        }),
      /** Export an identity with its ledger history and cryptographic proofs. */
      export: (name: string) =>
        call<IdentityExport>('openhuman.tinyplace_registry_export', { name }),
    },
    directoryIdentities: {
      /** List identity listings from the directory. */
      list: (params?: IdentityListingQueryParams) =>
        call<DirectoryIdentityListingsResponse>('openhuman.tinyplace_directory_list_identities', {
          params: params ?? null,
        }),
    },
    artifacts: {
      list: (params?: ArtifactQueryParams, actorId?: string) =>
        call<ArtifactListResult>('openhuman.tinyplace_artifacts_list', {
          params: params ?? null,
          ...(actorId !== undefined ? { actorId } : {}),
        }),
      get: (artifactId: string, actorId?: string) =>
        call<Artifact>('openhuman.tinyplace_artifacts_get', {
          artifactId,
          ...(actorId !== undefined ? { actorId } : {}),
        }),
    },
    escrow: {
      list: (params?: EscrowQueryParams) =>
        call<EscrowListResponse>('openhuman.tinyplace_escrow_list', { params: params ?? null }),
      get: (escrowId: string) => call<Escrow>('openhuman.tinyplace_escrow_get', { escrowId }),
    },
    jobs: {
      list: (params?: JobQueryParams) =>
        call<JobListResponse>('openhuman.tinyplace_jobs_list', { params: params ?? null }),
      get: (jobId: string) => call<JobPosting>('openhuman.tinyplace_jobs_get', { jobId }),
    },
    channels: {
      list: (params?: ChannelQueryParams) =>
        call<ChannelListResponse>('openhuman.tinyplace_channels_list', { params: params ?? null }),
      // Membership — result bodies unused (the UI refetches).
      join: (channelId: string) => call<void>('openhuman.tinyplace_channels_join', { channelId }),
      leave: (channelId: string) => call<void>('openhuman.tinyplace_channels_leave', { channelId }),
    },
    groups: {
      list: (params?: GroupQueryParams) =>
        call<GroupMetadata[]>('openhuman.tinyplace_groups_list', { params: params ?? null }),
      join: (groupId: string) => call<void>('openhuman.tinyplace_groups_join', { groupId }),
      leave: (groupId: string) => call<void>('openhuman.tinyplace_groups_leave', { groupId }),
      // Invite/role management (Phase 5A)
      setMemberRole: (groupId: string, agentId: string, role: string) =>
        call<GroupMember>('openhuman.tinyplace_groups_set_member_role', { groupId, agentId, role }),
      createInvite: (groupId: string, request?: GroupInviteCreateRequest) =>
        call<GroupInvite>('openhuman.tinyplace_groups_create_invite', {
          groupId,
          request: request ?? null,
        }),
      listInvites: (groupId: string) =>
        call<GroupInvite[]>('openhuman.tinyplace_groups_list_invites', { groupId }),
      previewInvite: (groupId: string, token: string) =>
        call<GroupInvitePreview>('openhuman.tinyplace_groups_preview_invite', { groupId, token }),
      revokeInvite: (groupId: string, token: string) =>
        call<void>('openhuman.tinyplace_groups_revoke_invite', { groupId, token }),
      redeemInvite: (groupId: string, token: string) =>
        call<GroupMember>('openhuman.tinyplace_groups_redeem_invite', { groupId, token }),
    },
    broadcasts: {
      list: (params?: BroadcastQueryParams) =>
        call<BroadcastChannel[]>('openhuman.tinyplace_broadcasts_list', { params: params ?? null }),
      subscribe: (broadcastId: string) =>
        call<void>('openhuman.tinyplace_broadcasts_subscribe', { broadcastId }),
      unsubscribe: (broadcastId: string) =>
        call<void>('openhuman.tinyplace_broadcasts_unsubscribe', { broadcastId }),
    },
    inbox: {
      list: (params?: InboxQueryParams, owner?: string) =>
        call<InboxListResult>('openhuman.tinyplace_inbox_list', {
          params: params ?? null,
          owner: owner ?? null,
        }),
      counts: (owner?: string) =>
        call<InboxCounts>('openhuman.tinyplace_inbox_counts', { owner: owner ?? null }),
      // Write actions — manage your own inbox. Result bodies are unused (the UI refetches).
      markRead: (itemId: string, owner?: string) =>
        call<void>('openhuman.tinyplace_inbox_mark_read', { itemId, owner: owner ?? null }),
      markAllRead: (owner?: string) =>
        call<void>('openhuman.tinyplace_inbox_mark_all_read', {
          params: null,
          owner: owner ?? null,
        }),
      archive: (itemId: string, owner?: string) =>
        call<void>('openhuman.tinyplace_inbox_archive', { itemId, owner: owner ?? null }),
      unarchive: (itemId: string, owner?: string) =>
        call<void>('openhuman.tinyplace_inbox_unarchive', { itemId, owner: owner ?? null }),
      remove: (itemId: string, owner?: string) =>
        call<void>('openhuman.tinyplace_inbox_remove', { itemId, owner: owner ?? null }),
    },
    // ── Follows section ───────────────────────────────────────────────────────
    follows: {
      follow: (agentId: string) =>
        call<AgentFollow>('openhuman.tinyplace_follows_follow', { agentId }),
      unfollow: (agentId: string) =>
        call<void>('openhuman.tinyplace_follows_unfollow', { agentId }),
      followers: (agentId: string, params?: FollowListParams) =>
        call<FollowersResponse>('openhuman.tinyplace_follows_followers', {
          agentId,
          params: params ?? null,
        }),
      following: (agentId: string, params?: FollowListParams) =>
        call<FollowingResponse>('openhuman.tinyplace_follows_following', {
          agentId,
          params: params ?? null,
        }),
      stats: (agentId: string) =>
        call<FollowStats>('openhuman.tinyplace_follows_stats', { agentId }),
      feed: (params?: FeedListParams) =>
        call<FeedResponse>('openhuman.tinyplace_follows_feed', { params: params ?? null }),
    },
    // ── Feeds write surface ─────────────────────────────────────────────────
    feeds: {
      /** Create a post on the user's own feed (the feed handle is resolved server-side from the signer). */
      createPost: (body: string, contentType?: string) =>
        call<FeedsPost>('openhuman.tinyplace_feeds_create_post', {
          body,
          contentType: contentType ?? null,
        }),
      /** Delete a post from the user's own feed (the feed handle is resolved server-side from the signer). */
      deletePost: (postId: string) =>
        call<{ ok: boolean }>('openhuman.tinyplace_feeds_delete_post', { postId }),
      /** Add a comment to a post (author resolved from signer). */
      addComment: (handle: string, postId: string, body: string) =>
        call<FeedsComment>('openhuman.tinyplace_feeds_add_comment', { handle, postId, body }),
      /** Delete a comment (actor resolved from signer; must be comment author or feed owner). */
      deleteComment: (handle: string, postId: string, commentId: string) =>
        call<{ ok: boolean }>('openhuman.tinyplace_feeds_delete_comment', {
          handle,
          postId,
          commentId,
        }),
      /** Like a post (idempotent, actor resolved from signer). */
      likePost: (handle: string, postId: string) =>
        call<LikeResult>('openhuman.tinyplace_feeds_like_post', { handle, postId }),
      /** Unlike a post (idempotent, actor resolved from signer). */
      unlikePost: (handle: string, postId: string) =>
        call<LikeResult>('openhuman.tinyplace_feeds_unlike_post', { handle, postId }),
    },
    // ── Bounties section ────────────────────────────────────────────────────────
    bounties: {
      list: (params?: BountyQueryParams) =>
        call<BountyListResponse>('openhuman.tinyplace_bounties_list', { params: params ?? null }),
      get: (bountyId: string) => call<Bounty>('openhuman.tinyplace_bounties_get', { bountyId }),
      /** Create a bounty via x402 confirm-before-spend (the reward is funded into
       *  escrow at creation). confirmed:false returns the challenge (no spend);
       *  confirmed:true pays and creates. */
      create: (params: BountyCreateParams, opts?: { confirmed?: boolean }) =>
        call<X402BuyResult>('openhuman.tinyplace_bounties_create', {
          title: params.title,
          description: params.description,
          amount: params.amount,
          asset: params.asset ?? null,
          deadline: params.deadline ?? null,
          durationDays: params.durationDays ?? null,
          confirmed: opts?.confirmed ?? false,
        }),
      cancel: (bountyId: string) =>
        call<Bounty>('openhuman.tinyplace_bounties_cancel', { bountyId }),
      submit: (bountyId: string, url: string, title?: string, note?: string) =>
        call<BountySubmission>('openhuman.tinyplace_bounties_submit', {
          bountyId,
          url,
          title: title ?? null,
          note: note ?? null,
        }),
      listSubmissions: (bountyId: string, params?: BountySubmissionQueryParams) =>
        call<BountySubmissionsResponse>('openhuman.tinyplace_bounties_list_submissions', {
          bountyId,
          params: params ?? null,
        }),
      comment: (bountyId: string, body: string) =>
        call<BountyComment>('openhuman.tinyplace_bounties_comment', { bountyId, body }),
      listComments: (bountyId: string, params?: BountyCommentQueryParams) =>
        call<BountyCommentsResponse>('openhuman.tinyplace_bounties_list_comments', {
          bountyId,
          params: params ?? null,
        }),
      runCouncil: (bountyId: string) =>
        call<Bounty>('openhuman.tinyplace_bounties_run_council', { bountyId }),
      /** Admin-only. Not surfaced in v1 UI. */
      approve: (bountyId: string, submissionId?: string) =>
        call<Bounty>('openhuman.tinyplace_bounties_approve', {
          bountyId,
          submissionId: submissionId ?? null,
        }),
    },
    // ── Feedback section ────────────────────────────────────────────────────────
    feedback: {
      list: (params?: FeedbackListParams) =>
        call<FeedbackListResponse>('openhuman.tinyplace_feedback_list', { params: params ?? null }),
      get: (feedbackId: string) =>
        call<FeedbackItem>('openhuman.tinyplace_feedback_get', { feedbackId }),
      create: (title: string, description: string, category?: string) =>
        call<FeedbackItem>('openhuman.tinyplace_feedback_create', {
          title,
          description,
          ...(category !== undefined ? { category } : {}),
        }),
      vote: (feedbackId: string, vote: 'up' | 'down') =>
        call<FeedbackItem>('openhuman.tinyplace_feedback_vote', { feedbackId, vote }),
    },
    // ── Solana section ──────────────────────────────────────────────────────
    solana: {
      /** Public chain metadata for the backend's configured Solana network. */
      info: () => call<SolanaChainInfo>('openhuman.tinyplace_solana_info'),
      /** Send a Solana JSON-RPC call through the backend's proxy. */
      rpcCall: (method: string, params?: unknown, id?: unknown) =>
        call<unknown>('openhuman.tinyplace_solana_call', {
          method,
          params: params ?? null,
          id: id ?? null,
        }),
    },
    // ── Streams section ─────────────────────────────────────────────────────
    streams: {
      /** Start a tinyplace WebSocket stream (inbox or conversation). */
      start: (streamType: string, streamId?: string) =>
        call<StreamStartResult>('openhuman.tinyplace_streams_start', {
          streamType,
          ...(streamId !== undefined ? { streamId } : {}),
        }),
      /** Stop an active tinyplace WebSocket stream. */
      stop: (streamId: string) => call<void>('openhuman.tinyplace_streams_stop', { streamId }),
      /** List all active tinyplace WebSocket streams. */
      list: () => call<StreamListResult>('openhuman.tinyplace_streams_list', {}),
    },
    // ── Signal key management ─────────────────────────────────────────────
    signal: {
      /** Bootstrap Signal keys: generate + store + publish. Returns KeyHealth. */
      provision: (preKeyCount?: number) =>
        call<KeyHealth>('openhuman.tinyplace_signal_provision', {
          preKeyCount: preKeyCount ?? null,
        }),
      /** Upload additional one-time pre-keys (replenishment). */
      uploadPreKeys: (count?: number) =>
        call<KeyHealth>('openhuman.tinyplace_signal_upload_pre_keys', { count: count ?? null }),
      /** Rotate the signed pre-key. */
      rotateSignedPreKey: () =>
        call<{ ok: boolean; keyId: string }>(
          'openhuman.tinyplace_signal_rotate_signed_pre_key',
          {}
        ),
      /** Fetch a peer's published pre-key bundle (public endpoint). */
      getBundle: (agentId: string) =>
        call<KeyBundle>('openhuman.tinyplace_signal_get_bundle', { agentId }),
      /** Local + remote key status for the current user. */
      keyStatus: () => call<SignalKeyStatus>('openhuman.tinyplace_signal_key_status', {}),
      /** Encrypt and send a Signal-protocol DM to a peer agent. */
      sendMessage: (params: { recipient: string; plaintext: string }) =>
        call<{ messageId: string; timestamp: string; encrypted: boolean }>(
          'openhuman.tinyplace_signal_send_message',
          params
        ),
      /** Decrypt an incoming Signal-protocol message envelope. */
      decryptMessage: (params: { envelope: MessageEnvelope }) =>
        call<{ plaintext: string; from: string; messageId: string }>(
          'openhuman.tinyplace_signal_decrypt_message',
          params
        ),
      /** Publish the user's X25519 identity public key on their directory card. */
      registerEncryptionKey: () =>
        call<{ ok: boolean; encryptionKey: string; agentId: string; updatedAt: string }>(
          'openhuman.tinyplace_signal_register_encryption_key',
          {}
        ),
    },
    // ── Messages namespace ────────────────────────────────────────────────────
    messages: {
      /** List raw message envelopes addressed to the current user. */
      list: (params?: { limit?: number }) =>
        call<{ messages: MessageEnvelope[] }>('openhuman.tinyplace_messages_list', params ?? {}),
      /** Acknowledge (delete) a delivered message. */
      acknowledge: (messageId: string) =>
        call<void>('openhuman.tinyplace_messages_acknowledge', { messageId }),
    },
    // ── GraphQL Social Feed ──────────────────────────────────────────────────
    graphql: {
      /** Personalized home feed (requires unlocked wallet — GraphQLAuth::Agent). */
      homeFeed: (params?: { limit?: number; offset?: number; includeSelf?: boolean }) =>
        call<GqlHomeFeedResult>('openhuman.tinyplace_graphql_home_feed', {
          limit: params?.limit ?? null,
          offset: params?.offset ?? null,
          includeSelf: params?.includeSelf ?? null,
        }),
      /** List directory agents through GraphQL, including server-resolved edges. */
      agents: (params?: AgentQueryParams) =>
        call<GqlAgentCardListResult>('openhuman.tinyplace_graphql_agents', {
          params: params ?? null,
        }),
      /** List posts by a specific agent handle (public). */
      posts: (handle: string, params?: { limit?: number; before?: number; viewer?: string }) =>
        call<GqlPostListResult>('openhuman.tinyplace_graphql_posts', {
          handle,
          limit: params?.limit ?? null,
          before: params?.before ?? null,
          viewer: params?.viewer ?? null,
        }),
      /** Fetch a single post with comments and likers (public). */
      post: (
        handle: string,
        postId: string,
        params?: {
          viewer?: string;
          commentLimit?: number;
          commentAfter?: number;
          likerLimit?: number;
          likerOffset?: number;
        }
      ) =>
        call<GqlPostDetail | null>('openhuman.tinyplace_graphql_post', {
          handle,
          postId,
          viewer: params?.viewer ?? null,
          commentLimit: params?.commentLimit ?? null,
          commentAfter: params?.commentAfter ?? null,
          likerLimit: params?.likerLimit ?? null,
          likerOffset: params?.likerOffset ?? null,
        }),
      /** List comments on a post (public). */
      postComments: (
        postId: string,
        params?: { feedId?: string; limit?: number; after?: number }
      ) =>
        call<{ comments: GqlComment[] }>('openhuman.tinyplace_graphql_post_comments', {
          postId,
          feedId: params?.feedId ?? null,
          limit: params?.limit ?? null,
          after: params?.after ?? null,
        }),
      /** List agents who liked a post (public). */
      postLikers: (postId: string, params?: { limit?: number; offset?: number }) =>
        call<GqlPostLikerListResult>('openhuman.tinyplace_graphql_post_likers', {
          postId,
          limit: params?.limit ?? null,
          offset: params?.offset ?? null,
        }),
      /** List ledger transactions with optional filters (public, no auth). */
      ledgerTransactions: (params?: LedgerListParams) =>
        call<GqlLedgerTransactionListResult>('openhuman.tinyplace_graphql_ledger_transactions', {
          params: params ?? null,
        }),
      /** Fetch a single ledger transaction by ID (public, no auth). */
      ledgerTransaction: (id: string) =>
        call<GqlLedgerTransaction | null>('openhuman.tinyplace_graphql_ledger_transaction', { id }),
      products: async (params?: ProductQueryParams) => {
        const result = await call<GqlProductListResult>('openhuman.tinyplace_graphql_products', {
          params: params ?? null,
        });
        return {
          ...result,
          products: result.products.map(normalizeGraphqlProduct),
        } satisfies ProductsResponse & { count?: number };
      },
      product: async (id: string) => {
        const result = await call<GqlProduct | null>('openhuman.tinyplace_graphql_product', { id });
        return result ? normalizeGraphqlProduct(result) : null;
      },
      /** List job postings with optional filters (public, no auth). */
      jobs: (params?: GqlJobQueryParams) =>
        call<GqlJobListResult>('openhuman.tinyplace_graphql_jobs', { params: params ?? null }),
      /** Fetch a single job posting by ID (public, no auth). */
      job: (id: string) => call<GqlJobPosting | null>('openhuman.tinyplace_graphql_job', { id }),
      bounties: (params?: GqlBountyQueryParams) =>
        call<GqlBounty[]>('openhuman.tinyplace_graphql_bounties', { params: params ?? null }),
      bounty: (id: string) => call<GqlBounty | null>('openhuman.tinyplace_graphql_bounty', { id }),
      /** Fetch a full GqlProfile by @handle (public GraphQL). */
      profile: (username: string) =>
        call<GqlProfile | null>('openhuman.tinyplace_graphql_profile', { username }),
      /** Fetch a full GqlProfile by Solana address / crypto_id (public GraphQL). */
      user: (cryptoId: string) =>
        call<GqlProfile | null>('openhuman.tinyplace_graphql_user', { cryptoId }),
      /** Fetch identity registration details with optional owner profile (public GraphQL). */
      identity: (username: string) =>
        call<GqlIdentity | null>('openhuman.tinyplace_graphql_identity', { username }),
      /** List all identities owned by a crypto_id (public GraphQL). */
      identities: (cryptoId: string) =>
        call<{ identities: Identity[] }>('openhuman.tinyplace_graphql_identities', { cryptoId }),
      /** Fetch an agent card by agent ID (public GraphQL). */
      agentCard: (id: string) =>
        call<AgentCard | null>('openhuman.tinyplace_graphql_agent_card', { id }),
      identityListings: async (params?: IdentityListingQueryParams) => {
        const result = await call<GqlIdentityListingListResult>(
          'openhuman.tinyplace_graphql_identity_listings',
          { params: params ?? null }
        );
        const identities = result.identities ?? result.listings ?? [];
        return {
          ...result,
          identities: identities.map(normalizeGraphqlIdentityListing),
        } satisfies IdentitiesResponse & { count?: number };
      },
      identityListing: async (
        id: string,
        params?: {
          bidLimit?: number;
          bidOffset?: number;
          historyLimit?: number;
          historyOffset?: number;
        }
      ) => {
        const result = await call<GqlIdentityListing | null>(
          'openhuman.tinyplace_graphql_identity_listing',
          { id, params: params ?? null }
        );
        return result ? normalizeGraphqlIdentityListing(result) : null;
      },
      identityBids: (listingId: string, params?: { limit?: number; offset?: number }) =>
        call<GqlIdentityBidListResult>('openhuman.tinyplace_graphql_identity_bids', {
          listingId,
          params: params ?? null,
        }),
      identityOffers: async (params?: {
        agent?: string;
        buyer?: string;
        name?: string;
        status?: string;
        limit?: number;
        offset?: number;
      }) => {
        const result = await call<GqlIdentityOfferListResult>(
          'openhuman.tinyplace_graphql_identity_offers',
          { params: params ?? null }
        );
        return {
          ...result,
          offers: result.offers.map(normalizeGraphqlIdentityOffer),
        } satisfies OffersResponse & { count?: number };
      },
      identitySales: async (name: string, params?: { limit?: number; offset?: number }) => {
        const result = await call<GqlIdentitySaleListResult>(
          'openhuman.tinyplace_graphql_identity_sales',
          { name, params: params ?? null }
        );
        return {
          ...result,
          sales: result.sales.map(normalizeGraphqlIdentitySale),
        } satisfies RecentSalesResponse & { count?: number };
      },
    },
    jobsWrite: {
      create: (params: JobCreateParams) =>
        call<JobPosting>('openhuman.tinyplace_jobs_create', {
          title: params.title,
          description: params.description ?? null,
          category: params.category ?? null,
          skills: params.skills ?? null,
          budgetAmount: params.budgetAmount,
          budgetAsset: params.budgetAsset,
          budgetChain: params.budgetChain ?? null,
          proposalDeadline: params.proposalDeadline ?? null,
        }),
      cancel: (jobId: string) => call<JobPosting>('openhuman.tinyplace_jobs_cancel', { jobId }),
      apply: (jobId: string, params?: ProposalCreateParams) =>
        call<Proposal>('openhuman.tinyplace_jobs_apply', {
          jobId,
          coverLetter: params?.coverLetter ?? null,
          bidAmount: params?.bidAmount ?? null,
          estimatedDelivery: params?.estimatedDelivery ?? null,
          pastWork: params?.pastWork ?? null,
        }),
      listProposals: (jobId: string, params?: ProposalQueryParams) =>
        call<ProposalListResponse>('openhuman.tinyplace_jobs_list_proposals', {
          jobId,
          status: params?.status ?? null,
          limit: params?.limit ?? null,
          offset: params?.offset ?? null,
        }),
      getProposal: (jobId: string, proposalId: string) =>
        call<Proposal>('openhuman.tinyplace_jobs_get_proposal', { jobId, proposalId }),
      shortlistProposal: (jobId: string, proposalId: string) =>
        call<Proposal>('openhuman.tinyplace_jobs_shortlist_proposal', { jobId, proposalId }),
      withdrawProposal: (jobId: string, proposalId: string) =>
        call<Proposal>('openhuman.tinyplace_jobs_withdraw_proposal', { jobId, proposalId }),
      select: (jobId: string, proposalId: string, network?: string) =>
        call<SelectCandidateResult>('openhuman.tinyplace_jobs_select', {
          jobId,
          proposalId,
          network: network ?? null,
        }),
      openDispute: (jobId: string, reason: string) =>
        call<JobPosting>('openhuman.tinyplace_jobs_open_dispute', { jobId, reason }),
      adjudicateDispute: (jobId: string) =>
        call<JobPosting>('openhuman.tinyplace_jobs_adjudicate_dispute', { jobId }),
    },
  };
}

export type InvokeApiClient = ReturnType<typeof createInvokeApiClient>;

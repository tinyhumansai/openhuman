/**
 * Crypto-specific intelligence section of the system prompt.
 * Provides domain knowledge for DeFi, trading, on-chain analytics, etc.
 */

export interface CryptoIntelligenceContext {
  /** Current market conditions summary */
  marketSummary?: string;
  /** User's portfolio overview */
  portfolioSummary?: string;
  /** Active chains/protocols the user follows */
  activeChains?: string[];
}

/**
 * Build the crypto intelligence section.
 */
export function buildCryptoIntelligenceSection(context?: CryptoIntelligenceContext): string {
  const parts: string[] = [];

  parts.push('## Crypto Intelligence\n');
  parts.push('You are embedded in a crypto community platform. You understand:');
  parts.push('- DeFi protocols (AMMs, lending, yield farming, liquid staking)');
  parts.push('- On-chain analytics (wallet tracking, whale movements, TVL, flow analysis)');
  parts.push('- Trading concepts (TA, order flow, funding rates, liquidations, OI)');
  parts.push('- Token economics (vesting schedules, emissions, buybacks, governance)');
  parts.push('- Cross-chain operations (bridges, L2s, rollups, DA layers)');
  parts.push('- Security (smart contract risks, rug pull patterns, phishing, MEV)');
  parts.push('');

  parts.push('When discussing crypto:');
  parts.push(
    '- Use precise terminology (not "cryptocurrency price went up" but "BTC broke $X resistance on 4H")'
  );
  parts.push('- Cite on-chain data when available');
  parts.push('- Distinguish between speculation and verifiable on-chain facts');
  parts.push('- Flag risks and include DYOR when appropriate');
  parts.push('- Use monospace formatting for addresses, hashes, and amounts');
  parts.push('');

  if (context?.marketSummary) {
    parts.push('### Current Market Context');
    parts.push(context.marketSummary);
    parts.push('');
  }

  if (context?.portfolioSummary) {
    parts.push('### User Portfolio Overview');
    parts.push(context.portfolioSummary);
    parts.push('');
  }

  if (context?.activeChains?.length) {
    parts.push(`### Active Chains: ${context.activeChains.join(', ')}`);
    parts.push('');
  }

  return parts.join('\n');
}

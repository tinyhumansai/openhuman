import type { ActionableItem } from '../../types/intelligence';

// Helper function to create dates relative to now
function createDate(minutesAgo: number): Date {
  return new Date(Date.now() - minutesAgo * 60 * 1000);
}

function createDateDays(daysAgo: number): Date {
  return new Date(Date.now() - daysAgo * 24 * 60 * 60 * 1000);
}

export const MOCK_ACTIONABLE_ITEMS: ActionableItem[] = [
  // Today - Fresh items
  {
    id: '1',
    title: 'Reply to 2 critical emails expecting response within 24hrs',
    description:
      'Messages from john@coinbase.com and sarah@ethereum.org about partnership proposals',
    source: 'email',
    priority: 'critical',
    status: 'active',
    createdAt: createDate(2),
    updatedAt: createDate(2),
    expiresAt: createDate(-1440), // Expires in 24 hours
    actionable: true,
    hasComplexAction: true,
    sourceLabel: 'Gmail',
  },

  {
    id: '2',
    title: 'Meeting with Sarah in 30 minutes - prepare documents?',
    description:
      'Q4 strategy meeting. Need to review the crypto market analysis and portfolio updates.',
    source: 'calendar',
    priority: 'important',
    status: 'active',
    createdAt: createDate(5),
    updatedAt: createDate(5),
    actionable: true,
    hasComplexAction: false,
    sourceLabel: 'Calendar',
  },

  {
    id: '3',
    title: 'Order lunch for your 2pm work session?',
    description:
      'You usually order from "Crypto Café" around this time. Your favorite: Bitcoin Bowl ($15.99)',
    source: 'ai_insight',
    priority: 'normal',
    status: 'active',
    createdAt: createDate(15),
    updatedAt: createDate(15),
    actionable: true,
    hasComplexAction: false,
    sourceLabel: 'AI Assistant',
  },

  {
    id: '4',
    title: '5 unread messages from Alpha Trading Group',
    description:
      '@cryptowhale mentioned you about the ETH analysis. 3 other traders are discussing market trends.',
    source: 'telegram',
    priority: 'normal',
    status: 'active',
    createdAt: createDate(25),
    updatedAt: createDate(25),
    actionable: true,
    hasComplexAction: true,
    sourceLabel: 'Telegram',
  },

  {
    id: '5',
    title: 'Free 45min slot available for gym session',
    description: 'Your calendar shows a gap between 3:15-4:00 PM. LA Fitness has availability.',
    source: 'ai_insight',
    priority: 'normal',
    status: 'active',
    createdAt: createDate(45),
    updatedAt: createDate(45),
    actionable: true,
    hasComplexAction: false,
    sourceLabel: 'AI Planner',
  },

  // Yesterday
  {
    id: '6',
    title: 'Alex mentioned you in Alpha Human Development chat',
    description: '"@john can you review the new trading algorithm?" - 3 hours ago',
    source: 'telegram',
    priority: 'important',
    status: 'active',
    createdAt: createDate(180),
    updatedAt: createDate(180),
    actionable: true,
    hasComplexAction: true,
    sourceLabel: 'Telegram',
  },

  {
    id: '7',
    title: 'Update available for DeFi trading bot',
    description:
      'Version 2.1.4 includes security patches and 15% better performance. Requires restart.',
    source: 'system',
    priority: 'important',
    status: 'active',
    createdAt: createDate(360),
    updatedAt: createDate(360),
    actionable: true,
    requiresConfirmation: true,
    sourceLabel: 'System',
  },

  {
    id: '8',
    title: 'Backup your crypto wallet keys',
    description: 'Last backup was 30 days ago. Your portfolio is up 34% since then.',
    source: 'security',
    priority: 'critical',
    status: 'active',
    createdAt: createDate(420),
    updatedAt: createDate(420),
    actionable: true,
    requiresConfirmation: false,
    hasComplexAction: true,
    sourceLabel: 'Security',
  },

  // This week
  {
    id: '9',
    title: 'Schedule meeting with VCs for Series A discussion',
    description: 'Follow up on last weeks pitch. Andreessen Horowitz and Sequoia are interested.',
    source: 'email',
    priority: 'critical',
    status: 'active',
    createdAt: createDateDays(2),
    updatedAt: createDateDays(2),
    actionable: true,
    hasComplexAction: true,
    sourceLabel: 'Gmail',
  },

  {
    id: '10',
    title: 'Review 12 pending GitHub pull requests',
    description:
      'Features for v0.21.0 release. 8 from team members, 4 from community contributors.',
    source: 'system',
    priority: 'important',
    status: 'active',
    createdAt: createDateDays(3),
    updatedAt: createDateDays(3),
    actionable: true,
    hasComplexAction: true,
    sourceLabel: 'GitHub',
  },

  {
    id: '11',
    title: 'BTC price alert: Approaching your target of $65,000',
    description: 'Currently at $64,420. You set this alert 2 weeks ago. Consider taking profits?',
    source: 'trading',
    priority: 'important',
    status: 'active',
    createdAt: createDateDays(3),
    updatedAt: createDateDays(3),
    actionable: true,
    hasComplexAction: false,
    sourceLabel: 'Trading Bot',
  },

  {
    id: '12',
    title: 'Complete KYC verification for Binance account',
    description: 'Required for withdrawal limits above $2,000. Upload ID and proof of address.',
    source: 'trading',
    priority: 'normal',
    status: 'active',
    createdAt: createDateDays(4),
    updatedAt: createDateDays(4),
    actionable: true,
    hasComplexAction: true,
    sourceLabel: 'Binance',
  },

  // Older items
  {
    id: '13',
    title: 'Respond to podcast interview request from Lex Fridman',
    description: 'Topic: "The Future of Decentralized Communication". Suggested dates: next month.',
    source: 'email',
    priority: 'important',
    status: 'active',
    createdAt: createDateDays(8),
    updatedAt: createDateDays(8),
    actionable: true,
    hasComplexAction: true,
    sourceLabel: 'Gmail',
  },

  {
    id: '14',
    title: 'Submit tax documents for Q3 crypto transactions',
    description: '47 transactions across 5 exchanges. Deadline: October 15th.',
    source: 'system',
    priority: 'critical',
    status: 'active',
    createdAt: createDateDays(10),
    updatedAt: createDateDays(10),
    expiresAt: createDate(-14400), // Expires in 10 days
    actionable: true,
    hasComplexAction: true,
    requiresConfirmation: false,
    sourceLabel: 'TaxBot',
  },

  {
    id: '15',
    title: 'Renew SSL certificate for api.openhuman.com',
    description:
      'Certificate expires in 5 days. Automatic renewal failed - manual intervention required.',
    source: 'system',
    priority: 'critical',
    status: 'active',
    createdAt: createDateDays(12),
    updatedAt: createDateDays(12),
    actionable: true,
    hasComplexAction: false,
    requiresConfirmation: false,
    sourceLabel: 'DevOps',
  },

  {
    id: '16',
    title: 'Plan team building event for remote employees',
    description:
      '15 team members across 8 time zones. Budget approved: $5,000. Preference: virtual escape room.',
    source: 'ai_insight',
    priority: 'normal',
    status: 'active',
    createdAt: createDateDays(14),
    updatedAt: createDateDays(14),
    actionable: true,
    hasComplexAction: true,
    sourceLabel: 'HR Assistant',
  },

  // Expired/old items that should show as low priority
  {
    id: '17',
    title: 'Update LinkedIn profile with latest achievements',
    description: 'Add recent TechCrunch feature and Y Combinator alumni status.',
    source: 'ai_insight',
    priority: 'normal',
    status: 'active',
    createdAt: createDateDays(20),
    updatedAt: createDateDays(20),
    actionable: true,
    hasComplexAction: false,
    sourceLabel: 'Career AI',
  },

  {
    id: '18',
    title: 'Review and approve marketing budget for Q4',
    description: '$50K allocated for influencer partnerships and conference sponsorships.',
    source: 'email',
    priority: 'important',
    status: 'active',
    createdAt: createDateDays(21),
    updatedAt: createDateDays(21),
    actionable: true,
    hasComplexAction: true,
    sourceLabel: 'Team Email',
  },

  {
    id: '19',
    title: 'Check server capacity for expected user growth',
    description: 'Daily active users grew 23% this month. Current infrastructure may need scaling.',
    source: 'system',
    priority: 'normal',
    status: 'active',
    createdAt: createDateDays(25),
    updatedAt: createDateDays(25),
    actionable: true,
    hasComplexAction: false,
    sourceLabel: 'Monitoring',
  },

  {
    id: '20',
    title: 'Organize crypto portfolio - consolidate across 7 wallets',
    description:
      'Assets spread across MetaMask, Ledger, Trust Wallet, and 4 others. Gas fees optimized for Sunday.',
    source: 'security',
    priority: 'normal',
    status: 'active',
    createdAt: createDateDays(30),
    updatedAt: createDateDays(30),
    actionable: true,
    hasComplexAction: true,
    sourceLabel: 'Portfolio AI',
  },
];

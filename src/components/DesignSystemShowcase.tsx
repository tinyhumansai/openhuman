import React from 'react';

/**
 * Design System Showcase Component
 *
 * This component demonstrates the premium design system created for the crypto platform.
 * It showcases all the major components and styles that capture the calm, trustworthy
 * aesthetic from the reference screenshots while elevating it for professional crypto users.
 */

const DesignSystemShowcase: React.FC = () => {
  return (
    <div className="min-h-screen bg-gradient-to-br from-canvas-50 via-canvas-100 to-primary-50/20">
      {/* Hero Section - Similar to Screenshot #1 but elevated */}
      <section className="relative overflow-hidden">
        {/* Atmospheric Background */}
        <div className="absolute inset-0 bg-gradient-to-b from-primary-50/30 via-transparent to-transparent" />
        <div className="absolute top-0 right-0 w-96 h-96 bg-accent-sky/20 rounded-full filter blur-3xl" />
        <div className="absolute bottom-0 left-0 w-96 h-96 bg-accent-lavender/20 rounded-full filter blur-3xl" />

        <div className="relative z-10 container mx-auto px-6 py-24">
          {/* Greeting Section */}
          <div className="max-w-4xl">
            <h1 className="text-6xl font-display font-bold text-stone-900 mb-4">
              Good afternoon,
              <span className="text-gradient"> Cyrus</span>
            </h1>
            <p className="text-xl text-stone-600 mb-8">Your portfolio is up 12.5% today</p>

            {/* Status Card */}
            <div className="glass-surface inline-flex items-center space-x-4 px-6 py-4 rounded-2xl">
              <div className="flex items-center space-x-2">
                <div className="status-indicator online">
                  <span className="absolute inline-flex h-full w-full rounded-full bg-sage-400 opacity-75"></span>
                  <span className="relative inline-flex rounded-full h-3 w-3 bg-sage-500"></span>
                </div>
                <span className="text-stone-700 font-medium">Markets Open</span>
              </div>
              <div className="h-4 w-px bg-stone-300" />
              <span className="text-stone-600">26°C and clear</span>
            </div>
          </div>

          {/* Quick Actions */}
          <div className="mt-12 flex flex-wrap gap-4">
            <button className="btn-premium">
              <svg className="w-5 h-5 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M12 4v16m8-8H4"
                />
              </svg>
              Start Trading
            </button>
            <button className="btn-glass">View Portfolio</button>
            <button className="btn-outline">Market Analysis</button>
          </div>
        </div>
      </section>

      {/* Color Palette Section */}
      <section className="container mx-auto px-6 py-16">
        <h2 className="text-3xl font-display font-bold text-stone-900 mb-8">Color System</h2>

        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
          {/* Primary Colors */}
          <div className="card-elevated p-6">
            <h3 className="text-lg font-semibold text-stone-900 mb-4">Primary Ocean</h3>
            <div className="space-y-2">
              <div className="flex items-center space-x-3">
                <div className="w-12 h-12 bg-primary-500 rounded-xl shadow-soft" />
                <div>
                  <p className="text-sm font-medium text-stone-900">Primary 500</p>
                  <p className="text-xs text-stone-500">#5B9BF3</p>
                </div>
              </div>
              <div className="flex items-center space-x-3">
                <div className="w-12 h-12 bg-primary-600 rounded-xl shadow-soft" />
                <div>
                  <p className="text-sm font-medium text-stone-900">Primary 600</p>
                  <p className="text-xs text-stone-500">#4A83DD</p>
                </div>
              </div>
            </div>
          </div>

          {/* Success Colors */}
          <div className="card-elevated p-6">
            <h3 className="text-lg font-semibold text-stone-900 mb-4">Sage Success</h3>
            <div className="space-y-2">
              <div className="flex items-center space-x-3">
                <div className="w-12 h-12 bg-sage-500 rounded-xl shadow-soft" />
                <div>
                  <p className="text-sm font-medium text-stone-900">Sage 500</p>
                  <p className="text-xs text-stone-500">#4DC46F</p>
                </div>
              </div>
              <div className="flex items-center space-x-3">
                <div className="w-12 h-12 bg-sage-600 rounded-xl shadow-soft" />
                <div>
                  <p className="text-sm font-medium text-stone-900">Sage 600</p>
                  <p className="text-xs text-stone-500">#3BA858</p>
                </div>
              </div>
            </div>
          </div>

          {/* Accent Colors */}
          <div className="card-elevated p-6">
            <h3 className="text-lg font-semibold text-stone-900 mb-4">Accent Palette</h3>
            <div className="space-y-2">
              <div className="flex items-center space-x-3">
                <div className="w-12 h-12 bg-accent-lavender rounded-xl shadow-soft" />
                <div>
                  <p className="text-sm font-medium text-stone-900">Lavender</p>
                  <p className="text-xs text-stone-500">#9B8AFB</p>
                </div>
              </div>
              <div className="flex items-center space-x-3">
                <div className="w-12 h-12 bg-accent-sky rounded-xl shadow-soft" />
                <div>
                  <p className="text-sm font-medium text-stone-900">Sky</p>
                  <p className="text-xs text-stone-500">#7DD3FC</p>
                </div>
              </div>
            </div>
          </div>
        </div>
      </section>

      {/* Typography Section */}
      <section className="container mx-auto px-6 py-16">
        <h2 className="text-3xl font-display font-bold text-stone-900 mb-8">Typography Scale</h2>

        <div className="card-elevated p-8 space-y-6">
          <div>
            <p className="text-xs text-stone-500 mb-1">Display · 4.5rem</p>
            <h1 className="text-7xl font-display font-bold text-stone-900">Beautiful Typography</h1>
          </div>

          <div>
            <p className="text-xs text-stone-500 mb-1">Heading 1 · 3rem</p>
            <h2 className="text-5xl font-display font-semibold text-stone-900">
              Clear Information Hierarchy
            </h2>
          </div>

          <div>
            <p className="text-xs text-stone-500 mb-1">Heading 2 · 1.875rem</p>
            <h3 className="text-3xl font-display font-semibold text-stone-900">
              Structured Content Design
            </h3>
          </div>

          <div>
            <p className="text-xs text-stone-500 mb-1">Body · 1rem</p>
            <p className="text-base text-stone-700 leading-relaxed max-w-3xl">
              The typography system is designed to create clear visual hierarchy while maintaining
              excellent readability. Each text size has been carefully calibrated with appropriate
              line heights and letter spacing to ensure optimal legibility across all device sizes.
            </p>
          </div>

          <div>
            <p className="text-xs text-stone-500 mb-1">Monospace · For prices and data</p>
            <p className="text-crypto-price text-2xl text-stone-900">
              $48,392.50 <span className="text-market-bullish">+2.45%</span>
            </p>
          </div>
        </div>
      </section>

      {/* Card Components Section */}
      <section className="container mx-auto px-6 py-16">
        <h2 className="text-3xl font-display font-bold text-stone-900 mb-8">Card Components</h2>

        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
          {/* Market Card */}
          <div className="market-card">
            <div className="relative z-10">
              <div className="flex items-center justify-between mb-4">
                <div className="flex items-center space-x-3">
                  <div className="w-10 h-10 bg-market-bitcoin/20 rounded-xl flex items-center justify-center">
                    <span className="text-market-bitcoin font-bold">₿</span>
                  </div>
                  <div>
                    <p className="font-semibold text-stone-900">Bitcoin</p>
                    <p className="text-sm text-stone-500">BTC</p>
                  </div>
                </div>
                <span className="badge-premium">Trending</span>
              </div>

              <div className="space-y-2">
                <p className="price-ticker up">$48,392.50</p>
                <div className="flex items-center space-x-2">
                  <span className="text-sm text-market-bullish font-medium">+$1,185.20</span>
                  <span className="text-sm text-market-bullish">(+2.45%)</span>
                </div>
              </div>
            </div>
          </div>

          {/* Interactive Card */}
          <div className="card-interactive">
            <h3 className="text-lg font-semibold text-stone-900 mb-2">Portfolio Overview</h3>
            <p className="text-stone-600 mb-4">
              Track your investments and monitor performance in real-time.
            </p>
            <div className="flex items-center text-primary-600 font-medium">
              <span>View Details</span>
              <svg className="w-4 h-4 ml-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M9 5l7 7-7 7"
                />
              </svg>
            </div>
          </div>

          {/* Elevated Card */}
          <div className="card-elevated">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-lg font-semibold text-stone-900">Quick Stats</h3>
              <div className="w-8 h-8 bg-primary-100 rounded-lg flex items-center justify-center">
                <svg
                  className="w-4 h-4 text-primary-600"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"
                  />
                </svg>
              </div>
            </div>
            <div className="space-y-3">
              <div className="flex justify-between items-center">
                <span className="text-sm text-stone-600">Total Value</span>
                <span className="font-mono font-semibold text-stone-900">$125,430</span>
              </div>
              <div className="flex justify-between items-center">
                <span className="text-sm text-stone-600">24h Change</span>
                <span className="font-mono font-semibold text-market-bullish">+3.2%</span>
              </div>
              <div className="flex justify-between items-center">
                <span className="text-sm text-stone-600">Holdings</span>
                <span className="font-mono font-semibold text-stone-900">12</span>
              </div>
            </div>
          </div>
        </div>
      </section>

      {/* Navigation Pattern - Similar to Screenshot #2 */}
      <section className="container mx-auto px-6 py-16">
        <h2 className="text-3xl font-display font-bold text-stone-900 mb-8">
          Navigation & Settings
        </h2>

        <div className="max-w-md">
          <div className="card-elevated p-2">
            {/* User Profile Section */}
            <div className="p-4 border-b border-stone-200">
              <div className="flex items-center space-x-4">
                <div className="w-12 h-12 bg-gradient-to-br from-primary-400 to-accent-lavender rounded-xl" />
                <div>
                  <p className="font-semibold text-stone-900">Cyrus Amini</p>
                  <p className="text-sm text-stone-500">Premium Member</p>
                </div>
              </div>
            </div>

            {/* Navigation Items */}
            <nav className="p-2">
              <a href="#" className="nav-item-premium active">
                <svg className="w-5 h-5 mr-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M3 12l2-2m0 0l7-7 7 7M5 10v10a1 1 0 001 1h3m10-11l2 2m-2-2v10a1 1 0 01-1 1h-3m-6 0a1 1 0 001-1v-4a1 1 0 011-1h2a1 1 0 011 1v4a1 1 0 001 1m-6 0h6"
                  />
                </svg>
                Dashboard
              </a>
              <a href="#" className="nav-item-premium">
                <svg className="w-5 h-5 mr-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z"
                  />
                </svg>
                Profile Settings
              </a>
              <a href="#" className="nav-item-premium">
                <svg className="w-5 h-5 mr-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9"
                  />
                </svg>
                Notifications
              </a>
              <a href="#" className="nav-item-premium">
                <svg className="w-5 h-5 mr-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z"
                  />
                </svg>
                Security
              </a>
              <a href="#" className="nav-item-premium">
                <svg className="w-5 h-5 mr-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                  />
                </svg>
                Help & Support
              </a>
            </nav>
          </div>
        </div>
      </section>

      {/* Form Elements */}
      <section className="container mx-auto px-6 py-16">
        <h2 className="text-3xl font-display font-bold text-stone-900 mb-8">Form Elements</h2>

        <div className="max-w-2xl">
          <div className="card-elevated p-8 space-y-6">
            <div>
              <label className="block text-sm font-medium text-stone-700 mb-2">Email Address</label>
              <input type="email" className="input-elevated" placeholder="Enter your email" />
            </div>

            <div>
              <label className="block text-sm font-medium text-stone-700 mb-2">
                Investment Amount
              </label>
              <div className="relative">
                <span className="absolute left-4 top-3 text-stone-500">$</span>
                <input type="text" className="input-elevated pl-8" placeholder="0.00" />
              </div>
            </div>

            <div>
              <label className="block text-sm font-medium text-stone-700 mb-2">
                Select Currency
              </label>
              <select className="input-elevated">
                <option>Bitcoin (BTC)</option>
                <option>Ethereum (ETH)</option>
                <option>USDT</option>
              </select>
            </div>

            <div className="flex items-center space-x-3">
              <input
                type="checkbox"
                id="terms"
                className="w-4 h-4 text-primary-600 border-stone-300 rounded focus:ring-primary-500"
              />
              <label htmlFor="terms" className="text-sm text-stone-700">
                I agree to the terms and conditions
              </label>
            </div>

            <button className="btn-premium w-full">Complete Transaction</button>
          </div>
        </div>
      </section>
    </div>
  );
};

export default DesignSystemShowcase;

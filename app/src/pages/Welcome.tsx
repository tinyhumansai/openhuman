import { useState } from 'react';

import OAuthProviderButton from '../components/oauth/OAuthProviderButton';
import { oauthProviderConfigs } from '../components/oauth/providerConfigs';
import RotatingTetrahedronCanvas from '../components/RotatingTetrahedronCanvas';
import { sendEmailMagicLink } from '../services/api/authApi';
import {
  clearStoredRpcUrl,
  getDefaultRpcUrl,
  getStoredRpcUrl,
  isValidRpcUrl,
  normalizeRpcUrl,
  storeRpcUrl,
} from '../utils/configPersistence';
import { useDeepLinkAuthState } from '../store/deepLinkAuthState';
import { isTauri } from '../utils/tauriCommands';

// Desktop deep-link scheme root; must match DESKTOP_FRONTEND_URI on the backend
const DESKTOP_FRONTEND_URI = 'openhuman://';

const Welcome = () => {
  const { isProcessing, errorMessage } = useDeepLinkAuthState();

  const [email, setEmail] = useState('');
  const [isSending, setIsSending] = useState(false);
  const [isSent, setIsSent] = useState(false);
  const [emailError, setEmailError] = useState<string | null>(null);

  // RPC URL configuration state
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [rpcUrl, setRpcUrl] = useState(getStoredRpcUrl());
  const [rpcUrlError, setRpcUrlError] = useState<string | null>(null);
  const [isTestingConnection, setIsTestingConnection] = useState(false);
  const [connectionTested, setConnectionTested] = useState(false);

  // Persist RPC URL changes to localStorage
  const handleRpcUrlChange = (value: string) => {
    setRpcUrl(value);
    setRpcUrlError(null);
    setConnectionTested(false);
  };

  // Save RPC URL preference
  const handleSaveRpcUrl = () => {
    const normalized = normalizeRpcUrl(rpcUrl);

    if (!isValidRpcUrl(normalized)) {
      setRpcUrlError('Please enter a valid HTTP or HTTPS URL');
      return;
    }

    storeRpcUrl(normalized);
    setRpcUrlError(null);
    setConnectionTested(true);

    // Show brief success feedback
    setTimeout(() => setConnectionTested(false), 2000);
  };

  // Reset RPC URL to default
  const handleResetRpcUrl = () => {
    clearStoredRpcUrl();
    setRpcUrl(getDefaultRpcUrl());
    setRpcUrlError(null);
    setConnectionTested(false);
  };

  // Test RPC connection
  const handleTestConnection = async () => {
    const normalized = normalizeRpcUrl(rpcUrl);

    if (!isValidRpcUrl(normalized)) {
      setRpcUrlError('Please enter a valid HTTP or HTTPS URL');
      return;
    }

    setIsTestingConnection(true);
    setRpcUrlError(null);

    try {
      // Simple health check - try to fetch the RPC endpoint
      const response = await fetch(normalized, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 1,
          method: 'openhuman.ping',
          params: {},
        }),
      });

      if (response.ok || response.status === 400 || response.status === 405) {
        // 400/405 means the endpoint exists but method not found - still valid
        setConnectionTested(true);
        storeRpcUrl(normalized);
      } else {
        setRpcUrlError(`Connection failed: ${response.status} ${response.statusText}`);
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unable to reach the RPC endpoint';
      setRpcUrlError(message);
    } finally {
      setIsTestingConnection(false);
    }
  };

  const handleEmailSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!email.trim() || isSending) return;

    setEmailError(null);
    setIsSending(true);

    try {
      // Desktop: redirect back via openhuman:// deep link.
      // Web: redirect to the current origin so the app's hash router picks up the token.
      const frontendRedirectUri = isTauri() ? DESKTOP_FRONTEND_URI : window.location.origin;
      await sendEmailMagicLink(email.trim(), frontendRedirectUri);
      setIsSent(true);
    } catch (err) {
      const message =
        err instanceof Error ? err.message : 'Something went wrong. Please try again.';
      setEmailError(message);
    } finally {
      setIsSending(false);
    }
  };

  return (
    <div className="min-h-full flex flex-col items-center justify-center p-4">
      <div className="max-w-md w-full">
        <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-8 animate-fade-up">
          {/* Logo */}
          <div className="flex justify-center mb-6">
            <div className="h-20 w-20">
              <RotatingTetrahedronCanvas />
            </div>
          </div>

          {/* Heading */}
          <h1 className="text-2xl font-bold text-stone-900 text-center mb-2">
            Sign in! Let's Cook
          </h1>

          {/* Subtitle */}
          <p className="text-sm text-stone-500 text-center mb-6 leading-relaxed">
            Welcome to <span className="font-medium text-stone-900">OpenHuman</span>! Your Personal
            AI Super Intelligence. Private, Simple and extremely powerful.
          </p>

          {/* RPC URL Configuration (Advanced) */}
          {showAdvanced ? (
            <div className="mb-5 p-4 bg-stone-50 rounded-xl border border-stone-200">
              <div className="flex items-center justify-between mb-3">
                <label htmlFor="rpc-url-input" className="text-xs font-medium text-stone-700">
                  Core RPC URL
                </label>
                <button
                  type="button"
                  onClick={() => setShowAdvanced(false)}
                  className="text-xs text-stone-500 hover:text-stone-700">
                  Close
                </button>
              </div>
              <div className="flex gap-2">
                <input
                  id="rpc-url-input"
                  type="url"
                  value={rpcUrl}
                  onChange={e => handleRpcUrlChange(e.target.value)}
                  placeholder="http://127.0.0.1:7788/rpc"
                  className="flex-1 rounded-lg border border-stone-300 bg-white px-3 py-2 text-xs text-stone-900 placeholder:text-stone-400 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
                />
                <button
                  type="button"
                  onClick={handleTestConnection}
                  disabled={isTestingConnection}
                  className="px-3 py-2 bg-stone-200 hover:bg-stone-300 text-stone-700 text-xs font-medium rounded-lg transition-colors disabled:opacity-60">
                  {isTestingConnection ? (
                    <span className="flex items-center gap-1">
                      <span className="h-3 w-3 animate-spin rounded-full border border-stone-400 border-t-transparent" />
                      Testing
                    </span>
                  ) : (
                    'Test'
                  )}
                </button>
              </div>
              {rpcUrlError ? (
                <p className="mt-2 text-xs text-red-600">{rpcUrlError}</p>
              ) : connectionTested ? (
                <p className="mt-2 text-xs text-green-600">Connection successful! URL saved.</p>
              ) : null}
              <div className="mt-3 flex gap-2">
                <button
                  type="button"
                  onClick={handleSaveRpcUrl}
                  className="px-3 py-1.5 bg-primary-500 hover:bg-primary-600 text-white text-xs font-medium rounded-lg transition-colors">
                  Save
                </button>
                <button
                  type="button"
                  onClick={handleResetRpcUrl}
                  className="px-3 py-1.5 bg-stone-200 hover:bg-stone-300 text-stone-700 text-xs font-medium rounded-lg transition-colors">
                  Reset to Default
                </button>
              </div>
            </div>
          ) : (
            <button
              type="button"
              onClick={() => setShowAdvanced(true)}
              className="mb-5 text-xs text-stone-500 hover:text-stone-700 underline">
              Configure RPC URL (Advanced)
            </button>
          )}

          {errorMessage ? (
            <div
              role="alert"
              className="mb-5 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700">
              {errorMessage}
            </div>
          ) : null}

          {isProcessing ? (
            <div
              role="status"
              aria-live="polite"
              aria-atomic="true"
              className="mb-5 flex flex-col items-center justify-center gap-3 py-2">
              <div className="h-6 w-6 animate-spin rounded-full border-2 border-stone-300 border-t-primary-500" />
              <p className="text-sm font-medium text-stone-700">Signing you in...</p>
            </div>
          ) : isSent ? (
            <div
              role="status"
              aria-live="polite"
              aria-atomic="true"
              className="flex flex-col items-center gap-3 py-2 text-center">
              <div className="flex h-12 w-12 items-center justify-center rounded-full bg-primary-50">
                <svg
                  className="h-6 w-6 text-primary-500"
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  strokeWidth={2}>
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
                  />
                </svg>
              </div>
              <p className="text-sm font-medium text-stone-900">Check your email</p>
              <p className="text-xs text-stone-500">
                We sent a sign-in link to{' '}
                <span className="font-medium text-stone-700">{email}</span>. Click it to continue.
              </p>
              <button
                type="button"
                onClick={() => {
                  setIsSent(false);
                  setEmail('');
                }}
                className="mt-1 text-xs text-primary-500 hover:underline">
                Use a different email
              </button>
            </div>
          ) : (
            <>
              {/* OAuth buttons — horizontal row */}
              <div className="flex items-center justify-center gap-3 mb-5">
                {oauthProviderConfigs
                  .filter(p => ['google', 'github', 'twitter'].includes(p.id))
                  .map(provider => (
                    <OAuthProviderButton
                      key={provider.id}
                      provider={provider}
                      className="!rounded-full !px-4 !py-2"
                    />
                  ))}
              </div>

              {/* Email login */}
              <div className="flex items-center gap-3 mb-5">
                <div className="flex-1 h-px bg-stone-200" />
                <span className="text-xs text-stone-400">Or</span>
                <div className="flex-1 h-px bg-stone-200" />
              </div>

              <form onSubmit={handleEmailSubmit} className="space-y-3">
                {emailError ? (
                  <div
                    role="alert"
                    className="rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-700">
                    {emailError}
                  </div>
                ) : null}
                <label htmlFor="email-login-input" className="sr-only">
                  Email address
                </label>
                <input
                  id="email-login-input"
                  type="email"
                  value={email}
                  onChange={e => setEmail(e.target.value)}
                  placeholder="Enter your email"
                  required
                  disabled={isSending}
                  className="w-full rounded-xl border border-stone-200 bg-white px-4 py-3 text-sm text-stone-900 placeholder:text-stone-400 outline-none focus:border-primary-500 focus:ring-1 focus:ring-primary-500 transition-colors disabled:opacity-60"
                />
                <button
                  type="submit"
                  disabled={isSending || !email.trim()}
                  className="w-full py-3 bg-primary-500 hover:bg-primary-600 text-white font-medium text-sm rounded-xl transition-colors duration-200 disabled:opacity-60 disabled:cursor-not-allowed flex items-center justify-center gap-2">
                  {isSending ? (
                    <span
                      role="status"
                      aria-live="polite"
                      aria-atomic="true"
                      className="flex items-center justify-center gap-2">
                      <div className="h-4 w-4 animate-spin rounded-full border-2 border-white/40 border-t-white" />
                      Sending link...
                    </span>
                  ) : (
                    'Continue with email'
                  )}
                </button>
              </form>
            </>
          )}
        </div>
      </div>
    </div>
  );
};

export default Welcome;

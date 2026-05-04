import { useEffect, useState } from 'react';

import OAuthProviderButton from '../components/oauth/OAuthProviderButton';
import { oauthProviderConfigs } from '../components/oauth/providerConfigs';
import RotatingTetrahedronCanvas from '../components/RotatingTetrahedronCanvas';
import { isEmailAuthAvailable, sendEmailMagicLink } from '../services/api/authApi';
import { clearCoreRpcUrlCache } from '../services/coreRpcClient';
import { useDeepLinkAuthState } from '../store/deepLinkAuthState';
import {
  clearStoredRpcUrl,
  getDefaultRpcUrl,
  getStoredRpcUrl,
  isValidRpcUrl,
  normalizeRpcUrl,
  storeRpcUrl,
} from '../utils/configPersistence';
import { isTauri } from '../utils/tauriCommands';

const Welcome = () => {
  const { isProcessing, errorMessage } = useDeepLinkAuthState();

  const [showAdvanced, setShowAdvanced] = useState(false);
  const [rpcUrl, setRpcUrl] = useState(getStoredRpcUrl());
  const [rpcUrlError, setRpcUrlError] = useState<string | null>(null);
  const [isTestingConnection, setIsTestingConnection] = useState(false);
  const [saveSuccess, setSaveSuccess] = useState(false);
  const [emailAuthAvailable, setEmailAuthAvailable] = useState<boolean | null>(null);
  const [email, setEmail] = useState('');
  const [emailError, setEmailError] = useState<string | null>(null);
  const [emailSuccess, setEmailSuccess] = useState<string | null>(null);
  const [isSendingEmail, setIsSendingEmail] = useState(false);

  useEffect(() => {
    let cancelled = false;

    const resolveEmailAuthAvailability = async () => {
      try {
        const available = await isEmailAuthAvailable();
        if (!cancelled) {
          setEmailAuthAvailable(available);
        }
      } catch (error) {
        console.debug('[welcome][email-auth] failed to resolve availability', error);
        if (!cancelled) {
          setEmailAuthAvailable(false);
        }
      }
    };

    resolveEmailAuthAvailability();

    return () => {
      cancelled = true;
    };
  }, []);

  const handleRpcUrlChange = (value: string) => {
    setRpcUrl(value);
    setRpcUrlError(null);
    setSaveSuccess(false);
  };

  const handleSaveRpcUrl = () => {
    const normalized = normalizeRpcUrl(rpcUrl);

    if (!isValidRpcUrl(normalized)) {
      setRpcUrlError('Please enter a valid HTTP or HTTPS URL');
      return;
    }

    storeRpcUrl(normalized);
    clearCoreRpcUrlCache();
    setRpcUrlError(null);
    setSaveSuccess(true);

    setTimeout(() => setSaveSuccess(false), 2000);
  };

  const handleResetRpcUrl = () => {
    clearStoredRpcUrl();
    clearCoreRpcUrlCache();
    setRpcUrl(getDefaultRpcUrl());
    setRpcUrlError(null);
    setSaveSuccess(false);
  };

  const handleTestConnection = async () => {
    const normalized = normalizeRpcUrl(rpcUrl);

    if (!isValidRpcUrl(normalized)) {
      setRpcUrlError('Please enter a valid HTTP or HTTPS URL');
      return;
    }

    setIsTestingConnection(true);
    setRpcUrlError(null);

    try {
      const response = await fetch(normalized, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'openhuman.ping', params: {} }),
      });

      if (response.ok || response.status === 405) {
        setSaveSuccess(true);
        storeRpcUrl(normalized);
        clearCoreRpcUrlCache();
      } else {
        setRpcUrlError(`Connection failed: ${response.status} ${response.statusText}`);
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unable to reach the RPC endpoint';
      setRpcUrlError(`Connection failed: ${message}`);
    } finally {
      setIsTestingConnection(false);
    }
  };

  const handleSendMagicLink = async () => {
    const trimmed = email.trim();
    if (!trimmed) {
      setEmailError('Please enter your email address.');
      setEmailSuccess(null);
      return;
    }

    setIsSendingEmail(true);
    setEmailError(null);
    setEmailSuccess(null);

    try {
      const frontendRedirectUri = isTauri() ? 'openhuman://' : window.location.origin;
      await sendEmailMagicLink(trimmed, frontendRedirectUri);
      setEmailSuccess('Magic link sent. Check your inbox to continue.');
    } catch (error) {
      setEmailError(error instanceof Error ? error.message : 'Failed to send magic link.');
    } finally {
      setIsSendingEmail(false);
    }
  };

  return (
    <div className="min-h-full flex flex-col items-center justify-center p-4">
      <div className="max-w-md w-full">
        <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-8 animate-fade-up">
          <div className="flex justify-center mb-6">
            <div className="h-20 w-20">
              <RotatingTetrahedronCanvas />
            </div>
          </div>

          <h1 className="text-2xl font-bold text-stone-900 text-center mb-2">
            Sign in! Let's Cook
          </h1>

          <p className="text-sm text-stone-500 text-center mb-6 leading-relaxed">
            Welcome to <span className="font-medium text-stone-900">OpenHuman</span>! Your Personal
            AI Super Intelligence. Private, Simple and extremely powerful.
          </p>

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
              ) : saveSuccess ? (
                <p className="mt-2 text-xs text-green-600">URL saved successfully.</p>
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
          ) : (
            <>
              {/* Real OAuth: click → system browser → backend → deep link back to app. */}
              <div className="flex items-center justify-center gap-3">
                {oauthProviderConfigs
                  .filter(provider => provider.showOnWelcome)
                  .map(provider => (
                    <OAuthProviderButton
                      key={provider.id}
                      provider={provider}
                      className="!rounded-full !px-4 !py-2"
                    />
                  ))}
              </div>
              {emailAuthAvailable === true ? (
                <div className="mt-6">
                  <div className="mb-3 flex items-center gap-3">
                    <div className="h-px flex-1 bg-stone-200" />
                    <p className="text-[11px] font-medium uppercase tracking-[0.14em] text-stone-400">
                      Or continue with email
                    </p>
                    <div className="h-px flex-1 bg-stone-200" />
                  </div>

                  <form
                    onSubmit={event => {
                      event.preventDefault();
                      void handleSendMagicLink();
                    }}
                    className="space-y-2">
                    <div className="rounded-xl border border-stone-200 bg-stone-50/70 p-3">
                      <label htmlFor="email-login-input" className="sr-only">
                        Email address
                      </label>
                      <div className="flex items-center gap-2 rounded-lg border border-stone-300 bg-white px-3 py-2 transition-colors focus-within:border-stone-500 focus-within:ring-1 focus-within:ring-stone-300">
                        <svg
                          aria-hidden="true"
                          viewBox="0 0 24 24"
                          className="h-4 w-4 text-stone-400"
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="1.8">
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            d="M4 6.75h16A1.25 1.25 0 0 1 21.25 8v8A1.25 1.25 0 0 1 20 17.25H4A1.25 1.25 0 0 1 2.75 16V8A1.25 1.25 0 0 1 4 6.75Z"
                          />
                          <path strokeLinecap="round" strokeLinejoin="round" d="m4 8 8 6 8-6" />
                        </svg>
                        <input
                          id="email-login-input"
                          type="email"
                          autoComplete="email"
                          value={email}
                          onChange={e => {
                            setEmail(e.target.value);
                            setEmailError(null);
                            setEmailSuccess(null);
                          }}
                          placeholder="you@example.com"
                          className="w-full appearance-none border-0 bg-transparent p-0 text-sm text-stone-900 placeholder:text-stone-400 focus:!border-0 focus:!outline-none focus:!ring-0 focus:!shadow-none"
                        />
                      </div>

                      <p className="mt-2 text-xs text-stone-500">
                        We will send a secure magic link. No password required.
                      </p>
                    </div>

                    <button
                      type="submit"
                      disabled={isSendingEmail}
                      className="w-full rounded-lg bg-stone-900 px-4 py-2.5 text-sm font-medium text-white transition-colors hover:bg-stone-700 disabled:cursor-not-allowed disabled:opacity-60">
                      {isSendingEmail ? 'Sending...' : 'Continue with email'}
                    </button>
                  </form>

                  {emailError ? <p className="mt-2 text-xs text-red-600">{emailError}</p> : null}
                  {emailSuccess ? (
                    <p className="mt-2 text-xs text-green-600">{emailSuccess}</p>
                  ) : null}
                </div>
              ) : null}
            </>
          )}
        </div>
      </div>
    </div>
  );
};

export default Welcome;

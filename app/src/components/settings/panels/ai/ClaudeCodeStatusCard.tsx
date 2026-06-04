import { useCallback, useEffect, useState } from 'react';

import {
  type ClaudeCodeAuthStatus,
  type ClaudeCodeStatus,
  openhumanClaudeCodeAuthStatus,
  openhumanClaudeCodeLoginLaunch,
  openhumanClaudeCodeStatus,
} from '../../../../utils/tauriCommands/config';

/**
 * Status card for the Claude Code CLI provider.
 *
 * Surfaces two independent probes:
 *   1. Binary install + version (slow — spawns `claude --version`).
 *   2. Auth state — Pro/Max subscription via `~/.claude/.credentials.json`
 *      or `ANTHROPIC_API_KEY` env (fast — pure FS).
 *
 * Each refreshes independently so a user who just ran `claude login` can
 * re-probe auth without re-spawning the binary.
 */
export function ClaudeCodeStatusCard() {
  const [status, setStatus] = useState<ClaudeCodeStatus | null>(null);
  const [auth, setAuth] = useState<ClaudeCodeAuthStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [authError, setAuthError] = useState<string | null>(null);
  const [loading, setLoading] = useState<boolean>(false);
  const [authLoading, setAuthLoading] = useState<boolean>(false);

  const probe = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const resp = await openhumanClaudeCodeStatus();
      setStatus(resp.result);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setStatus(null);
    } finally {
      setLoading(false);
    }
  }, []);

  const probeAuth = useCallback(async () => {
    setAuthLoading(true);
    setAuthError(null);
    try {
      const resp = await openhumanClaudeCodeAuthStatus();
      setAuth(resp.result);
    } catch (err) {
      setAuthError(err instanceof Error ? err.message : String(err));
      setAuth(null);
    } finally {
      setAuthLoading(false);
    }
  }, []);

  useEffect(() => {
    void probe();
    void probeAuth();
  }, [probe, probeAuth]);

  return (
    <section
      data-testid="claude-code-status-card"
      className="rounded-lg border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-900">
      <header className="mb-2 flex items-center justify-between">
        <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
          Claude Code CLI
        </h3>
        <button
          type="button"
          onClick={() => {
            void probe();
          }}
          disabled={loading}
          className="text-xs text-neutral-500 hover:text-neutral-900 disabled:opacity-50 dark:text-neutral-400 dark:hover:text-neutral-100">
          {loading ? 'Probing…' : 'Probe'}
        </button>
      </header>
      <StatusBody status={status} error={error} />

      <div className="mt-4 border-t border-neutral-200 pt-3 dark:border-neutral-800">
        <header className="mb-2 flex items-center justify-between">
          <h4 className="text-xs font-semibold uppercase tracking-wide text-neutral-500 dark:text-neutral-400">
            Authentication
          </h4>
          <button
            type="button"
            onClick={() => {
              void probeAuth();
            }}
            disabled={authLoading}
            className="text-xs text-neutral-500 hover:text-neutral-900 disabled:opacity-50 dark:text-neutral-400 dark:hover:text-neutral-100">
            {authLoading ? 'Checking…' : 'Recheck'}
          </button>
        </header>
        <AuthBody auth={auth} error={authError} />
      </div>

      <p className="mt-3 text-xs text-neutral-500 dark:text-neutral-400">
        Use the <code>claude-code:&lt;model&gt;</code> provider string to route chat, agentic, or
        reasoning workloads through your local Claude Code CLI install.
      </p>
    </section>
  );
}

function StatusBody({ status, error }: { status: ClaudeCodeStatus | null; error: string | null }) {
  if (error) {
    return <p className="text-xs text-rose-600 dark:text-rose-400">Failed to probe: {error}</p>;
  }
  if (!status) {
    return <p className="text-xs text-neutral-500 dark:text-neutral-400">Probing…</p>;
  }
  switch (status.status) {
    case 'ok':
      return (
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
          <dt className="text-neutral-500">Status</dt>
          <dd className="text-emerald-600 dark:text-emerald-400">Installed ({status.version})</dd>
          <dt className="text-neutral-500">Path</dt>
          <dd className="font-mono text-neutral-700 dark:text-neutral-300">{status.path}</dd>
        </dl>
      );
    case 'not_installed':
      return (
        <p className="text-xs text-amber-600 dark:text-amber-400">
          Claude Code CLI is not installed. Install via{' '}
          <code>npm install -g @anthropic-ai/claude-code</code> or follow{' '}
          <a
            href="https://docs.anthropic.com/en/docs/claude-code"
            target="_blank"
            rel="noreferrer noopener"
            className="underline hover:text-amber-700 dark:hover:text-amber-300">
            Anthropic's docs
          </a>
          .
        </p>
      );
    case 'outdated':
      return (
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
          <dt className="text-neutral-500">Status</dt>
          <dd className="text-rose-600 dark:text-rose-400">
            Outdated — found {status.version}, need ≥ {status.min_required}
          </dd>
          <dt className="text-neutral-500">Path</dt>
          <dd className="font-mono text-neutral-700 dark:text-neutral-300">{status.path}</dd>
        </dl>
      );
    case 'unusable':
      return (
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
          <dt className="text-neutral-500">Status</dt>
          <dd className="text-rose-600 dark:text-rose-400">Unusable — {status.reason}</dd>
          <dt className="text-neutral-500">Path</dt>
          <dd className="font-mono text-neutral-700 dark:text-neutral-300">{status.path}</dd>
        </dl>
      );
  }
}

function AuthBody({ auth, error }: { auth: ClaudeCodeAuthStatus | null; error: string | null }) {
  if (error) {
    return <p className="text-xs text-rose-600 dark:text-rose-400">Failed to check: {error}</p>;
  }
  if (!auth) {
    return <p className="text-xs text-neutral-500 dark:text-neutral-400">Checking…</p>;
  }
  if (auth.source === 'subscription') {
    return (
      <div className="space-y-1">
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
          <dt className="text-neutral-500">Signed in</dt>
          <dd className="text-emerald-600 dark:text-emerald-400">
            {auth.account_email ?? 'Claude subscription'}
          </dd>
          {auth.expires_at && (
            <>
              <dt className="text-neutral-500">Token expires</dt>
              <dd className="font-mono text-neutral-700 dark:text-neutral-300">
                {auth.expires_at}
              </dd>
            </>
          )}
        </dl>
        <p className="text-xs text-neutral-500 dark:text-neutral-400">
          To sign out, run <code>claude logout</code> in your terminal, then click Recheck.
        </p>
      </div>
    );
  }
  if (auth.source === 'api_key_env') {
    return (
      <p className="text-xs text-emerald-600 dark:text-emerald-400">
        <code>ANTHROPIC_API_KEY</code> detected in environment.
      </p>
    );
  }
  return <SignedOut />;
}

function SignedOut() {
  const [launchError, setLaunchError] = useState<string | null>(null);
  const [launching, setLaunching] = useState(false);

  const launchLogin = async () => {
    setLaunching(true);
    setLaunchError(null);
    try {
      await openhumanClaudeCodeLoginLaunch();
    } catch (err) {
      setLaunchError(err instanceof Error ? err.message : String(err));
    } finally {
      setLaunching(false);
    }
  };

  return (
    <div className="space-y-2">
      <p className="text-xs text-amber-600 dark:text-amber-400">Not signed in.</p>
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          onClick={() => {
            void launchLogin();
          }}
          disabled={launching}
          className="rounded-md bg-neutral-900 px-2.5 py-1 text-xs font-medium text-white hover:bg-neutral-700 disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-300">
          {launching ? 'Opening terminal…' : 'Sign in with Claude'}
        </button>
        <span className="text-xs text-neutral-500 dark:text-neutral-400">
          Opens a terminal running <code>claude login</code>.
        </span>
      </div>
      {launchError && <p className="text-xs text-rose-600 dark:text-rose-400">{launchError}</p>}
      <p className="text-xs text-neutral-500 dark:text-neutral-400">
        After completing login, click <strong>Recheck</strong> above. Alternatively set{' '}
        <code>ANTHROPIC_API_KEY</code> to use an API key.
      </p>
    </div>
  );
}

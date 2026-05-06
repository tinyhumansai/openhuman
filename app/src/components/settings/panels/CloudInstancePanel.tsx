import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  type DeploymentInstance,
  deploymentsApi,
  type DeploymentStatus,
  type ProvisionParams,
} from '../../../services/api/deploymentsApi';
import { clearCoreRpcTokenCache, clearCoreRpcUrlCache } from '../../../services/coreRpcClient';
import { buildRpcEndpoint } from '../../../utils/configPersistence';
import {
  clearCoreToken,
  clearStoredRpcUrl,
  storeCoreToken,
  storeRpcUrl,
} from '../../../utils/configPersistence';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const log = createDebug('openhuman:cloud-instance-panel');

// Minimum IAM permissions required for BYOC deployment
const IAM_POLICY = JSON.stringify(
  {
    Version: '2012-10-17',
    Statement: [
      {
        Effect: 'Allow',
        Action: [
          'ec2:RunInstances',
          'ec2:TerminateInstances',
          'ec2:DescribeInstances',
          'ec2:DescribeInstanceStatus',
          'ec2:AllocateAddress',
          'ec2:ReleaseAddress',
          'ec2:AssociateAddress',
          'ec2:DisassociateAddress',
          'ec2:CreateSecurityGroup',
          'ec2:DeleteSecurityGroup',
          'ec2:AuthorizeSecurityGroupIngress',
          'ec2:CreateTags',
          'ec2:CreateVolume',
          'ec2:DeleteVolume',
          'ec2:AttachVolume',
          'ec2:DescribeVolumes',
          'ec2:DescribeAddresses',
          'ec2:DescribeSecurityGroups',
        ],
        Resource: '*',
      },
    ],
  },
  null,
  2
);

const AWS_REGIONS = [
  { value: 'us-east-1', label: 'US East (N. Virginia)' },
  { value: 'us-west-2', label: 'US West (Oregon)' },
  { value: 'eu-west-1', label: 'EU West (Ireland)' },
  { value: 'eu-central-1', label: 'EU Central (Frankfurt)' },
  { value: 'ap-southeast-1', label: 'Asia Pacific (Singapore)' },
];

const STATUS_LABELS: Record<DeploymentStatus, string> = {
  pending: 'Pending...',
  provisioning: 'Provisioning AWS resources...',
  deploying: 'Pulling container image...',
  starting: 'Starting core service...',
  active: 'Active',
  unhealthy: 'Unhealthy',
  terminating: 'Terminating...',
  terminated: 'Terminated',
  failed: 'Failed',
};

const IN_PROGRESS_STATUSES: DeploymentStatus[] = [
  'pending',
  'provisioning',
  'deploying',
  'starting',
];

type ViewState = 'loading' | 'no-instance' | 'in-progress' | 'active' | 'failed';

const CloudInstancePanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  const [viewState, setViewState] = useState<ViewState>('loading');
  const [instance, setInstance] = useState<DeploymentInstance | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Form state
  const [accessKeyId, setAccessKeyId] = useState('');
  const [secretAccessKey, setSecretAccessKey] = useState('');
  const [showSecret, setShowSecret] = useState(false);
  const [region, setRegion] = useState('us-east-1');
  const [domain, setDomain] = useState('');
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [policyExpanded, setPolicyExpanded] = useState(false);
  const [policyCopied, setPolicyCopied] = useState(false);

  // Terminate confirm
  const [showTerminateConfirm, setShowTerminateConfirm] = useState(false);

  // Countdown for estimated ready time
  const [secondsRemaining, setSecondsRemaining] = useState<number | null>(null);
  const countdownRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Current mode (remote vs local)
  const [isRemoteMode, setIsRemoteMode] = useState(false);

  const resolveViewState = useCallback((dep: DeploymentInstance | null) => {
    if (!dep) {
      setViewState('no-instance');
      return;
    }
    if (IN_PROGRESS_STATUSES.includes(dep.status)) {
      setViewState('in-progress');
      return;
    }
    if (dep.status === 'active') {
      setViewState('active');
      return;
    }
    if (dep.status === 'failed' || dep.status === 'unhealthy') {
      setViewState('failed');
      return;
    }
    // terminated / terminating → show no-instance
    setViewState('no-instance');
  }, []);

  const loadStatus = useCallback(async () => {
    log('[deployment] loading deployment status');
    try {
      const res = await deploymentsApi.getStatus();
      const dep = res.data;
      setInstance(dep);
      setError(null);
      resolveViewState(dep);
    } catch (err) {
      log('[deployment] failed to load status: %O', err);
      // Non-fatal — keep showing current view
      setError(err instanceof Error ? err.message : 'Failed to load deployment status');
      setViewState('no-instance');
    }
  }, [resolveViewState]);

  useEffect(() => {
    void loadStatus();
  }, [loadStatus]);

  // Poll while in progress
  useEffect(() => {
    if (viewState !== 'in-progress') {
      if (countdownRef.current) {
        clearInterval(countdownRef.current);
        countdownRef.current = null;
      }
      return;
    }

    const pollInterval = setInterval(async () => {
      log('[deployment] polling status');
      try {
        const res = await deploymentsApi.getStatus();
        const dep = res.data;
        setInstance(dep);
        if (dep && !IN_PROGRESS_STATUSES.includes(dep.status)) {
          resolveViewState(dep);
        }
      } catch {
        // Polling failures are non-fatal
      }
    }, 5000);

    // Countdown timer
    if (secondsRemaining !== null && secondsRemaining > 0) {
      countdownRef.current = setInterval(() => {
        setSecondsRemaining(prev => (prev !== null && prev > 0 ? prev - 1 : 0));
      }, 1000);
    }

    return () => {
      clearInterval(pollInterval);
      if (countdownRef.current) {
        clearInterval(countdownRef.current);
        countdownRef.current = null;
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewState, resolveViewState]);

  const handleDeploy = async () => {
    if (!accessKeyId.trim() || !secretAccessKey.trim()) return;

    setIsSubmitting(true);
    setError(null);
    log('[deployment] initiating provision: region=%s', region);

    try {
      const params: ProvisionParams = {
        awsAccessKeyId: accessKeyId.trim(),
        awsSecretAccessKey: secretAccessKey.trim(),
        awsRegion: region,
      };
      if (domain.trim()) {
        params.domain = domain.trim();
      }

      const res = await deploymentsApi.provision(params);
      const data = res.data;
      log('[deployment] provision started: deploymentId=%s', data.deploymentId);
      setSecondsRemaining(data.estimatedReadySeconds);
      setViewState('in-progress');
      // Refresh to get the full instance record
      await loadStatus();
    } catch (err) {
      log('[deployment] provision failed: %O', err);
      setError(err instanceof Error ? err.message : 'Failed to start deployment');
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleSwitchToRemote = async () => {
    if (!instance?.url) return;

    log('[deployment] switching to remote core: url=%s', instance.url);
    try {
      // Fetch coreToken from backend /auth/me (user.coreToken).
      // Gracefully degrades if backend hasn't been updated yet — the stored
      // URL is still set so the next RPC call reaches the remote instance,
      // and the Tauri-managed token is used as fallback.
      const coreToken = await deploymentsApi.getCoreToken();
      if (coreToken) {
        storeCoreToken(coreToken);
        log('[deployment] stored core token for remote connection');
      }

      // Build the /rpc endpoint URL from the deployment URL.
      const rpcUrl = buildRpcEndpoint(instance.url);
      storeRpcUrl(rpcUrl);
      clearCoreRpcUrlCache();
      clearCoreRpcTokenCache();
      setIsRemoteMode(true);
      log('[deployment] switched to remote core: rpcUrl=%s', rpcUrl);
    } catch (err) {
      log('[deployment] failed to switch to remote: %O', err);
      setError(err instanceof Error ? err.message : 'Failed to switch to remote core');
    }
  };

  const handleSwitchToLocal = () => {
    log('[deployment] switching back to local core');
    clearCoreToken();
    clearStoredRpcUrl();
    clearCoreRpcUrlCache();
    clearCoreRpcTokenCache();
    setIsRemoteMode(false);
    log('[deployment] switched to local core');
  };

  const handleTerminate = async () => {
    log('[deployment] terminating deployment');
    setShowTerminateConfirm(false);

    try {
      await deploymentsApi.terminate();
      // Switch back to local if we were in remote mode
      if (isRemoteMode) {
        handleSwitchToLocal();
      }
      setInstance(null);
      setViewState('no-instance');
      log('[deployment] termination initiated');
    } catch (err) {
      log('[deployment] termination failed: %O', err);
      setError(err instanceof Error ? err.message : 'Failed to terminate deployment');
    }
  };

  const handleCopyPolicy = async () => {
    try {
      await navigator.clipboard.writeText(IAM_POLICY);
      setPolicyCopied(true);
      setTimeout(() => setPolicyCopied(false), 2000);
    } catch {
      // Fallback: select the text
    }
  };

  const handleRetry = () => {
    setViewState('no-instance');
    setError(null);
  };

  return (
    <div data-testid="settings-cloud-instance-panel">
      <SettingsHeader
        title="Cloud Instance"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        {/* Current mode indicator */}
        <div className="flex items-center gap-2 text-sm">
          <span
            className={`w-2 h-2 rounded-full flex-shrink-0 ${isRemoteMode ? 'bg-primary-500' : 'bg-sage-400'}`}
          />
          <span className="text-stone-600 text-xs">
            {isRemoteMode ? 'Connected to cloud core' : 'Using local core'}
          </span>
        </div>

        {/* Error banner */}
        {error && (
          <div className="rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-700">
            {error}
          </div>
        )}

        {/* Loading */}
        {viewState === 'loading' && (
          <div className="text-center py-10 text-sm text-stone-400">Loading...</div>
        )}

        {/* View A — No active deployment */}
        {viewState === 'no-instance' && (
          <div className="space-y-4">
            <div className="rounded-xl border border-stone-200 bg-white p-5">
              <h3 className="text-sm font-semibold text-stone-900">Deploy Cloud Instance</h3>
              <p className="mt-1 text-xs text-stone-500">
                Run your OpenHuman core 24/7 on your own AWS account. You pay your own AWS bill
                (~$15-18/month for a t3.small instance).
              </p>
            </div>

            {/* IAM Policy */}
            <div className="rounded-xl border border-stone-200 bg-white p-5 space-y-3">
              <div className="flex items-center justify-between">
                <h4 className="text-xs font-semibold text-stone-700">
                  Step 1: Create an AWS IAM user
                </h4>
                <button
                  type="button"
                  onClick={() => setPolicyExpanded(v => !v)}
                  className="text-xs text-primary-600 hover:text-primary-700">
                  {policyExpanded ? 'Hide policy' : 'Show IAM policy'}
                </button>
              </div>
              <ol className="text-xs text-stone-600 space-y-1 list-decimal list-inside">
                <li>Go to AWS IAM Console and create a new user</li>
                <li>Attach the policy below (inline or as a managed policy)</li>
                <li>Create an access key for the user</li>
                <li>Paste the key ID and secret below</li>
              </ol>
              {policyExpanded && (
                <div className="relative">
                  <pre className="rounded-lg bg-stone-900 p-3 text-[10px] text-stone-300 overflow-x-auto leading-relaxed">
                    {IAM_POLICY}
                  </pre>
                  <button
                    type="button"
                    onClick={() => void handleCopyPolicy()}
                    className="absolute top-2 right-2 rounded-md bg-stone-700 px-2 py-1 text-[10px] text-stone-300 hover:bg-stone-600 transition-colors">
                    {policyCopied ? 'Copied!' : 'Copy'}
                  </button>
                </div>
              )}
            </div>

            {/* Credentials form */}
            <div className="rounded-xl border border-stone-200 bg-white p-5 space-y-3">
              <h4 className="text-xs font-semibold text-stone-700">Step 2: Enter credentials</h4>

              {/* AWS Access Key ID */}
              <div>
                <label className="block text-xs font-medium text-stone-700 mb-1">
                  AWS Access Key ID
                </label>
                <input
                  type="text"
                  value={accessKeyId}
                  onChange={e => setAccessKeyId(e.target.value)}
                  placeholder="AKIAIOSFODNN7EXAMPLE"
                  autoComplete="off"
                  spellCheck={false}
                  className="w-full rounded-lg border border-stone-300 px-3 py-2 text-sm font-mono focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-primary-500"
                />
              </div>

              {/* AWS Secret Access Key */}
              <div>
                <label className="block text-xs font-medium text-stone-700 mb-1">
                  AWS Secret Access Key
                </label>
                <div className="relative">
                  <input
                    type={showSecret ? 'text' : 'password'}
                    value={secretAccessKey}
                    onChange={e => setSecretAccessKey(e.target.value)}
                    placeholder="wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
                    autoComplete="new-password"
                    spellCheck={false}
                    className="w-full rounded-lg border border-stone-300 px-3 py-2 pr-10 text-sm font-mono focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-primary-500"
                  />
                  <button
                    type="button"
                    onClick={() => setShowSecret(v => !v)}
                    className="absolute right-3 top-1/2 -translate-y-1/2 text-stone-400 hover:text-stone-600">
                    {showSecret ? (
                      <svg
                        className="w-4 h-4"
                        fill="none"
                        stroke="currentColor"
                        viewBox="0 0 24 24">
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21"
                        />
                      </svg>
                    ) : (
                      <svg
                        className="w-4 h-4"
                        fill="none"
                        stroke="currentColor"
                        viewBox="0 0 24 24">
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
                        />
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"
                        />
                      </svg>
                    )}
                  </button>
                </div>
              </div>

              {/* AWS Region */}
              <div>
                <label className="block text-xs font-medium text-stone-700 mb-1">AWS Region</label>
                <select
                  value={region}
                  onChange={e => setRegion(e.target.value)}
                  className="w-full rounded-lg border border-stone-300 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-primary-500 bg-white">
                  {AWS_REGIONS.map(r => (
                    <option key={r.value} value={r.value}>
                      {r.label}
                    </option>
                  ))}
                </select>
              </div>

              {/* Custom domain (optional) */}
              <div>
                <label className="block text-xs font-medium text-stone-700 mb-1">
                  Custom Domain <span className="font-normal text-stone-400">(optional)</span>
                </label>
                <input
                  type="text"
                  value={domain}
                  onChange={e => setDomain(e.target.value)}
                  placeholder="core.yourdomain.com — leave blank to use IP"
                  className="w-full rounded-lg border border-stone-300 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-primary-500"
                />
                <p className="mt-1 text-[10px] text-stone-400">
                  If left blank, an Elastic IP address with Caddy and nip.io is used — no DNS setup
                  required.
                </p>
              </div>

              {/* Deploy button */}
              <button
                type="button"
                onClick={() => void handleDeploy()}
                disabled={isSubmitting || !accessKeyId.trim() || !secretAccessKey.trim()}
                className="w-full rounded-lg bg-primary-600 py-2.5 px-4 text-sm font-medium text-white hover:bg-primary-700 disabled:cursor-not-allowed disabled:opacity-50 transition-colors">
                {isSubmitting ? 'Starting deployment...' : 'Deploy'}
              </button>

              <p className="text-[10px] text-stone-400 text-center">
                Your AWS credentials are sent securely and used only to provision your instance.
              </p>
            </div>
          </div>
        )}

        {/* View B — Deployment in progress */}
        {viewState === 'in-progress' && instance && (
          <div className="rounded-xl border border-stone-200 bg-white p-5 space-y-4">
            <div className="flex items-center gap-3">
              {/* Spinner */}
              <svg
                className="w-5 h-5 animate-spin text-primary-500 flex-shrink-0"
                xmlns="http://www.w3.org/2000/svg"
                fill="none"
                viewBox="0 0 24 24">
                <circle
                  className="opacity-25"
                  cx="12"
                  cy="12"
                  r="10"
                  stroke="currentColor"
                  strokeWidth="4"
                />
                <path
                  className="opacity-75"
                  fill="currentColor"
                  d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                />
              </svg>
              <h3 className="text-sm font-semibold text-stone-900">
                {STATUS_LABELS[instance.status] ?? 'Setting up...'}
              </h3>
            </div>

            {/* Step breakdown */}
            <div className="space-y-1.5">
              {(['pending', 'provisioning', 'deploying', 'starting'] as const).map(step => {
                const statuses: DeploymentStatus[] = [
                  'pending',
                  'provisioning',
                  'deploying',
                  'starting',
                ];
                const currentIdx = statuses.indexOf(instance.status);
                const stepIdx = statuses.indexOf(step);
                const isCompleted = stepIdx < currentIdx;
                const isCurrent = stepIdx === currentIdx;
                return (
                  <div key={step} className="flex items-center gap-2 text-xs">
                    <span
                      className={`w-4 h-4 rounded-full flex items-center justify-center flex-shrink-0 text-[10px] font-bold
                        ${isCompleted ? 'bg-sage-500 text-white' : isCurrent ? 'bg-primary-500 text-white' : 'bg-stone-200 text-stone-400'}`}>
                      {isCompleted ? '✓' : stepIdx + 1}
                    </span>
                    <span
                      className={
                        isCurrent
                          ? 'text-stone-900 font-medium'
                          : isCompleted
                            ? 'text-stone-400 line-through'
                            : 'text-stone-400'
                      }>
                      {STATUS_LABELS[step]}
                    </span>
                  </div>
                );
              })}
            </div>

            {secondsRemaining !== null && secondsRemaining > 0 && (
              <p className="text-xs text-stone-400">
                Estimated time remaining:{' '}
                <span className="font-medium text-stone-600">
                  {Math.floor(secondsRemaining / 60)}m {secondsRemaining % 60}s
                </span>
              </p>
            )}

            <p className="text-[10px] text-stone-400">
              EC2 t3.small instances typically take 60-90 seconds to boot and pull the image.
            </p>
          </div>
        )}

        {/* View C — Active deployment */}
        {viewState === 'active' && instance && (
          <div className="space-y-4">
            <div className="rounded-xl border border-stone-200 bg-white p-5 space-y-4">
              <div className="flex items-center justify-between">
                <h3 className="text-sm font-semibold text-stone-900">Cloud Instance</h3>
                <span className="inline-flex items-center gap-1.5 rounded-full border border-sage-200 bg-sage-50 px-2 py-0.5 text-[10px] font-medium text-sage-700">
                  <span className="w-1.5 h-1.5 rounded-full bg-sage-500" />
                  Connected
                </span>
              </div>

              <div className="space-y-1.5 text-xs text-stone-600">
                <div>
                  <span className="font-medium text-stone-700">URL:</span>{' '}
                  <span className="font-mono text-[11px]">{instance.url ?? '—'}</span>
                </div>
                <div>
                  <span className="font-medium text-stone-700">Region:</span> {instance.region}
                </div>
                <div>
                  <span className="font-medium text-stone-700">Image:</span> {instance.imageTag}
                </div>
                <div>
                  <span className="font-medium text-stone-700">Deployed:</span>{' '}
                  {new Date(instance.createdAt).toLocaleDateString(undefined, {
                    year: 'numeric',
                    month: 'short',
                    day: 'numeric',
                  })}
                </div>
                {instance.activatedAt && (
                  <div>
                    <span className="font-medium text-stone-700">Activated:</span>{' '}
                    {new Date(instance.activatedAt).toLocaleDateString(undefined, {
                      year: 'numeric',
                      month: 'short',
                      day: 'numeric',
                    })}
                  </div>
                )}
              </div>

              {/* Switch buttons */}
              <div className="flex gap-2">
                {!isRemoteMode ? (
                  <button
                    type="button"
                    onClick={() => void handleSwitchToRemote()}
                    className="flex-1 rounded-lg bg-primary-600 py-2 px-3 text-xs font-medium text-white hover:bg-primary-700 transition-colors">
                    Switch to Remote
                  </button>
                ) : (
                  <button
                    type="button"
                    onClick={handleSwitchToLocal}
                    className="flex-1 rounded-lg bg-stone-100 py-2 px-3 text-xs font-medium text-stone-700 hover:bg-stone-200 transition-colors">
                    Switch to Local
                  </button>
                )}
              </div>
            </div>

            {/* Terminate section */}
            <div className="rounded-xl border border-red-100 bg-white p-5 space-y-3">
              <h4 className="text-xs font-semibold text-stone-700">Danger Zone</h4>
              <p className="text-xs text-stone-500">
                Terminating permanently deletes your cloud workspace. All memory, config, and
                conversation history stored on the instance will be gone.
              </p>
              {!showTerminateConfirm ? (
                <button
                  type="button"
                  onClick={() => setShowTerminateConfirm(true)}
                  className="w-full rounded-lg border border-red-200 bg-white py-2 px-3 text-xs font-medium text-red-600 hover:bg-red-50 transition-colors">
                  Terminate Instance
                </button>
              ) : (
                <div className="space-y-2">
                  <p className="text-xs font-medium text-red-700 bg-red-50 rounded-lg p-3">
                    This will permanently delete your cloud workspace data including memory and
                    conversation history. This cannot be undone.
                  </p>
                  <div className="flex gap-2">
                    <button
                      type="button"
                      onClick={() => setShowTerminateConfirm(false)}
                      className="flex-1 rounded-lg border border-stone-200 bg-white py-2 px-3 text-xs font-medium text-stone-600 hover:bg-stone-50 transition-colors">
                      Cancel
                    </button>
                    <button
                      type="button"
                      onClick={() => void handleTerminate()}
                      className="flex-1 rounded-lg bg-red-600 py-2 px-3 text-xs font-medium text-white hover:bg-red-700 transition-colors">
                      Confirm Terminate
                    </button>
                  </div>
                </div>
              )}
            </div>
          </div>
        )}

        {/* View D — Failed deployment */}
        {viewState === 'failed' && (
          <div className="rounded-xl border border-red-200 bg-white p-5 space-y-4">
            <h3 className="text-sm font-semibold text-red-700">Deployment Failed</h3>
            {instance?.failureReason && (
              <p className="rounded-lg bg-red-50 px-3 py-2 font-mono text-[11px] text-red-700 whitespace-pre-wrap">
                {instance.failureReason}
              </p>
            )}
            <div className="flex gap-2">
              <button
                type="button"
                onClick={handleRetry}
                className="flex-1 rounded-lg bg-primary-600 py-2 px-3 text-xs font-medium text-white hover:bg-primary-700 transition-colors">
                Retry
              </button>
              {instance && (
                <button
                  type="button"
                  onClick={() => setShowTerminateConfirm(true)}
                  className="flex-1 rounded-lg border border-red-200 bg-white py-2 px-3 text-xs font-medium text-red-600 hover:bg-red-50 transition-colors">
                  Clean Up
                </button>
              )}
            </div>
            {showTerminateConfirm && (
              <div className="space-y-2">
                <p className="text-xs font-medium text-red-700 bg-red-50 rounded-lg p-3">
                  This will attempt to clean up any partially provisioned AWS resources. Cloud
                  workspace data (if any) will be deleted.
                </p>
                <div className="flex gap-2">
                  <button
                    type="button"
                    onClick={() => setShowTerminateConfirm(false)}
                    className="flex-1 rounded-lg border border-stone-200 bg-white py-2 px-3 text-xs font-medium text-stone-600 hover:bg-stone-50 transition-colors">
                    Cancel
                  </button>
                  <button
                    type="button"
                    onClick={() => void handleTerminate()}
                    className="flex-1 rounded-lg bg-red-600 py-2 px-3 text-xs font-medium text-white hover:bg-red-700 transition-colors">
                    Confirm
                  </button>
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
};

export default CloudInstancePanel;

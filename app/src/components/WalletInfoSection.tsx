import { useCallback, useEffect, useRef, useState } from 'react';

import { useUser } from '../hooks/useUser';
import { useSkillConnectionStatus } from '../lib/skills/hooks';
import { skillManager } from '../lib/skills/manager';
import { useCoreState } from '../providers/CoreStateProvider';
import { buildManualSentryEvent, enqueueError } from '../services/errorReportQueue';

function truncateAddress(address: string): string {
  if (!address || address.length < 12) return address;
  return `${address.slice(0, 6)}…${address.slice(-4)}`;
}

export default function WalletInfoSection() {
  const walletStatus = useSkillConnectionStatus('wallet');
  const { user } = useUser();
  const { snapshot } = useCoreState();
  const primaryAddress = user?._id ? snapshot.localState.primaryWalletAddress : undefined;

  const [networkName, setNetworkName] = useState<string | null>(null);
  const [balance, setBalance] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isConnected = walletStatus === 'connected' && !!primaryAddress;
  const cancelledRef = useRef(false);
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const fetchWalletInfoRef = useRef<(address: string, attempt?: number) => Promise<void>>(null!);

  const fetchWalletInfo = useCallback(async (address: string, attempt = 0): Promise<void> => {
    if (cancelledRef.current) return;

    if (!skillManager.isSkillRunning('wallet')) {
      if (attempt < 5 && !cancelledRef.current) {
        retryTimerRef.current = setTimeout(
          () => fetchWalletInfoRef.current(address, attempt + 1),
          1500
        );
        return;
      }
      if (!cancelledRef.current) {
        setLoading(false);
        setNetworkName(null);
        setBalance(null);
        setError(null);
      }
      return;
    }

    try {
      // Wallet skill only supports Ethereum Mainnet (chain_id "1")
      const listRes = await skillManager.callTool('wallet', 'list_networks', {});
      if (cancelledRef.current) return;
      const listText = listRes.content?.[0]?.text;
      if (!listText || listRes.isError) {
        if (!cancelledRef.current) {
          setError('Could not load networks');
          setLoading(false);
        }
        return;
      }
      const listData = JSON.parse(listText) as {
        networks?: Array<{ chain_id?: string; name?: string; chain_type?: string }>;
      };
      const networks = Array.isArray(listData.networks) ? listData.networks : [];
      // Prefer Ethereum Mainnet (skill always has it); else first EVM, else first network
      const ethMainnet = networks.find(n => n?.chain_id === '1' && n?.chain_type === 'evm');
      const firstEvm = networks.find(n => n && n.chain_type === 'evm');
      const chosen = ethMainnet ?? firstEvm ?? networks.find(Boolean);
      if (!chosen || cancelledRef.current) {
        if (!cancelledRef.current) setLoading(false);
        return;
      }

      const networkNameVal = chosen.name ?? chosen.chain_id ?? 'Unknown';
      if (!cancelledRef.current) setNetworkName(networkNameVal);

      const chainId = chosen.chain_id ?? '';
      if (!chainId) {
        if (!cancelledRef.current) {
          setBalance('—');
          setLoading(false);
        }
        return;
      }

      const balanceRes = await skillManager.callTool('wallet', 'get_balance', {
        address,
        chain_id: chainId,
        chain_type: chosen.chain_type ?? 'evm',
      });
      if (cancelledRef.current) return;
      const balanceText = balanceRes.content?.[0]?.text;
      if (!balanceText || balanceRes.isError) {
        if (!cancelledRef.current) {
          setBalance('—');
          setLoading(false);
        }
        return;
      }
      const balanceData = JSON.parse(balanceText) as {
        balance_eth?: string;
        symbol?: string;
        error?: string;
      };
      if (balanceData.error) {
        if (!cancelledRef.current) {
          setBalance('—');
          setLoading(false);
        }
        return;
      }
      const eth = balanceData.balance_eth ?? '0';
      const symbol = balanceData.symbol ?? 'ETH';
      const parsed = parseFloat(eth);
      const value = Number.isFinite(parsed) ? parsed : 0;
      const display = value < 0.0001 ? '0' : value.toFixed(4);
      if (!cancelledRef.current) {
        setBalance(`${display} ${symbol}`);
        setLoading(false);
      }
    } catch (e) {
      if (!cancelledRef.current) {
        const msg = e instanceof Error ? e.message : String(e);
        const isTransient =
          msg.includes('not running') || msg.includes('not started') || msg.includes('transport');
        if (isTransient && attempt < 3) {
          retryTimerRef.current = setTimeout(
            () => fetchWalletInfoRef.current(address, attempt + 1),
            2000
          );
          return;
        }
        console.error('[WalletInfoSection] Failed to load wallet info:', e);
        enqueueError({
          id: crypto.randomUUID(),
          timestamp: Date.now(),
          source: 'manual',
          title: 'Failed to load wallet info',
          message: e instanceof Error ? e.message : String(e),
          sentryEvent: buildManualSentryEvent(
            { type: 'WalletInfoLoadError', value: e instanceof Error ? e.message : String(e) },
            { component: 'WalletInfoSection' }
          ),
          originalError: e instanceof Error ? e : new Error(String(e)),
        });
        setError('Failed to load wallet info');
        setBalance(null);
        setNetworkName(null);
        setLoading(false);
      }
    }
  }, []);

  fetchWalletInfoRef.current = fetchWalletInfo;

  useEffect(() => {
    cancelledRef.current = false;
    if (retryTimerRef.current) {
      clearTimeout(retryTimerRef.current);
      retryTimerRef.current = null;
    }

    if (!isConnected || !primaryAddress) {
      setLoading(false);
      setNetworkName(null);
      setBalance(null);
      setError(null);
      return;
    }

    setLoading(true);
    setError(null);

    fetchWalletInfo(primaryAddress);

    return () => {
      cancelledRef.current = true;
      if (retryTimerRef.current) clearTimeout(retryTimerRef.current);
    };
  }, [isConnected, primaryAddress, fetchWalletInfo]);

  if (!isConnected) return null;

  return (
    <div className="glass rounded-3xl p-4 shadow-large animate-fade-up mt-4">
      <div className="flex items-center gap-2 mb-3">
        <svg
          className="w-5 h-5 text-primary-500"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M3 10h18M7 15h1m4 0h1m-7 4h12a3 3 0 003-3V8a3 3 0 00-3-3H6a3 3 0 00-3 3v8a3 3 0 003 3z"
          />
        </svg>
        <span className="font-semibold text-sm">Web3 Wallet</span>
      </div>
      <div className="space-y-2 text-sm">
        <div className="flex justify-between items-center">
          <span className="opacity-70">Address</span>
          <span className="font-mono text-xs" title={primaryAddress ?? ''}>
            {primaryAddress ? truncateAddress(primaryAddress) : '—'}
          </span>
        </div>
        <div className="flex justify-between items-center">
          <span className="opacity-70">Network</span>
          <span>
            {loading ? (
              <span className="opacity-60">Loading…</span>
            ) : error ? (
              <span className="text-coral-500 text-xs">{error}</span>
            ) : (
              (networkName ?? '—')
            )}
          </span>
        </div>
        <div className="flex justify-between items-center">
          <span className="opacity-70">Balance</span>
          <span>
            {loading ? (
              <span className="opacity-60">Loading…</span>
            ) : error ? (
              '—'
            ) : (
              (balance ?? '—')
            )}
          </span>
        </div>
      </div>
    </div>
  );
}

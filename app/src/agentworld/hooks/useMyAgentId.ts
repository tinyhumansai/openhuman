import debugFactory from 'debug';
import { useEffect, useState } from 'react';

import { fetchWalletStatus } from '../../services/walletApi';

const debug = debugFactory('agentworld:wallet-identity');

export type MyAgentIdState =
  | { status: 'loading' }
  | { status: 'disconnected' }
  | { status: 'ready'; agentId: string }
  | { status: 'error'; error: Error };

export function useMyAgentId(): MyAgentIdState {
  const [state, setState] = useState<MyAgentIdState>({ status: 'loading' });

  useEffect(() => {
    let mounted = true;

    void fetchWalletStatus()
      .then(status => {
        if (!mounted) return;

        const solana = (status.accounts ?? []).find(account => account.chain === 'solana');
        setState(
          solana?.address
            ? { status: 'ready', agentId: solana.address }
            : { status: 'disconnected' }
        );
      })
      .catch((value: unknown) => {
        if (!mounted) return;

        debug('wallet identity resolution failed');
        setState({
          status: 'error',
          error: value instanceof Error ? value : new Error(String(value)),
        });
      });

    return () => {
      mounted = false;
    };
  }, []);

  return state;
}

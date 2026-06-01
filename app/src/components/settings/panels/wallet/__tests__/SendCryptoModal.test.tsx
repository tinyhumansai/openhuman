import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  BalanceInfo,
  ExecutionResult,
  PreparedTransaction,
} from '../../../../../services/walletApi';
import { renderWithProviders } from '../../../../../test/test-utils';
import SendCryptoModal from '../SendCryptoModal';

const mockPrepareTransfer = vi.fn<() => Promise<PreparedTransaction>>();
const mockExecutePrepared = vi.fn<() => Promise<ExecutionResult>>();

vi.mock('../../../../../services/walletApi', () => ({
  prepareTransfer: (...args: unknown[]) => mockPrepareTransfer(...(args as [])),
  executePrepared: (...args: unknown[]) => mockExecutePrepared(...(args as [])),
}));

const EVM_BALANCE: BalanceInfo = {
  chain: 'evm',
  evmNetwork: 'base_mainnet',
  address: '0x9858EfFD232B4033E47d90003D41EC34EcaEda94',
  assetSymbol: 'ETH',
  decimals: 18,
  raw: '2000000000000000000',
  formatted: '2.000000000000000000',
  providerStatus: 'ready',
};

const PREPARED: PreparedTransaction = {
  quoteId: 'quote-1',
  kind: 'native_transfer',
  chain: 'evm',
  evmNetwork: 'base_mainnet',
  fromAddress: EVM_BALANCE.address,
  toAddress: '0x1111111111111111111111111111111111111111',
  assetSymbol: 'ETH',
  amountRaw: '1000000000000000000',
  amountFormatted: '1',
  estimatedFeeRaw: '21000000000000',
  status: 'awaiting_confirmation',
  createdAtMs: 1,
  expiresAtMs: 2,
  notes: ['Simulated fee only'],
};

const EXECUTED: ExecutionResult = {
  quoteId: 'quote-1',
  status: 'broadcasted',
  chain: 'evm',
  evmNetwork: 'base_mainnet',
  transactionHash: '0xabc123def456',
  explorerUrl: 'https://basescan.org/tx/0xabc123def456',
  transaction: PREPARED,
};

beforeEach(() => {
  mockPrepareTransfer.mockReset();
  mockExecutePrepared.mockReset();
});

function renderModal() {
  const onClose = vi.fn();
  const onSuccess = vi.fn();
  renderWithProviders(
    <SendCryptoModal balance={EVM_BALANCE} onClose={onClose} onSuccess={onSuccess} />
  );
  return { onClose, onSuccess };
}

describe('SendCryptoModal', () => {
  it('drives prepare → review → execute and shows the tx hash', async () => {
    mockPrepareTransfer.mockResolvedValueOnce(PREPARED);
    mockExecutePrepared.mockResolvedValueOnce(EXECUTED);
    const { onSuccess } = renderModal();

    fireEvent.change(screen.getByTestId('send-recipient'), {
      target: { value: '0x1111111111111111111111111111111111111111' },
    });
    fireEvent.change(screen.getByTestId('send-amount'), { target: { value: '1' } });
    fireEvent.click(screen.getByTestId('send-review'));

    // Prepare is called with the amount converted to wei + the row's network.
    await waitFor(() => expect(mockPrepareTransfer).toHaveBeenCalledTimes(1));
    expect(mockPrepareTransfer).toHaveBeenCalledWith({
      chain: 'evm',
      toAddress: '0x1111111111111111111111111111111111111111',
      amountRaw: '1000000000000000000',
      evmNetwork: 'base_mainnet',
    });

    // Review step shows the simulated fee.
    await waitFor(() => expect(screen.getByTestId('send-fee')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('send-confirm'));

    await waitFor(() => expect(mockExecutePrepared).toHaveBeenCalledWith('quote-1'));
    await waitFor(() =>
      expect(screen.getByTestId('send-tx-hash')).toHaveTextContent('0xabc123def456')
    );

    // Completing the flow refreshes the parent.
    fireEvent.click(screen.getByRole('button', { name: /done/i }));
    expect(onSuccess).toHaveBeenCalled();
  });

  it('blocks an invalid amount before calling prepare', async () => {
    renderModal();

    fireEvent.change(screen.getByTestId('send-recipient'), { target: { value: '0xabc' } });
    fireEvent.change(screen.getByTestId('send-amount'), { target: { value: 'not-a-number' } });
    fireEvent.click(screen.getByTestId('send-review'));

    await waitFor(() => expect(screen.getByRole('alert')).toBeInTheDocument());
    expect(mockPrepareTransfer).not.toHaveBeenCalled();
  });

  it('surfaces a prepare failure as an alert', async () => {
    mockPrepareTransfer.mockRejectedValueOnce(new Error('insufficient funds'));
    renderModal();

    fireEvent.change(screen.getByTestId('send-recipient'), {
      target: { value: '0x1111111111111111111111111111111111111111' },
    });
    fireEvent.change(screen.getByTestId('send-amount'), { target: { value: '1' } });
    fireEvent.click(screen.getByTestId('send-review'));

    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent(/insufficient funds/i));
  });
});

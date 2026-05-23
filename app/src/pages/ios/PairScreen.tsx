/**
 * PairScreen — iOS-only QR pairing flow.
 *
 * Flow:
 *   1. User taps "Scan QR code" → barcode scanner opens.
 *   2. App parses the openhuman://pair?... URL from the scan result.
 *   3. Validates fields; rejects expired codes.
 *   4. Generates a fresh device X25519 keypair.
 *   5. Builds a ConnectionProfile and saves it via profileStore.
 *   6. Probes the channel via TransportManager.isHealthy().
 *   7. On success: navigates to /human (mobile tab bar shows Human/Chat/Settings).
 *   8. On failure: shows error + retry button.
 *
 * No dynamic imports. Static import of barcode scanner — caller guard is
 * the iOS-only route; desktop never renders this component.
 */
import { Format, scan } from '@tauri-apps/plugin-barcode-scanner';
import debug from 'debug';
import { type FC, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { base64urlEncode, generateKeypair } from '../../lib/tunnel/crypto';
import { type ConnectionProfile, saveProfile } from '../../services/transport/profileStore';
import { createTransportManager } from '../../services/transport/TransportManager';
import { BACKEND_URL } from '../../utils/config';

const log = debug('ios:pair-screen');
const logErr = debug('ios:pair-screen:error');

// -- QR payload parsing -------------------------------------------------------

interface PairPayload {
  channelId: string;
  pairingToken: string;
  corePubkey: string;
  rpcUrl?: string;
  expiresAt: number; // unix timestamp
}

function parsePairUrl(raw: string): PairPayload | null {
  log('[ios] parsing pair URL len=%d', raw.length);
  try {
    // Accept both the openhuman:// deep-link and a plain https:// fallback.
    // Normalise openhuman:// → https:// so URL() can parse it.
    const normalised = raw.startsWith('openhuman://')
      ? raw.replace('openhuman://', 'https://openhuman.app/')
      : raw;
    const url = new URL(normalised);
    const p = url.searchParams;

    const channelId = p.get('cid');
    const pairingToken = p.get('pt');
    const corePubkey = p.get('cpk');
    const rpcRaw = p.get('rpc');
    const expRaw = p.get('exp');

    if (!channelId || !pairingToken || !corePubkey || !expRaw) {
      logErr(
        '[ios] missing required QR fields cid=%s pt_len=%d cpk_len=%d exp=%s',
        channelId,
        pairingToken?.length ?? 0,
        corePubkey?.length ?? 0,
        expRaw
      );
      return null;
    }

    const expiresAt = parseInt(expRaw, 10);
    if (isNaN(expiresAt)) {
      logErr('[ios] invalid exp field: %s', expRaw);
      return null;
    }

    return { channelId, pairingToken, corePubkey, rpcUrl: rpcRaw ?? undefined, expiresAt };
  } catch (err) {
    logErr('[ios] URL parse error: %o', err);
    return null;
  }
}

// -- component ---------------------------------------------------------------

type ScreenState =
  | { kind: 'idle' }
  | { kind: 'scanning' }
  | { kind: 'error'; message: string }
  | { kind: 'expired' }
  | { kind: 'connecting' }
  | { kind: 'success' };

export const PairScreen: FC = () => {
  const navigate = useNavigate();
  const [state, setState] = useState<ScreenState>({ kind: 'idle' });

  async function startScan(): Promise<void> {
    log('[ios] starting QR scan');
    setState({ kind: 'scanning' });
    try {
      const result = await scan({ windowed: false, formats: [Format.QRCode] });
      const rawContent = result.content;
      log('[ios] scan result received len=%d', rawContent.length);

      await handleScanResult(rawContent);
    } catch (err) {
      logErr('[ios] scan error: %o', err);
      setState({
        kind: 'error',
        message: 'Camera scan failed. Check camera permissions and try again.',
      });
    }
  }

  async function handleScanResult(raw: string): Promise<void> {
    // 1. Parse
    const payload = parsePairUrl(raw);
    if (!payload) {
      setState({
        kind: 'error',
        message: 'Invalid QR code. Make sure you are scanning an OpenHuman pairing code.',
      });
      return;
    }

    // 2. Check expiry
    const nowSecs = Math.floor(Date.now() / 1000);
    if (payload.expiresAt < nowSecs) {
      log('[ios] QR expired at=%d now=%d', payload.expiresAt, nowSecs);
      setState({ kind: 'expired' });
      return;
    }
    log('[ios] QR valid; expires in %ds', payload.expiresAt - nowSecs);

    // 3. Generate device keypair
    const keypair = generateKeypair();
    const devicePubkeyB64 = base64urlEncode(keypair.publicKey);
    const devicePrivkeyB64 = base64urlEncode(keypair.secretKey);
    log('[ios] device keypair generated pubkey_len=%d', devicePubkeyB64.length);
    // NOTE: Never log the private key value — log length only.
    log('[ios] device privkey_len=%d (not logged)', devicePrivkeyB64.length);

    // 4. Build and persist profile
    const profile: ConnectionProfile = {
      id: payload.channelId,
      label: 'Desktop',
      kind: 'tunnel',
      channelId: payload.channelId,
      pairingToken: payload.pairingToken,
      corePubkey: payload.corePubkey,
      rpcUrl: payload.rpcUrl,
      devicePrivkey: devicePrivkeyB64,
      // sessionToken will be written after the tunnel handshake completes.
    };
    saveProfile(profile);
    log('[ios] profile saved id=%s kind=%s', profile.id, profile.kind);

    // 5. Probe transport health
    setState({ kind: 'connecting' });
    try {
      const manager = createTransportManager(profile, { backendSocketUrl: BACKEND_URL });
      const transport = await manager.getTransport();
      const healthy = await transport.isHealthy();
      if (!healthy) {
        logErr('[ios] transport health check failed kind=%s', transport.kind);
        setState({
          kind: 'error',
          message: 'Could not reach the desktop. Make sure both devices are online and try again.',
        });
        return;
      }
      log('[ios] transport healthy kind=%s; navigating to /human', transport.kind);
    } catch (err) {
      logErr('[ios] transport probe error: %o', err);
      setState({
        kind: 'error',
        message: 'Connection failed. Make sure the desktop app is running and try again.',
      });
      return;
    }

    // 6. Navigate to the Human page now that pairing is established.
    setState({ kind: 'success' });
    navigate('/human', { replace: true });
  }

  return (
    <div className="flex flex-col items-center justify-center min-h-screen bg-[#0f1117] text-white px-6 py-12">
      <div className="flex flex-col items-center gap-8 max-w-sm w-full">
        {/* Logo / icon area */}
        <div className="w-20 h-20 rounded-2xl bg-[#4A83DD] flex items-center justify-center shadow-lg">
          <svg
            width="40"
            height="40"
            viewBox="0 0 40 40"
            fill="none"
            xmlns="http://www.w3.org/2000/svg"
            aria-hidden="true">
            <rect x="4" y="4" width="14" height="14" rx="2" fill="white" fillOpacity="0.9" />
            <rect x="22" y="4" width="14" height="14" rx="2" fill="white" fillOpacity="0.9" />
            <rect x="4" y="22" width="14" height="14" rx="2" fill="white" fillOpacity="0.9" />
            <rect x="26" y="26" width="6" height="6" rx="1" fill="white" fillOpacity="0.9" />
            <rect x="22" y="22" width="6" height="6" rx="1" fill="white" fillOpacity="0.6" />
            <rect x="32" y="22" width="6" height="6" rx="1" fill="white" fillOpacity="0.6" />
          </svg>
        </div>

        {/* Heading */}
        <div className="text-center">
          <h1 className="text-2xl font-semibold text-white mb-2">Pair with your desktop</h1>
          <p className="text-sm text-white/60 leading-relaxed">
            Open OpenHuman on your desktop, go to Settings &gt; Devices, and tap &ldquo;Pair
            phone&rdquo; to show the QR code.
          </p>
        </div>

        {/* State-specific content */}
        {state.kind === 'idle' && (
          <button
            onClick={() => void startScan()}
            className="w-full py-4 rounded-xl bg-[#4A83DD] text-white font-medium text-base
                       active:opacity-80 transition-opacity shadow-md">
            Scan QR code
          </button>
        )}

        {state.kind === 'scanning' && (
          <p className="text-white/60 text-sm text-center animate-pulse">Scanner opening...</p>
        )}

        {state.kind === 'connecting' && (
          <p className="text-white/60 text-sm text-center animate-pulse">
            Connecting to desktop...
          </p>
        )}

        {state.kind === 'success' && (
          <p className="text-green-400 text-sm text-center">Connected! Loading...</p>
        )}

        {state.kind === 'expired' && (
          <div className="flex flex-col items-center gap-4 text-center">
            <p className="text-amber-400 text-sm">
              QR code expired. Ask the desktop to regenerate the code.
            </p>
            <button
              onClick={() => setState({ kind: 'idle' })}
              className="w-full py-3 rounded-xl border border-white/20 text-white/80 text-sm
                         active:opacity-70 transition-opacity">
              Try again
            </button>
          </div>
        )}

        {state.kind === 'error' && (
          <div className="flex flex-col items-center gap-4 text-center w-full">
            <p className="text-red-400 text-sm">{state.message}</p>
            <button
              onClick={() => void startScan()}
              className="w-full py-3 rounded-xl bg-[#4A83DD]/80 text-white text-sm
                         active:opacity-70 transition-opacity">
              Retry scan
            </button>
            <button
              onClick={() => setState({ kind: 'idle' })}
              className="text-white/40 text-xs underline-offset-2 underline">
              Cancel
            </button>
          </div>
        )}

        {/* Step hint */}
        {(state.kind === 'idle' || state.kind === 'error' || state.kind === 'expired') && (
          <div className="flex flex-col gap-3 w-full mt-2">
            {[
              'Open OpenHuman on desktop',
              'Go to Settings > Devices',
              'Tap "Pair phone" to show QR',
            ].map((step, i) => (
              <div key={step} className="flex items-center gap-3 text-white/50 text-xs">
                <span className="w-5 h-5 rounded-full border border-white/20 flex items-center justify-center text-[10px] shrink-0">
                  {i + 1}
                </span>
                {step}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
};

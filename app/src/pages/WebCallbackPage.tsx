import { useEffect } from 'react';
import { useLocation, useParams } from 'react-router-dom';

import { handleDeepLinkUrls } from '../utils/desktopDeepLinkListener';

interface WebCallbackPageProps {
  callbackKind?: string;
  callbackStatus?: string;
}

function buildSyntheticDeepLink(
  kind: string | undefined,
  status: string | undefined,
  search: string
): string | null {
  if (kind === 'auth') {
    return `openhuman://auth${search}`;
  }

  if (kind === 'oauth' && status) {
    return `openhuman://oauth/${status}${search}`;
  }

  return null;
}

export default function WebCallbackPage({
  callbackKind,
  callbackStatus,
}: WebCallbackPageProps = {}) {
  const { kind: routeKind, status: routeStatus } = useParams();
  const location = useLocation();
  const kind = callbackKind ?? routeKind;
  const status = callbackStatus ?? routeStatus;

  useEffect(() => {
    const synthetic = buildSyntheticDeepLink(kind, status, location.search);
    if (!synthetic) return;

    // This is the SAME-ORIGIN web callback route, reached only through the app's
    // own routing / the backend OAuth redirect — not via the OS `openhuman://`
    // scheme that any external app can trigger. The C3 state-nonce CSRF guard
    // targets that custom-scheme transport, so it does not apply here; pass
    // requireStateNonce:false. (Web-build login-CSRF hardening — binding this
    // callback to a backend-echoed OAuth state — is tracked as a follow-up.)
    void handleDeepLinkUrls([synthetic], { requireStateNonce: false });
  }, [kind, status, location.search]);

  return (
    <div className="flex min-h-[60vh] items-center justify-center px-6 text-center">
      <div className="max-w-md space-y-3">
        <h1 className="text-2xl font-semibold text-slate-900">Completing sign-in</h1>
        <p className="text-sm text-slate-600">
          OpenHuman is processing your callback and will continue automatically.
        </p>
      </div>
    </div>
  );
}

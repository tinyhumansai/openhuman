import { useEffect } from 'react';
import { useLocation, useParams } from 'react-router-dom';

import { handleDeepLinkUrls } from '../utils/desktopDeepLinkListener';

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

export default function WebCallbackPage() {
  const { kind, status } = useParams();
  const location = useLocation();

  useEffect(() => {
    const synthetic = buildSyntheticDeepLink(kind, status, location.search);
    if (!synthetic) return;
    void handleDeepLinkUrls([synthetic]);
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

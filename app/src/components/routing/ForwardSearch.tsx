import { Navigate, useLocation } from 'react-router-dom';

/**
 * A back-compat redirect that carries the deep link with it.
 *
 * `<Navigate to="/x" />` discards the current `search` and `hash`, so a link
 * like `/webhooks?tab=inbound#delivery-3` lands on a bare destination and the
 * user loses the thing they followed the link for. This copies both onto the
 * target instead.
 *
 * Introduced for `/skills` in #5924 and lifted out of `AppRoutes.tsx` here so
 * the settings route table can use the same mechanism. It cannot simply import
 * from `AppRoutes.tsx`: `AppRoutes` → `Settings` → `settingsRouteElements`
 * already, so that import would be circular.
 *
 * Use it for a redirect whose destination takes NO query of its own. Where the
 * target already carries one — `/channels` → `/connections?tab=messaging` —
 * appending the incoming `search` would produce a second `?` and a malformed
 * URL, so those need a merge rather than a concatenation.
 */
export default function ForwardSearch({ to }: { to: string }) {
  const { search, hash } = useLocation();
  return <Navigate to={`${to}${search}${hash}`} replace />;
}

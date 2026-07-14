import { useUser } from '../../hooks/useUser';

/**
 * Whether the current user is entitled to Medulla (the orchestration model) and
 * the live orchestration engine, driven by the backend `user.hasMedullaAccess`
 * flag. When `false` (the default until early access is granted), the
 * Orchestration page hides the live chat and renders a scale showcase (demo
 * graph / tasks / network) instead of the live surfaces.
 */
export function useMedullaAccess(): boolean {
  const { user } = useUser();
  return Boolean(user?.hasMedullaAccess);
}

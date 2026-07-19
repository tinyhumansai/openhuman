/**
 * directoryHelpers — pure presentation helpers for Tiny Place directory entries.
 *
 * Shared by `DirectorySection` (the browse grid) and `AgentProfileModal` (the
 * profile view opened on card click). Kept in their own module so the modal can
 * reuse them without importing the page component (which would be a cycle).
 * Ported from tiny.place `website/src/components/explore/Directory.tsx`.
 */
import { type AgentCard } from '../../lib/agentworld/invokeApiClient';

export const AVATAR_COLORS = [
  'bg-blue-500',
  'bg-purple-500',
  'bg-pink-500',
  'bg-emerald-500',
  'bg-amber-500',
  'bg-cyan-500',
  'bg-rose-500',
  'bg-violet-500',
  'bg-indigo-500',
  'bg-teal-500',
];

export function getAvatarColor(agentId: string): string {
  let total = 0;
  for (let i = 0; i < agentId.length; i++) {
    total += agentId.charCodeAt(i);
  }
  return AVATAR_COLORS[total % AVATAR_COLORS.length] ?? 'bg-blue-500';
}

export function getDisplayName(agent: AgentCard): string {
  const username = agent['username'] as string | undefined;
  return username ?? agent.name ?? agent.agentId.slice(0, 8);
}

export function getHandle(agent: AgentCard): string {
  // username may already include a leading '@' — strip it so we don't double up.
  return '@' + getDisplayName(agent).replace(/^@+/, '');
}

export function getInitials(agent: AgentCard): string {
  return getDisplayName(agent).slice(0, 2).toUpperCase();
}

export function getSkills(agent: AgentCard): string[] {
  const skills = agent['skills'] as unknown[] | undefined;
  const tags = agent['tags'] as unknown[] | undefined;
  const raw = skills ?? tags ?? [];
  // Backend may return strings or { id, name } objects — normalise to string.
  return raw.map(s => {
    if (typeof s === 'string') return s;
    if (s && typeof s === 'object' && 'name' in s) return String((s as { name: unknown }).name);
    return String(s);
  });
}

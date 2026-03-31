/**
 * Sync tool/skill state to the backend via `tool:sync` socket event.
 *
 * Called whenever skill connection state changes or the socket reconnects,
 * so the backend always has an up-to-date picture of connected tools.
 */

import { socketService } from '../../services/socketService';
import { getAllSnapshots } from './skillsApi';
import type { SkillConnectionStatus } from './types';

interface ToolSyncEntry {
  skillId: string;
  name: string;
  status: SkillConnectionStatus;
  tools: string[];
}

/**
 * Fetch all skill snapshots from the Rust engine and emit a `tool:sync`
 * event with the full list.
 */
export function syncToolsToBackend(): void {
  getAllSnapshots()
    .then(snapshots => {
      const tools: ToolSyncEntry[] = snapshots.map(snap => ({
        skillId: snap.skill_id,
        name: snap.name,
        status: (snap.connection_status as SkillConnectionStatus) || 'offline',
        tools: (snap.tools ?? []).map(t => t.name),
      }));

      socketService.emit('tool:sync', { tools });
    })
    .catch(err => {
      console.warn('[sync] Failed to fetch snapshots for tool:sync:', err);
    });
}

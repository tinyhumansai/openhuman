/**
 * Management panel shown when clicking a connected (setupComplete) skill.
 * Displays connection status, configurable options, and action buttons.
 */

import { useState, useEffect, useCallback } from "react";
import { useAppSelector } from "../../store/hooks";
import {
  useSkillConnectionStatus,
  useSkillConnectionInfo,
  useSkillState,
} from "../../lib/skills/hooks";
import { skillManager } from "../../lib/skills/manager";
import type {
  SkillOptionDefinition,
} from "../../lib/skills/types";

interface SkillManagementPanelProps {
  skillId: string;
  onClose: () => void;
  /** If provided, shows a "Re-run Setup" button. Omit for skills without setup. */
  onReconfigure?: () => void;
}


export default function SkillManagementPanel({
  skillId,
  onClose,
  onReconfigure,
}: SkillManagementPanelProps) {
  const skill = useAppSelector((state) => state.skills.skills[skillId]);
  const connectionStatus = useSkillConnectionStatus(skillId);
  const connectionInfo = useSkillConnectionInfo(skillId);
  const skillState = useSkillState<{
    syncInProgress?: boolean;
    syncProgress?: number | null;
    syncProgressMessage?: string | null;
    syncCompleted?: boolean;
    syncError?: string | null;
    lastSyncTime?: number | null;
    storage?: {
      chatCount?: number;
      messageCount?: number;
      contactCount?: number;
      unreadCount?: number;
    };
  }>(skillId);

  const [options, setOptions] = useState<SkillOptionDefinition[]>([]);
  const [togglingOption, setTogglingOption] = useState<string | null>(null);
  const [restarting, setRestarting] = useState(false);
  const [disconnecting, setDisconnecting] = useState(false);
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);

  // Load options from the skill process
  useEffect(() => {
    let cancelled = false;
    async function load() {
      if (!skillManager.isSkillRunning(skillId)) return;
      try {
        const opts = await skillManager.listOptions(skillId);
        if (!cancelled) setOptions(opts);
      } catch {
        // Skill may not support options
      }
    }
    load();
    return () => {
      cancelled = true;
    };
  }, [skillId, connectionStatus]);

  const handleToggleOption = useCallback(
    async (name: string, currentValue: unknown) => {
      setTogglingOption(name);
      try {
        const newValue = !currentValue;
        await skillManager.setOption(skillId, name, newValue);
        const opts = await skillManager.listOptions(skillId);
        setOptions(opts);
      } catch (err) {
        console.error("[SkillManagementPanel] Toggle option failed:", err);
      } finally {
        setTogglingOption(null);
      }
    },
    [skillId],
  );

  const handleSelectOption = useCallback(
    async (name: string, value: string) => {
      setTogglingOption(name);
      try {
        await skillManager.setOption(skillId, name, value);
        const opts = await skillManager.listOptions(skillId);
        setOptions(opts);
      } catch (err) {
        console.error("[SkillManagementPanel] Set option failed:", err);
      } finally {
        setTogglingOption(null);
      }
    },
    [skillId],
  );

  const handleRestart = useCallback(async () => {
    if (!skill?.manifest) return;
    setRestarting(true);
    try {
      await skillManager.stopSkill(skillId);
      await skillManager.startSkill(skill.manifest);
    } catch (err) {
      console.error("[SkillManagementPanel] Restart failed:", err);
    } finally {
      setRestarting(false);
    }
  }, [skillId, skill?.manifest]);

  const handleDisconnect = useCallback(async () => {
    setDisconnecting(true);
    try {
      await skillManager.disconnectSkill(skillId);
      onClose();
    } catch (err) {
      console.error("[SkillManagementPanel] Disconnect failed:", err);
      setDisconnecting(false);
    }
  }, [skillId, onClose]);

  const handleSync = useCallback(async () => {
    try {
      await skillManager.triggerSync(skillId);
    } catch (err) {
      console.error("[SkillManagementPanel] Sync trigger failed:", err);
    }
  }, [skillId]);

  const syncInProgress = skillState?.syncInProgress ?? false;
  const syncProgress = skillState?.syncProgress ?? null;
  const syncProgressMessage = skillState?.syncProgressMessage ?? null;
  const syncCompleted = skillState?.syncCompleted ?? false;
  const syncError = skillState?.syncError ?? null;
  const lastSyncTime = skillState?.lastSyncTime ?? null;
  // const storage = skillState?.storage;
  const hasSyncSupport = syncCompleted || syncInProgress || syncError !== null;

  return (
    <div className="space-y-4">
      {/* Error message */}
      {connectionInfo.error && (
        <div className="text-xs text-coral-400 bg-coral-500/10 border border-coral-500/20 rounded-lg px-3 py-2 break-words">
          {connectionInfo.error}
        </div>
      )}

      {/* Data Sync */}
      {hasSyncSupport && (
        <div className="space-y-2">
          <div className="flex items-center justify-between px-0.5">
            <div className="text-xs font-medium text-stone-400">Data Sync</div>
            <button
              onClick={handleSync}
              disabled={syncInProgress || connectionStatus !== "connected"}
              className="px-2.5 py-1 text-[11px] font-medium text-primary-300 bg-primary-500/10 border border-primary-500/30 rounded-lg hover:bg-primary-500/20 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
            >
              {syncInProgress ? "Syncing..." : "Sync Now"}
            </button>
          </div>

          {syncInProgress && (
            <div className="rounded-lg bg-stone-800/40 border border-stone-700/40 px-3 py-2.5 space-y-2">
              {syncProgressMessage && (
                <div className="text-[11px] text-stone-400 truncate">
                  {syncProgressMessage}
                </div>
              )}
              <div className="flex items-center gap-2">
                <div className="flex-1 h-1.5 bg-stone-700/60 rounded-full overflow-hidden">
                  <div
                    className="h-full bg-primary-500 rounded-full transition-all duration-300 ease-out"
                    style={{ width: `${syncProgress ?? 0}%` }}
                  />
                </div>
                {syncProgress !== null && (
                  <span className="text-[11px] text-stone-500 tabular-nums w-8 text-right">
                    {syncProgress}%
                  </span>
                )}
              </div>
            </div>
          )}

          {!syncInProgress && syncError && (
            <div className="text-[11px] text-coral-400 bg-coral-500/10 border border-coral-500/20 rounded-lg px-3 py-2 break-words">
              Sync failed: {syncError}
            </div>
          )}

          {!syncInProgress && syncCompleted && (
            <div className="rounded-lg bg-stone-800/40 border border-stone-700/40 px-3 py-2">
              {lastSyncTime && (
                <div className="text-[11px] text-stone-400">
                  Last synced: {new Date(lastSyncTime).toLocaleString()}
                </div>
              )}
              {/* {storage && (storage.chatCount > 0 || storage.messageCount > 0 || storage.contactCount > 0) && (
                <div className="text-[11px] text-stone-500 mt-0.5">
                  {[
                    storage.chatCount > 0 && `${storage.chatCount} chats`,
                    storage.messageCount > 0 && `${storage.messageCount.toLocaleString()} messages`,
                    storage.contactCount > 0 && `${storage.contactCount} contacts`,
                  ].filter(Boolean).join(" \u00B7 ")}
                </div>
              )} */}
            </div>
          )}
        </div>
      )}

      {/* Options */}
      {options.length > 0 && (
        <div className="space-y-1">
          <div className="text-xs font-medium text-stone-400 px-0.5 mb-2">
            Options
          </div>
          {options.map((opt) => (
            <div
              key={opt.name}
              className="flex items-center justify-between rounded-lg bg-stone-800/40 border border-stone-700/40 px-3 py-2.5"
            >
              <div className="min-w-0 mr-3">
                <div className="text-xs font-medium text-stone-200">
                  {opt.label}
                </div>
                {opt.description && (
                  <div className="text-[11px] text-stone-500 mt-0.5 line-clamp-1">
                    {opt.description}
                  </div>
                )}
              </div>
              {opt.type === "boolean" && (
                <button
                  onClick={() => handleToggleOption(opt.name, opt.value)}
                  disabled={togglingOption === opt.name}
                  className={`relative flex-shrink-0 w-9 h-5 rounded-full transition-colors duration-200 ${opt.value ? "bg-primary-500" : "bg-stone-600"
                    } ${togglingOption === opt.name ? "opacity-50" : ""}`}
                >
                  <span
                    className={`absolute top-0.5 left-0.5 w-4 h-4 bg-white rounded-full shadow transition-transform duration-200 ${opt.value ? "translate-x-4" : "translate-x-0"
                      }`}
                  />
                </button>
              )}
              {opt.type === "select" && opt.options && (
                <select
                  value={String(opt.value ?? "")}
                  onChange={(e) => handleSelectOption(opt.name, e.target.value)}
                  disabled={togglingOption === opt.name}
                  className={`flex-shrink-0 text-xs bg-stone-700/60 border border-stone-600/50 text-stone-200 rounded-lg px-2 py-1.5 outline-none focus:border-primary-500/50 transition-colors ${
                    togglingOption === opt.name ? "opacity-50" : ""
                  }`}
                >
                  {opt.options.map((o) => (
                    <option key={o.value} value={o.value}>
                      {o.label}
                    </option>
                  ))}
                </select>
              )}
            </div>
          ))}
        </div>
      )}

      {/* Action buttons — single row */}
      <div className="pt-1">
        {!confirmDisconnect ? (
          <div className="flex space-x-2">
            <button
              onClick={handleRestart}
              disabled={restarting}
              className="flex-1 px-3 py-2 text-xs font-medium text-white bg-stone-700/60 border border-stone-600/50 rounded-xl hover:bg-stone-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {restarting ? "Restarting..." : "Restart"}
            </button>
            {onReconfigure && (
              <button
                onClick={onReconfigure}
                className="flex-1 px-3 py-2 text-xs font-medium text-primary-300 bg-primary-500/10 border border-primary-500/30 rounded-xl hover:bg-primary-500/20 transition-colors"
              >
                Re-run Setup
              </button>
            )}
            <button
              onClick={() => setConfirmDisconnect(true)}
              className="flex-1 px-3 py-2 text-xs font-medium text-coral-400 bg-coral-500/10 border border-coral-500/30 rounded-xl hover:bg-coral-500/20 transition-colors"
            >
              Disconnect
            </button>
          </div>
        ) : (
          <div className="flex space-x-2">
            <button
              onClick={() => setConfirmDisconnect(false)}
              className="flex-1 px-3 py-2 text-xs font-medium text-stone-400 bg-stone-800/50 border border-stone-700 rounded-xl hover:bg-stone-800 transition-colors"
            >
              Cancel
            </button>
            <button
              onClick={handleDisconnect}
              disabled={disconnecting}
              className="flex-1 px-3 py-2 text-xs font-medium text-white bg-coral-500 rounded-xl hover:bg-coral-600 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {disconnecting ? "Disconnecting..." : "Confirm Disconnect"}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

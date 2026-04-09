/**
 * Hardware and service management commands.
 */
import { invoke } from '@tauri-apps/api/core';

import { callCoreRpc } from '../../services/coreRpcClient';
import { CommandResponse, isTauri, parseServiceCliOutput } from './common';

export type HardwareTransport = 'Native' | 'Serial' | 'Probe' | 'None';
export type ServiceState = 'Running' | 'Stopped' | 'NotInstalled' | { Unknown: string };

export interface DiscoveredDevice {
  name: string;
  detail?: string | null;
  device_path?: string | null;
  transport: HardwareTransport;
}

export interface HardwareIntrospect {
  path: string;
  vid?: number | null;
  pid?: number | null;
  board_name?: string | null;
  architecture?: string | null;
  memory_map_note: string;
}

export interface ServiceStatus {
  state: ServiceState;
  unit_path?: string | null;
  label: string;
  details?: string | null;
}

export interface AgentServerStatus {
  running: boolean;
  url: string;
}

export interface DaemonHostConfig {
  show_tray: boolean;
}

export interface RestartStatus {
  accepted: boolean;
  source: string;
  reason: string;
}

export async function openhumanHardwareDiscover(): Promise<CommandResponse<DiscoveredDevice[]>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<DiscoveredDevice[]>>({
    method: 'openhuman.hardware_discover',
  });
}

export async function openhumanHardwareIntrospect(
  path: string
): Promise<CommandResponse<HardwareIntrospect>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<HardwareIntrospect>>({
    method: 'openhuman.hardware_introspect',
    params: { path },
  });
}

export async function openhumanServiceInstall(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  try {
    return await callCoreRpc<CommandResponse<ServiceStatus>>({
      method: 'openhuman.service_install',
    });
  } catch {
    const raw = await invoke<string>('service_install_direct');
    return parseServiceCliOutput<ServiceStatus>(raw);
  }
}

export async function openhumanServiceStart(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  try {
    return await callCoreRpc<CommandResponse<ServiceStatus>>({ method: 'openhuman.service_start' });
  } catch {
    const raw = await invoke<string>('service_start_direct');
    return parseServiceCliOutput<ServiceStatus>(raw);
  }
}

export async function openhumanServiceStop(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  try {
    return await callCoreRpc<CommandResponse<ServiceStatus>>({ method: 'openhuman.service_stop' });
  } catch {
    const raw = await invoke<string>('service_stop_direct');
    return parseServiceCliOutput<ServiceStatus>(raw);
  }
}

export async function openhumanServiceStatus(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  try {
    return await callCoreRpc<CommandResponse<ServiceStatus>>({
      method: 'openhuman.service_status',
    });
  } catch {
    const raw = await invoke<string>('service_status_direct');
    return parseServiceCliOutput<ServiceStatus>(raw);
  }
}

export async function openhumanServiceUninstall(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  try {
    return await callCoreRpc<CommandResponse<ServiceStatus>>({
      method: 'openhuman.service_uninstall',
    });
  } catch {
    const raw = await invoke<string>('service_uninstall_direct');
    return parseServiceCliOutput<ServiceStatus>(raw);
  }
}

export async function openhumanServiceRestart(
  source?: string,
  reason?: string
): Promise<CommandResponse<RestartStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<RestartStatus>>({
    method: 'openhuman.service_restart',
    params: { source, reason },
  });
}

export async function openhumanAgentServerStatus(): Promise<CommandResponse<AgentServerStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AgentServerStatus>>({
    method: 'openhuman.agent_server_status',
  });
}

export async function openhumanGetDaemonHostConfig(): Promise<CommandResponse<DaemonHostConfig>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<DaemonHostConfig>>({
    method: 'openhuman.service_daemon_host_get',
  });
}

export async function openhumanSetDaemonHostConfig(
  showTray: boolean
): Promise<CommandResponse<DaemonHostConfig>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<DaemonHostConfig>>({
    method: 'openhuman.service_daemon_host_set',
    params: { show_tray: showTray },
  });
}

import { useMemo, useState } from 'react';

import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import {
  alphahumanAgentChat,
  alphahumanDecryptSecret,
  alphahumanDoctorModels,
  alphahumanDoctorReport,
  alphahumanGetIntegrationInfo,
  alphahumanGetConfig,
  alphahumanHardwareDiscover,
  alphahumanHardwareIntrospect,
  alphahumanListIntegrations,
  alphahumanMigrateOpenclaw,
  alphahumanModelsRefresh,
  alphahumanUpdateGatewaySettings,
  alphahumanUpdateMemorySettings,
  alphahumanUpdateModelSettings,
  alphahumanUpdateRuntimeSettings,
  alphahumanUpdateTunnelSettings,
  alphahumanServiceInstall,
  alphahumanServiceStart,
  alphahumanServiceStatus,
  alphahumanServiceStop,
  alphahumanServiceUninstall,
  alphahumanEncryptSecret,
  isTauri,
  TunnelConfig,
  runtimeDisableSkill,
  runtimeEnableSkill,
  runtimeIsSkillEnabled,
  runtimeListSkills,
  SkillSnapshot,
} from '../../../utils/tauriCommands';

const formatJson = (value: unknown) => JSON.stringify(value, null, 2);

const TauriCommandsPanel = () => {
  const { navigateBack } = useSettingsNavigation();
  const [output, setOutput] = useState<string>('');
  const [error, setError] = useState<string>('');
  const [providerOverride, setProviderOverride] = useState<string>('');
  const [integrationName, setIntegrationName] = useState<string>('');
  const [hardwarePath, setHardwarePath] = useState<string>('');
  const [migrationSource, setMigrationSource] = useState<string>('');
  const [encryptInput, setEncryptInput] = useState<string>('');
  const [decryptInput, setDecryptInput] = useState<string>('');
  const [apiKey, setApiKey] = useState<string>('');
  const [apiUrl, setApiUrl] = useState<string>('');
  const [defaultProvider, setDefaultProvider] = useState<string>('');
  const [defaultModel, setDefaultModel] = useState<string>('');
  const [defaultTemp, setDefaultTemp] = useState<string>('0.7');
  const [tunnelProvider, setTunnelProvider] = useState<string>('none');
  const [cloudflareToken, setCloudflareToken] = useState<string>('');
  const [ngrokToken, setNgrokToken] = useState<string>('');
  const [tailscaleHostname, setTailscaleHostname] = useState<string>('');
  const [customCommand, setCustomCommand] = useState<string>('');
  const [gatewayHost, setGatewayHost] = useState<string>('127.0.0.1');
  const [gatewayPort, setGatewayPort] = useState<string>('3000');
  const [gatewayPairing, setGatewayPairing] = useState<boolean>(true);
  const [gatewayPublic, setGatewayPublic] = useState<boolean>(false);
  const [memoryBackend, setMemoryBackend] = useState<string>('sqlite');
  const [memoryAutoSave, setMemoryAutoSave] = useState<boolean>(true);
  const [embeddingProvider, setEmbeddingProvider] = useState<string>('none');
  const [embeddingModel, setEmbeddingModel] = useState<string>('text-embedding-3-small');
  const [embeddingDims, setEmbeddingDims] = useState<string>('1536');
  const [runtimeKind, setRuntimeKind] = useState<string>('native');
  const [reasoningEnabled, setReasoningEnabled] = useState<boolean>(false);
  const [skills, setSkills] = useState<
    Array<{ snapshot: SkillSnapshot; enabled: boolean }>
  >([]);
  const [skillsLoading, setSkillsLoading] = useState<boolean>(false);
  const [chatInput, setChatInput] = useState<string>('');
  const [chatProvider, setChatProvider] = useState<string>('');
  const [chatModel, setChatModel] = useState<string>('');
  const [chatTemperature, setChatTemperature] = useState<string>('0.7');
  const [chatLog, setChatLog] = useState<Array<{ role: 'user' | 'agent'; text: string }>>([]);
  const tauriAvailable = useMemo(() => isTauri(), []);
  const parseOptionalNumber = (value: string): number | null => {
    if (!value.trim()) {
      return null;
    }
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  };

  const run = async (fn: () => Promise<unknown>) => {
    setError('');
    try {
      const result = await fn();
      setOutput(formatJson(result));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
    }
  };

  const runWithResult = async <T,>(fn: () => Promise<T>): Promise<T | null> => {
    setError('');
    try {
      const result = await fn();
      setOutput(formatJson(result));
      return result;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      return null;
    }
  };

  const loadConfig = async () => {
    const response = await runWithResult(() => alphahumanGetConfig());
    if (!response) {
      return;
    }
    try {
      const snapshot = response.result;
      const config = snapshot.config as Record<string, unknown>;
      setApiKey((config.api_key as string) ?? '');
      setApiUrl((config.api_url as string) ?? '');
      setDefaultProvider((config.default_provider as string) ?? '');
      setDefaultModel((config.default_model as string) ?? '');
      setDefaultTemp(String((config.default_temperature as number) ?? 0.7));

      const tunnel = (config.tunnel as Record<string, unknown>) ?? {};
      setTunnelProvider((tunnel.provider as string) ?? 'none');
      setCloudflareToken(((tunnel.cloudflare as Record<string, unknown>)?.token as string) ?? '');
      setNgrokToken(((tunnel.ngrok as Record<string, unknown>)?.auth_token as string) ?? '');
      setTailscaleHostname(((tunnel.tailscale as Record<string, unknown>)?.hostname as string) ?? '');
      setCustomCommand(((tunnel.custom as Record<string, unknown>)?.start_command as string) ?? '');

      const gateway = (config.gateway as Record<string, unknown>) ?? {};
      setGatewayHost((gateway.host as string) ?? '127.0.0.1');
      setGatewayPort(String((gateway.port as number) ?? 3000));
      setGatewayPairing((gateway.require_pairing as boolean) ?? true);
      setGatewayPublic((gateway.allow_public_bind as boolean) ?? false);

      const memory = (config.memory as Record<string, unknown>) ?? {};
      setMemoryBackend((memory.backend as string) ?? 'sqlite');
      setMemoryAutoSave((memory.auto_save as boolean) ?? true);
      setEmbeddingProvider((memory.embedding_provider as string) ?? 'none');
      setEmbeddingModel((memory.embedding_model as string) ?? 'text-embedding-3-small');
      setEmbeddingDims(String((memory.embedding_dimensions as number) ?? 1536));

      const runtime = (config.runtime as Record<string, unknown>) ?? {};
      setRuntimeKind((runtime.kind as string) ?? 'native');
      setReasoningEnabled((runtime.reasoning_enabled as boolean) ?? false);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
    }
  };

  const buildTunnelConfig = (): TunnelConfig => {
    if (tunnelProvider === 'cloudflare') {
      return {
        provider: 'cloudflare',
        cloudflare: { token: cloudflareToken },
      };
    }
    if (tunnelProvider === 'ngrok') {
      return {
        provider: 'ngrok',
        ngrok: { auth_token: ngrokToken },
      };
    }
    if (tunnelProvider === 'tailscale') {
      return {
        provider: 'tailscale',
        tailscale: { hostname: tailscaleHostname || null },
      };
    }
    if (tunnelProvider === 'custom') {
      return {
        provider: 'custom',
        custom: { start_command: customCommand },
      };
    }
    return { provider: 'none' };
  };

  const saveModelSettings = () =>
    run(() =>
      alphahumanUpdateModelSettings({
        api_key: apiKey.trim() ? apiKey : null,
        api_url: apiUrl.trim() ? apiUrl : null,
        default_provider: defaultProvider.trim() ? defaultProvider : null,
        default_model: defaultModel.trim() ? defaultModel : null,
        default_temperature: parseOptionalNumber(defaultTemp),
      })
    );

  const saveTunnelSettings = () => run(() => alphahumanUpdateTunnelSettings(buildTunnelConfig()));

  const saveGatewaySettings = () =>
    run(() =>
      alphahumanUpdateGatewaySettings({
        host: gatewayHost.trim() ? gatewayHost : null,
        port: parseOptionalNumber(gatewayPort),
        require_pairing: gatewayPairing,
        allow_public_bind: gatewayPublic,
      })
    );

  const saveMemorySettings = () =>
    run(() =>
      alphahumanUpdateMemorySettings({
        backend: memoryBackend.trim() ? memoryBackend : null,
        auto_save: memoryAutoSave,
        embedding_provider: embeddingProvider.trim() ? embeddingProvider : null,
        embedding_model: embeddingModel.trim() ? embeddingModel : null,
        embedding_dimensions: parseOptionalNumber(embeddingDims),
      })
    );

  const saveRuntimeSettings = () =>
    run(() =>
      alphahumanUpdateRuntimeSettings({
        kind: runtimeKind.trim() ? runtimeKind : null,
        reasoning_enabled: reasoningEnabled,
      })
    );

  const loadSkills = async () => {
    setSkillsLoading(true);
    try {
      const snapshots = await runtimeListSkills();
      const enriched = await Promise.all(
        snapshots.map(async (snapshot) => ({
          snapshot,
          enabled: await runtimeIsSkillEnabled(snapshot.skill_id),
        }))
      );
      setSkills(enriched);
      setOutput(formatJson(enriched));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
    } finally {
      setSkillsLoading(false);
    }
  };

  const toggleSkill = async (skillId: string, nextEnabled: boolean) => {
    if (nextEnabled) {
      await run(() => runtimeEnableSkill(skillId));
    } else {
      await run(() => runtimeDisableSkill(skillId));
    }
    setSkills((prev) =>
      prev.map((item) =>
        item.snapshot.skill_id === skillId
          ? { ...item, enabled: nextEnabled }
          : item
      )
    );
  };

  const sendChat = async () => {
    if (!chatInput.trim()) {
      return;
    }
    const userMessage = chatInput.trim();
    setChatLog((prev) => [...prev, { role: 'user', text: userMessage }]);
    setChatInput('');
    const response = await runWithResult(() =>
      alphahumanAgentChat(
        userMessage,
        chatProvider.trim() ? chatProvider : undefined,
        chatModel.trim() ? chatModel : undefined,
        parseOptionalNumber(chatTemperature) ?? undefined
      )
    );
    if (response) {
      setChatLog((prev) => [...prev, { role: 'agent', text: response.result }]);
    }
  };

  return (
    <div className="h-full flex flex-col">
      <SettingsHeader
        title="Tauri Command Console"
        showBackButton={true}
        onBack={navigateBack}
      />

      <div className="flex-1 overflow-y-auto px-6 pb-10 space-y-6">
        {!tauriAvailable && (
          <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 px-4 py-3 text-sm text-amber-200">
            Tauri runtime not detected. Commands will fail in browser mode.
          </div>
        )}

        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Config</h3>
          <div className="flex flex-wrap gap-2">
            <button className="btn btn-primary" onClick={loadConfig}>
              Load Config
            </button>
          </div>
        </section>

        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Model API Keys</h3>
          <div className="grid gap-3 md:grid-cols-2">
            <label className="space-y-2 text-sm text-gray-300">
              API Key
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="sk-..."
                value={apiKey}
                onChange={(event) => setApiKey(event.target.value)}
              />
            </label>
            <label className="space-y-2 text-sm text-gray-300">
              API Base URL
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="https://api.openai.com/v1"
                value={apiUrl}
                onChange={(event) => setApiUrl(event.target.value)}
              />
            </label>
            <label className="space-y-2 text-sm text-gray-300">
              Default Provider
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="openai"
                value={defaultProvider}
                onChange={(event) => setDefaultProvider(event.target.value)}
              />
            </label>
            <label className="space-y-2 text-sm text-gray-300">
              Default Model
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="gpt-4.1-mini"
                value={defaultModel}
                onChange={(event) => setDefaultModel(event.target.value)}
              />
            </label>
            <label className="space-y-2 text-sm text-gray-300">
              Default Temperature
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="0.7"
                value={defaultTemp}
                onChange={(event) => setDefaultTemp(event.target.value)}
              />
            </label>
          </div>
          <button className="btn btn-primary" onClick={saveModelSettings}>
            Save Model Settings
          </button>
        </section>

        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Diagnostics</h3>
          <div className="flex flex-wrap gap-2">
            <button className="btn btn-primary" onClick={() => run(alphahumanDoctorReport)}>
              Run Doctor Report
            </button>
            <button
              className="btn btn-outline"
              onClick={() =>
                run(() =>
                  alphahumanDoctorModels(providerOverride || undefined, true)
                )
              }
            >
              Probe Models
            </button>
            <input
              className="input input-bordered w-full max-w-xs text-slate-900 bg-white"
              placeholder="Provider override (optional)"
              value={providerOverride}
              onChange={(event) => setProviderOverride(event.target.value)}
            />
          </div>
        </section>

        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Integrations</h3>
          <div className="flex flex-wrap gap-2">
            <button className="btn btn-primary" onClick={() => run(alphahumanListIntegrations)}>
              List Integrations
            </button>
            <button
              className="btn btn-outline"
              onClick={() =>
                run(() => alphahumanGetIntegrationInfo(integrationName))
              }
            >
              Get Integration Info
            </button>
            <input
              className="input input-bordered w-full max-w-xs text-slate-900 bg-white"
              placeholder="Integration name"
              value={integrationName}
              onChange={(event) => setIntegrationName(event.target.value)}
            />
          </div>
        </section>

        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Models</h3>
          <div className="flex flex-wrap gap-2">
            <button
              className="btn btn-primary"
              onClick={() =>
                run(() => alphahumanModelsRefresh(providerOverride || undefined, false))
              }
            >
              Refresh Models
            </button>
            <button
              className="btn btn-outline"
              onClick={() =>
                run(() => alphahumanModelsRefresh(providerOverride || undefined, true))
              }
            >
              Force Refresh
            </button>
          </div>
        </section>

        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Tunnels</h3>
          <div className="grid gap-3 md:grid-cols-2">
            <label className="space-y-2 text-sm text-gray-300">
              Provider
              <select
                className="select select-bordered w-full text-slate-900 bg-white"
                value={tunnelProvider}
                onChange={(event) => setTunnelProvider(event.target.value)}
              >
                <option value="none">none</option>
                <option value="cloudflare">cloudflare</option>
                <option value="ngrok">ngrok</option>
                <option value="tailscale">tailscale</option>
                <option value="custom">custom</option>
              </select>
            </label>
            <label className="space-y-2 text-sm text-gray-300">
              Cloudflare Token
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="cloudflare token"
                value={cloudflareToken}
                onChange={(event) => setCloudflareToken(event.target.value)}
              />
            </label>
            <label className="space-y-2 text-sm text-gray-300">
              Ngrok Token
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="ngrok token"
                value={ngrokToken}
                onChange={(event) => setNgrokToken(event.target.value)}
              />
            </label>
            <label className="space-y-2 text-sm text-gray-300">
              Tailscale Hostname
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="alpha.local"
                value={tailscaleHostname}
                onChange={(event) => setTailscaleHostname(event.target.value)}
              />
            </label>
            <label className="space-y-2 text-sm text-gray-300 md:col-span-2">
              Custom Start Command
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="ngrok http 3000"
                value={customCommand}
                onChange={(event) => setCustomCommand(event.target.value)}
              />
            </label>
          </div>
          <button className="btn btn-primary" onClick={saveTunnelSettings}>
            Save Tunnel Settings
          </button>
        </section>

        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Gateway</h3>
          <div className="grid gap-3 md:grid-cols-2">
            <label className="space-y-2 text-sm text-gray-300">
              Host
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="127.0.0.1"
                value={gatewayHost}
                onChange={(event) => setGatewayHost(event.target.value)}
              />
            </label>
            <label className="space-y-2 text-sm text-gray-300">
              Port
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="3000"
                value={gatewayPort}
                onChange={(event) => setGatewayPort(event.target.value)}
              />
            </label>
            <label className="flex items-center gap-2 text-sm text-gray-300">
              <input
                type="checkbox"
                className="checkbox checkbox-primary"
                checked={gatewayPairing}
                onChange={(event) => setGatewayPairing(event.target.checked)}
              />
              Require pairing
            </label>
            <label className="flex items-center gap-2 text-sm text-gray-300">
              <input
                type="checkbox"
                className="checkbox checkbox-primary"
                checked={gatewayPublic}
                onChange={(event) => setGatewayPublic(event.target.checked)}
              />
              Allow public bind
            </label>
          </div>
          <button className="btn btn-primary" onClick={saveGatewaySettings}>
            Save Gateway Settings
          </button>
        </section>

        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Memory</h3>
          <div className="grid gap-3 md:grid-cols-2">
            <label className="space-y-2 text-sm text-gray-300">
              Backend
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="sqlite"
                value={memoryBackend}
                onChange={(event) => setMemoryBackend(event.target.value)}
              />
            </label>
            <label className="flex items-center gap-2 text-sm text-gray-300">
              <input
                type="checkbox"
                className="checkbox checkbox-primary"
                checked={memoryAutoSave}
                onChange={(event) => setMemoryAutoSave(event.target.checked)}
              />
              Auto-save
            </label>
            <label className="space-y-2 text-sm text-gray-300">
              Embedding Provider
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="openai"
                value={embeddingProvider}
                onChange={(event) => setEmbeddingProvider(event.target.value)}
              />
            </label>
            <label className="space-y-2 text-sm text-gray-300">
              Embedding Model
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="text-embedding-3-small"
                value={embeddingModel}
                onChange={(event) => setEmbeddingModel(event.target.value)}
              />
            </label>
            <label className="space-y-2 text-sm text-gray-300">
              Embedding Dimensions
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="1536"
                value={embeddingDims}
                onChange={(event) => setEmbeddingDims(event.target.value)}
              />
            </label>
          </div>
          <button className="btn btn-primary" onClick={saveMemorySettings}>
            Save Memory Settings
          </button>
        </section>

        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Runtime & Skills</h3>
          <div className="grid gap-3 md:grid-cols-2">
            <label className="space-y-2 text-sm text-gray-300">
              Runtime Kind
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="native"
                value={runtimeKind}
                onChange={(event) => setRuntimeKind(event.target.value)}
              />
            </label>
            <label className="flex items-center gap-2 text-sm text-gray-300">
              <input
                type="checkbox"
                className="checkbox checkbox-primary"
                checked={reasoningEnabled}
                onChange={(event) => setReasoningEnabled(event.target.checked)}
              />
              Reasoning enabled
            </label>
          </div>
          <div className="flex flex-wrap gap-2">
            <button className="btn btn-primary" onClick={saveRuntimeSettings}>
              Save Runtime Settings
            </button>
            <button
              className="btn btn-outline"
              onClick={loadSkills}
              disabled={skillsLoading}
            >
              {skillsLoading ? 'Loading Skills…' : 'Load Skills'}
            </button>
          </div>
          {skills.length > 0 && (
            <div className="grid gap-2">
              {skills.map((item) => (
                <div
                  key={item.snapshot.skill_id}
                  className="flex items-center justify-between rounded-lg border border-white/10 bg-white/5 px-3 py-2"
                >
                  <div>
                    <div className="text-sm text-white">{item.snapshot.name}</div>
                    <div className="text-xs text-gray-400">
                      {item.snapshot.skill_id}
                    </div>
                  </div>
                  <label className="flex items-center gap-2 text-xs text-gray-300">
                    <input
                      type="checkbox"
                      className="checkbox checkbox-primary"
                      checked={item.enabled}
                      onChange={(event) =>
                        toggleSkill(item.snapshot.skill_id, event.target.checked)
                      }
                    />
                    {item.enabled ? 'Active' : 'Inactive'}
                  </label>
                </div>
              ))}
            </div>
          )}
        </section>

        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Migration</h3>
          <div className="flex flex-wrap gap-2">
            <button
              className="btn btn-primary"
              onClick={() =>
                run(() =>
                  alphahumanMigrateOpenclaw(
                    migrationSource || undefined,
                    true
                  )
                )
              }
            >
              Dry Run Migration
            </button>
            <button
              className="btn btn-outline"
              onClick={() =>
                run(() =>
                  alphahumanMigrateOpenclaw(
                    migrationSource || undefined,
                    false
                  )
                )
              }
            >
              Run Migration
            </button>
            <input
              className="input input-bordered w-full max-w-md text-slate-900 bg-white"
              placeholder="Source workspace (optional)"
              value={migrationSource}
              onChange={(event) => setMigrationSource(event.target.value)}
            />
          </div>
        </section>

        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Hardware</h3>
          <div className="flex flex-wrap gap-2">
            <button className="btn btn-primary" onClick={() => run(alphahumanHardwareDiscover)}>
              Discover Devices
            </button>
            <button
              className="btn btn-outline"
              onClick={() =>
                run(() => alphahumanHardwareIntrospect(hardwarePath))
              }
            >
              Introspect Device
            </button>
            <input
              className="input input-bordered w-full max-w-md text-slate-900 bg-white"
              placeholder="Device path (e.g. /dev/tty.usbmodem)"
              value={hardwarePath}
              onChange={(event) => setHardwarePath(event.target.value)}
            />
          </div>
        </section>

        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Service</h3>
          <div className="flex flex-wrap gap-2">
            <button className="btn btn-primary" onClick={() => run(alphahumanServiceStatus)}>
              Status
            </button>
            <button className="btn btn-outline" onClick={() => run(alphahumanServiceInstall)}>
              Install
            </button>
            <button className="btn btn-outline" onClick={() => run(alphahumanServiceStart)}>
              Start
            </button>
            <button className="btn btn-outline" onClick={() => run(alphahumanServiceStop)}>
              Stop
            </button>
            <button className="btn btn-outline" onClick={() => run(alphahumanServiceUninstall)}>
              Uninstall
            </button>
          </div>
        </section>

        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Secrets</h3>
          <div className="grid gap-3 md:grid-cols-2">
            <div className="space-y-2">
              <label className="text-sm text-gray-300">Encrypt</label>
              <textarea
                className="textarea textarea-bordered w-full min-h-[90px] text-slate-900 bg-white"
                placeholder="Plaintext"
                value={encryptInput}
                onChange={(event) => setEncryptInput(event.target.value)}
              />
              <button
                className="btn btn-primary"
                onClick={() => run(() => alphahumanEncryptSecret(encryptInput))}
              >
                Encrypt
              </button>
            </div>
            <div className="space-y-2">
              <label className="text-sm text-gray-300">Decrypt</label>
              <textarea
                className="textarea textarea-bordered w-full min-h-[90px] text-slate-900 bg-white"
                placeholder="Ciphertext"
                value={decryptInput}
                onChange={(event) => setDecryptInput(event.target.value)}
              />
              <button
                className="btn btn-outline"
                onClick={() => run(() => alphahumanDecryptSecret(decryptInput))}
              >
                Decrypt
              </button>
            </div>
          </div>
        </section>

        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Agent Chat</h3>
          <div className="grid gap-3 md:grid-cols-3">
            <label className="space-y-2 text-sm text-gray-300">
              Provider Override
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="openai"
                value={chatProvider}
                onChange={(event) => setChatProvider(event.target.value)}
              />
            </label>
            <label className="space-y-2 text-sm text-gray-300">
              Model Override
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="gpt-4.1-mini"
                value={chatModel}
                onChange={(event) => setChatModel(event.target.value)}
              />
            </label>
            <label className="space-y-2 text-sm text-gray-300">
              Temperature
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="0.7"
                value={chatTemperature}
                onChange={(event) => setChatTemperature(event.target.value)}
              />
            </label>
          </div>
          <textarea
            className="textarea textarea-bordered w-full min-h-[120px] text-slate-900 bg-white"
            placeholder="Send a message to the agent..."
            value={chatInput}
            onChange={(event) => setChatInput(event.target.value)}
          />
          <button className="btn btn-primary" onClick={sendChat}>
            Send Message
          </button>
          {chatLog.length > 0 && (
            <div className="space-y-2 rounded-lg border border-white/10 bg-white/5 p-3">
              {chatLog.map((entry, index) => (
                <div
                  key={`${entry.role}-${index}`}
                  className={`text-sm ${
                    entry.role === 'user' ? 'text-white' : 'text-emerald-200'
                  }`}
                >
                  <span className="font-semibold uppercase text-[10px] tracking-wide">
                    {entry.role}
                  </span>
                  <div className="whitespace-pre-wrap">{entry.text}</div>
                </div>
              ))}
            </div>
          )}
        </section>

        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Output</h3>
          {error && (
            <div className="rounded-lg border border-red-500/40 bg-red-500/10 px-4 py-3 text-sm text-red-200">
              {error}
            </div>
          )}
          <textarea
            className="textarea textarea-bordered w-full min-h-[240px] font-mono text-xs text-slate-900 bg-white"
            value={output}
            readOnly
          />
        </section>
      </div>
    </div>
  );
};

export default TauriCommandsPanel;

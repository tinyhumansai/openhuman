import { useEffect, useState } from 'react';
import {
  openhumanGetConfig,
  openhumanUpdateModelSettings,
} from '../../../../utils/tauriCommands/config';

const CustomModelSection = () => {
  const [apiUrl, setApiUrl] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState('');
  const [saveSuccess, setSaveSuccess] = useState(false);

  useEffect(() => {
    let mounted = true;
    const fetchConfig = async () => {
      try {
        const { result } = await openhumanGetConfig();
        if (mounted) {
          setApiUrl(String(result.config.api_url || ''));
          setApiKey(String(result.config.api_key || ''));
        }
      } catch (err) {
        if (mounted) {
          setError(err instanceof Error ? err.message : 'Failed to load custom backend config');
        }
      } finally {
        if (mounted) setIsLoading(false);
      }
    };
    void fetchConfig();
    return () => {
      mounted = false;
    };
  }, []);

  const handleSave = async () => {
    setIsSaving(true);
    setError('');
    setSaveSuccess(false);
    try {
      await openhumanUpdateModelSettings({
        api_url: apiUrl.trim() || null,
        api_key: apiKey.trim() || null,
      });
      setSaveSuccess(true);
      setTimeout(() => setSaveSuccess(false), 3000);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save custom backend settings');
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <section className="space-y-3">
      <h3 className="text-sm font-semibold text-stone-900">Custom Provider (OpenAI Compatible)</h3>
      <div className="bg-stone-50 rounded-lg border border-stone-200 p-4 space-y-4">
        <p className="text-xs text-stone-500">
          Configure a custom OpenAI-compatible backend (like vLLM or LiteLLM). If set, this will route requests to your custom endpoint instead of the standard local or built-in backends.
        </p>

        {isLoading ? (
          <div className="text-sm text-stone-500 animate-pulse">Loading settings…</div>
        ) : (
          <div className="space-y-3">
            <div>
              <label className="block text-xs font-medium text-stone-700 mb-1">
                Base URL
              </label>
              <input
                type="text"
                value={apiUrl}
                onChange={e => setApiUrl(e.target.value)}
                placeholder="http://localhost:8000/v1"
                className="w-full rounded-md border border-stone-300 bg-white px-3 py-2 text-sm text-stone-900 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
              />
            </div>
            <div>
              <label className="block text-xs font-medium text-stone-700 mb-1">
                API Key
              </label>
              <input
                type="password"
                value={apiKey}
                onChange={e => setApiKey(e.target.value)}
                placeholder="sk-..."
                className="w-full rounded-md border border-stone-300 bg-white px-3 py-2 text-sm text-stone-900 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
              />
            </div>

            <div className="flex items-center gap-3 pt-2">
              <button
                type="button"
                onClick={handleSave}
                disabled={isSaving}
                className="rounded-lg bg-primary-600 px-4 py-2 text-sm font-medium text-white hover:bg-primary-700 focus:outline-none disabled:opacity-50">
                {isSaving ? 'Saving…' : 'Save Config'}
              </button>
              {saveSuccess && <span className="text-xs text-green-600">Saved successfully.</span>}
            </div>

            {error && <div className="text-xs text-red-600 mt-2">{error}</div>}
          </div>
        )}
      </div>
    </section>
  );
};

export default CustomModelSection;

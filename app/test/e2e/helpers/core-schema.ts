import { resolveCoreRpcUrl } from './core-rpc-node';

export interface RpcMethodSchema {
  method: string;
  namespace: string;
  function: string;
  description: string;
  inputs: unknown[];
  outputs: unknown[];
}

interface HttpSchemaDump {
  methods: RpcMethodSchema[];
}

export async function fetchCoreSchemaDump(): Promise<HttpSchemaDump> {
  const rpcUrl = await resolveCoreRpcUrl();
  const schemaUrl = rpcUrl.replace(/\/rpc\/?$/, '/schema');
  const res = await fetch(schemaUrl, { method: 'GET' });
  if (!res.ok) {
    const body = await res.text();
    throw new Error(`schema fetch failed (${res.status}): ${body.slice(0, 240)}`);
  }
  return (await res.json()) as HttpSchemaDump;
}

export async function fetchCoreRpcMethods(): Promise<Set<string>> {
  const dump = await fetchCoreSchemaDump();
  return new Set((dump.methods || []).map(entry => entry.method));
}

export function expectRpcMethod(methods: Set<string>, method: string): void {
  expect(methods.has(method)).toBe(true);
}

#!/usr/bin/env node
// CEF runtime capability diagnostic — connects to the embedded webview's
// CDP debug port (`localhost:19222`, set by `lib.rs:1201`) and runs one
// of three probes against an active provider target. Surfaced during
// #1053 Phase A diagnostic but reusable for any CEF runtime audit
// (Web Push gap, BrowserChannel long-poll, codec demux, etc — see
// `feedback_cef_runtime_gaps.md`).
//
// Run while `pnpm dev:app` is up and a provider webview (gmeet, gmail,
// slack, …) has loaded its real URL — the harness picks the first target
// whose URL or title contains "meet" by default; tweak `pickGmeetTarget`
// to scope to a different provider.
//
// Usage:
//   node scripts/diagnose-cef-runtime.mjs probe     # capability-gate snapshot
//                                                   #   (crossOriginIsolated, SAB,
//                                                   #    insertable streams,
//                                                   #    WebGL2 / WebGPU, Atomics)
//   node scripts/diagnose-cef-runtime.mjs headers   # tail Network.responseReceived
//                                                   #   for COOP / COEP / CORP +
//                                                   #   any provider response (Ctrl-C dumps)
//   node scripts/diagnose-cef-runtime.mjs watch     # tail Console.messageAdded +
//                                                   #   Runtime.exceptionThrown
//                                                   #   (Ctrl-C dumps)
//
// Output goes to ./diagnosis-<mode>-<timestamp>.json. The transient JSONs
// are intentionally NOT committed (`.gitignore` excludes them).

import { writeFileSync } from 'node:fs';
import { setTimeout as sleep } from 'node:timers/promises';
import { argv } from 'node:process';

const CDP_HOST = 'localhost';
const CDP_PORT = 19222;

const ts = () => new Date().toISOString().replace(/[:.]/g, '-');
const out = (mode, data) => {
  const path = `diagnosis-${mode}-${ts()}.json`;
  writeFileSync(path, JSON.stringify(data, null, 2));
  console.log(`[diagnose] wrote ${path}`);
};

async function listTargets() {
  const res = await fetch(`http://${CDP_HOST}:${CDP_PORT}/json/list`);
  if (!res.ok) throw new Error(`json/list ${res.status}`);
  return await res.json();
}

async function pickGmeetTarget() {
  const targets = await listTargets();
  const gmeet = targets.filter(
    (t) =>
      t.type === 'page' &&
      (t.url?.includes('meet.google.com') || t.title?.toLowerCase().includes('meet')),
  );
  if (!gmeet.length) {
    console.error('[diagnose] No meet.google.com target found. All page targets:');
    for (const t of targets.filter((t) => t.type === 'page')) {
      console.error(`  ${t.title} :: ${t.url}`);
    }
    process.exit(2);
  }
  return gmeet[0];
}

async function attach(target) {
  const ws = await import('ws').catch(() => null);
  if (!ws) {
    console.error('[diagnose] missing ws — install via: pnpm add -D ws (or copy to /tmp + npm i ws)');
    process.exit(2);
  }
  const WebSocket = ws.default ?? ws.WebSocket;
  const sock = new WebSocket(target.webSocketDebuggerUrl);
  let id = 0;
  const pending = new Map();
  const events = [];
  sock.on('message', (raw) => {
    const msg = JSON.parse(raw.toString());
    if (msg.id != null && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      msg.error ? reject(msg.error) : resolve(msg.result);
    } else if (msg.method) {
      events.push({ at: ts(), ...msg });
    }
  });
  await new Promise((resolve, reject) => {
    sock.once('open', resolve);
    sock.once('error', reject);
  });
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const i = ++id;
      pending.set(i, { resolve, reject });
      sock.send(JSON.stringify({ id: i, method, params }));
    });
  return { send, events, close: () => sock.close() };
}

async function modeProbe() {
  const target = await pickGmeetTarget();
  console.log(`[diagnose] probe :: ${target.url}`);
  const cdp = await attach(target);
  const expr = `JSON.stringify({
    crossOriginIsolated: window.crossOriginIsolated,
    hasSAB: typeof SharedArrayBuffer === 'function',
    hasInsertableStreams: 'MediaStreamTrackProcessor' in window && 'MediaStreamTrackGenerator' in window,
    hasWebGL2: !!document.createElement('canvas').getContext('webgl2'),
    hasWebGPU: 'gpu' in navigator,
    hasAtomics: typeof Atomics === 'object',
    workerSAB: (() => {
      try { return new Worker(URL.createObjectURL(new Blob(['postMessage(typeof SharedArrayBuffer)'], {type:'text/javascript'}))) ? 'spawned' : 'no'; }
      catch (e) { return 'err: ' + e.message; }
    })(),
    coi: { coop: document.featurePolicy?.allowsFeature?.('cross-origin-isolated'), policy: 'see-network' },
    ua: navigator.userAgent,
    href: location.href,
  })`;
  const result = await cdp.send('Runtime.evaluate', { expression: expr, returnByValue: true });
  const parsed = JSON.parse(result.result.value);
  out('probe', parsed);
  cdp.close();
}

async function modeWatch() {
  const target = await pickGmeetTarget();
  console.log(`[diagnose] watch :: ${target.url} (Ctrl-C to stop)`);
  const cdp = await attach(target);
  await cdp.send('Console.enable');
  await cdp.send('Runtime.enable');
  await cdp.send('Log.enable');
  console.log('[diagnose] subscribed. Click Effects → Background NOW.');
  process.on('SIGINT', () => {
    out('watch', cdp.events);
    cdp.close();
    process.exit(0);
  });
  await new Promise(() => {}); // hang
}

// URL matchers for `headers` mode. Includes gstatic asset path so the dump
// captures `www.gstatic.com/video_effects/assets/*.mp4` — the requests
// implicated in the dynamic-background failure (#1053 Phase A).
const HEADERS_URL_MATCHERS = [
  (u) => u.includes('meet.google.com'),
  (u) => u.includes('gstatic.com/video_effects/assets/'),
];

async function modeHeaders() {
  const target = await pickGmeetTarget();
  console.log(`[diagnose] headers :: ${target.url}`);
  const cdp = await attach(target);
  await cdp.send('Network.enable');
  const seen = [];
  process.on('SIGINT', () => {
    out('headers', seen);
    cdp.close();
    process.exit(0);
  });
  cdp.events.length = 0;
  setInterval(() => {
    for (const ev of cdp.events.splice(0)) {
      if (ev.method === 'Network.responseReceived') {
        const r = ev.params.response;
        if (HEADERS_URL_MATCHERS.some((matches) => matches(r.url))) {
          seen.push({
            url: r.url,
            status: r.status,
            mimeType: r.mimeType,
            coop: r.headers['Cross-Origin-Opener-Policy'] ?? r.headers['cross-origin-opener-policy'],
            coep: r.headers['Cross-Origin-Embedder-Policy'] ?? r.headers['cross-origin-embedder-policy'],
            corp: r.headers['Cross-Origin-Resource-Policy'] ?? r.headers['cross-origin-resource-policy'],
          });
          console.log(`[diagnose] ${r.status} ${r.url} coop=${seen[seen.length - 1].coop} coep=${seen[seen.length - 1].coep}`);
        }
      }
    }
  }, 500);
  console.log('[diagnose] capturing meet.google.com + gstatic video_effects responses (Ctrl-C to dump).');
  await new Promise(() => {});
}

const mode = argv[2];
if (mode === 'probe') await modeProbe();
else if (mode === 'watch') await modeWatch();
else if (mode === 'headers') await modeHeaders();
else {
  console.error('Usage: diagnose-cef-runtime.mjs <probe|watch|headers>');
  process.exit(1);
}

// OpenHuman runtime-pool Node worker harness (issue #5106).
//
// A single long-lived `node` process that executes inline JavaScript jobs for
// many skill runs / node_exec calls, so the fleet pays one warm interpreter
// instead of one child per run.
//
// Protocol (newline-delimited JSON over an authenticated loopback socket,
// see runtime_pool/protocol.rs):
//   1. Print exactly one ready line:  {"ready":true,"protocol":1,"lang":"node"}
//   2. For each request line {id,kind:"inline",code,cwd,timeout_ms} reply with
//      {id,ok,stdout,stderr,exit_code,timed_out,elapsed_ms,error}.
//
// Each job runs in its own `worker_thread` for isolation (fresh module graph +
// globals per run) and safe termination (a runaway or process.exit()-y job is
// killed with worker.terminate() without taking down this host process). The
// job's stdout/stderr are isolated pipes (stdout:true/stderr:true). Protocol
// replies use a separate authenticated loopback socket so fd-level writes
// (`fs.writeSync`, inherited child stdio) cannot forge or corrupt frames.

'use strict';

const { Worker, isMainThread, parentPort, workerData } = require('worker_threads');

const PROTOCOL_VERSION = 1;

// ---------------------------------------------------------------------------
// Worker-thread mode: execute one job's code, then exit (flushing its pipes).
// ---------------------------------------------------------------------------
if (!isMainThread) {
  const path = require('path');
  const vm = require('vm');
  const { createRequire } = require('module');
  const { pathToFileURL } = require('url');

  async function runUserCode(code, cwd) {
    // NOTE: do NOT `process.chdir()` here — it throws ERR_WORKER_UNSUPPORTED_
    // OPERATION inside a worker thread. The host chdirs before spawning this
    // worker (jobs serialize per worker process), so `process.cwd()` is already
    // the job's directory; `cwd` roots require/__dirname and the import base.
    const dir = cwd || process.cwd();
    const filename = path.join(dir, 'inline.js');
    const req = createRequire(filename);
    const base = pathToFileURL(filename).href;
    // Use the ESM resolver (not createRequire.resolve) for bare dynamic
    // imports so import-only package exports retain `node -e` semantics.
    const { default: resolveEsm } = await import(
      'data:text/javascript,export default (specifier, parent) => import.meta.resolve(specifier, parent)'
    );
    const importFromJob = (specifier, _referrer, importAttributes) => {
      const options =
        importAttributes && Object.keys(importAttributes).length > 0
          ? { with: importAttributes }
          : undefined;
      if (
        specifier.startsWith('.') ||
        specifier.startsWith('/') ||
        specifier.startsWith('file:') ||
        specifier.startsWith('data:')
      ) {
        return import(new URL(specifier, base).href, options);
      }
      return import(resolveEsm(specifier, base), options);
    };
    // Mimic `node -e`: CommonJS-ish sloppy scope with require/__dirname, wrapped
    // in an async IIFE so top-level `await` works. `vm.compileFunction` (over a
    // bare `new Function`) lets us root dynamic `import()` at the job cwd via
    // `importModuleDynamically`, so `await import('./rel.mjs')` resolves like
    // `node -e` instead of relative to this harness file. Needs
    // `--experimental-vm-modules` (passed on the worker launch).
    const fn = vm.compileFunction(
      'return (async () => {\n' + code + '\n})();',
      ['require', '__filename', '__dirname', 'module', 'exports'],
      {
        filename,
        importModuleDynamically: importFromJob,
      }
    );
    const mod = { exports: {} };
    await fn(req, filename, dir, mod, mod.exports);
  }

  const code = (workerData && workerData.code) || '';
  const cwd = (workerData && workerData.cwd) || null;
  runUserCode(code, cwd).then(
    () => {
      // Resolve → let the thread exit naturally once its loop drains, which
      // flushes the stdout/stderr pipes before the 'exit' event fires.
    },
    (err) => {
      const msg = err && err.stack ? err.stack : String(err);
      process.stderr.write(msg + '\n');
      process.exitCode = 1;
    }
  );
  return;
}

// ---------------------------------------------------------------------------
// Main (host) mode: read jobs, run each in a worker thread, reply per job.
// ---------------------------------------------------------------------------

function collect(stream) {
  return new Promise((resolve) => {
    let buf = '';
    stream.setEncoding('utf8');
    stream.on('data', (d) => {
      buf += d;
    });
    const done = () => resolve(buf);
    stream.on('end', done);
    stream.on('close', done);
    stream.on('error', done);
  });
}

function runJob(job) {
  return new Promise((resolve) => {
    const start = Date.now();
    // Set the job's working directory on the HOST before spawning the worker:
    // a worker thread inherits the parent's cwd at creation and cannot chdir
    // itself. The worker captures cwd synchronously at construction, so we
    // restore the host's prior cwd immediately after — otherwise a later job
    // without `cwd` (or whose chdir failed) would silently inherit this job's
    // directory instead of the worker's original one.
    const priorCwd = process.cwd();
    if (job.cwd) {
      try {
        process.chdir(job.cwd);
      } catch (e) {
        resolve({
          id: job.id,
          ok: false,
          stdout: '',
          stderr: '',
          exit_code: null,
          timed_out: false,
          elapsed_ms: Date.now() - start,
          error: 'failed to set worker cwd: ' + (e && e.stack ? e.stack : String(e)),
        });
        return;
      }
    }
    let worker;
    try {
      worker = new Worker(__filename, {
        workerData: { id: job.id, code: job.code || '', cwd: job.cwd || null },
        stdout: true,
        stderr: true,
        // Propagate host node flags (e.g. --experimental-vm-modules) so the
        // worker's vm.compileFunction dynamic-import hook is enabled.
        execArgv: process.execArgv,
      });
    } catch (e) {
      try {
        process.chdir(priorCwd);
      } catch (_e) {
        /* best-effort restore */
      }
      resolve({
        id: job.id,
        ok: false,
        stdout: '',
        stderr: '',
        exit_code: null,
        timed_out: false,
        elapsed_ms: Date.now() - start,
        error: 'failed to spawn worker thread: ' + (e && e.stack ? e.stack : String(e)),
      });
      return;
    }

    const outP = collect(worker.stdout);
    const errP = collect(worker.stderr);
    let exitCode = 0;
    let timedOut = false;
    let extraErr = '';

    let timer = null;
    if (job.timeout_ms && job.timeout_ms > 0) {
      timer = setTimeout(() => {
        timedOut = true;
        worker.terminate();
      }, job.timeout_ms);
    }

    worker.on('error', (e) => {
      extraErr += (e && e.stack ? e.stack : String(e)) + '\n';
      if (!exitCode) exitCode = 1;
    });

    worker.on('exit', async (code) => {
      if (timer) clearTimeout(timer);
      // Restore the host cwd only now: a worker thread reads its cwd
      // asynchronously as it initializes, so the host must stay at `job.cwd`
      // for the worker's whole life. Jobs are serialized, so the next job
      // starts from this restored (prior) directory rather than inheriting
      // this one's.
      try {
        process.chdir(priorCwd);
      } catch (_e) {
        /* best-effort restore */
      }
      if (code && !exitCode) exitCode = code;
      const stdout = await outP;
      const stderr = (await errP) + extraErr;
      resolve({
        id: job.id,
        ok: true,
        stdout,
        stderr,
        exit_code: timedOut ? null : exitCode,
        timed_out: timedOut,
        elapsed_ms: Date.now() - start,
        error: null,
      });
    });
  });
}

let protocolInput = process.stdin;
let protocolStream = process.stdout;

function reply(obj) {
  protocolStream.write(JSON.stringify(obj) + '\n');
}

function serve() {
  // Announce readiness, then serve jobs one at a time (the Rust pool already
  // sends at most one outstanding job per worker; the chain keeps ordering).
  reply({
    ready: true,
    protocol: PROTOCOL_VERSION,
    lang: 'node',
    protocol_token: process.env.OPENHUMAN_RUNTIME_POOL_PROTOCOL_TOKEN || null,
  });

  const readline = require('readline');
  const rl = readline.createInterface({ input: protocolInput });
  let chain = Promise.resolve();
  rl.on('line', (line) => {
    const trimmed = line.trim();
    if (!trimmed) return;
    let job;
    try {
      job = JSON.parse(trimmed);
    } catch (_e) {
      return; // ignore unparseable lines
    }
    chain = chain
      .then(() => runJob(job))
      .then((res) => reply(res))
      .catch((e) => {
        reply({
          id: job && job.id,
          ok: false,
          stdout: '',
          stderr: '',
          exit_code: null,
          timed_out: false,
          elapsed_ms: 0,
          error: String((e && e.stack) || e),
        });
      });
  });
  rl.on('close', () => {
    // Drain any in-flight job before exiting so a closed stdin doesn't drop work
    // that was already accepted onto the chain.
    Promise.resolve(chain).finally(() => process.exit(0));
  });
}

const protocolAddr = process.env.OPENHUMAN_RUNTIME_POOL_PROTOCOL_ADDR;
if (protocolAddr) {
  const net = require('net');
  const split = protocolAddr.lastIndexOf(':');
  const host = protocolAddr.slice(0, split);
  const port = Number(protocolAddr.slice(split + 1));
  const socket = net.createConnection({ host, port });
  socket.once('connect', () => {
    protocolInput = socket;
    protocolStream = socket;
    serve();
  });
  socket.once('error', (e) => {
    process.stderr.write('runtime pool protocol connection failed: ' + String(e) + '\n');
    process.exit(1);
  });
} else {
  // Backward-compatible developer launch; production Node workers always use
  // the isolated socket configured by Rust.
  serve();
}

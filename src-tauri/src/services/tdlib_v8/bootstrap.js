/**
 * Bootstrap JavaScript for V8 Runtime
 *
 * Provides browser-like API shims that tdweb expects.
 * These shims call Rust "ops" for actual I/O.
 */

// Make globalThis.self and globalThis.window point to globalThis
globalThis.self = globalThis;
globalThis.window = globalThis;

// ============================================================================
// Console (uses Rust logging)
// ============================================================================
globalThis.console = {
  log: function (...args) {
    Deno.core.ops.op_console_log(args.map(String).join(' '));
  },
  info: function (...args) {
    Deno.core.ops.op_console_log(args.map(String).join(' '));
  },
  warn: function (...args) {
    Deno.core.ops.op_console_warn(args.map(String).join(' '));
  },
  error: function (...args) {
    Deno.core.ops.op_console_error(args.map(String).join(' '));
  },
  debug: function (...args) {
    Deno.core.ops.op_console_log('[DEBUG] ' + args.map(String).join(' '));
  },
};

// ============================================================================
// Timers (setTimeout, setInterval, clearTimeout, clearInterval)
// ============================================================================
const timerCallbacks = new Map();
let nextTimerId = 1;

globalThis.setTimeout = function (callback, delay, ...args) {
  const id = nextTimerId++;
  timerCallbacks.set(id, { callback, args, type: 'timeout' });
  Deno.core.ops.op_ah_timer_start(id, delay || 0, false);
  return id;
};

globalThis.setInterval = function (callback, delay, ...args) {
  const id = nextTimerId++;
  timerCallbacks.set(id, { callback, args, type: 'interval' });
  Deno.core.ops.op_ah_timer_start(id, delay || 0, true);
  return id;
};

globalThis.clearTimeout = function (id) {
  timerCallbacks.delete(id);
  Deno.core.ops.op_ah_timer_cancel(id);
};

globalThis.clearInterval = function (id) {
  timerCallbacks.delete(id);
  Deno.core.ops.op_ah_timer_cancel(id);
};

// Timer callback handler (called from Rust)
globalThis.__handleTimer = function (id) {
  const timer = timerCallbacks.get(id);
  if (timer) {
    if (timer.type === 'timeout') {
      timerCallbacks.delete(id);
    }
    try {
      timer.callback.apply(null, timer.args);
    } catch (e) {
      console.error('Timer callback error:', e);
    }
  }
};

// ============================================================================
// AbortController / AbortSignal Polyfill
// ============================================================================
class AbortSignal {
  constructor() {
    this.aborted = false;
    this.reason = undefined;
    this._listeners = [];
  }

  addEventListener(type, listener) {
    if (type === 'abort') {
      this._listeners.push(listener);
    }
  }

  removeEventListener(type, listener) {
    if (type === 'abort') {
      const idx = this._listeners.indexOf(listener);
      if (idx >= 0) this._listeners.splice(idx, 1);
    }
  }

  throwIfAborted() {
    if (this.aborted) {
      throw this.reason || new Error('Aborted');
    }
  }
}

class AbortController {
  constructor() {
    this.signal = new AbortSignal();
  }

  abort(reason) {
    if (!this.signal.aborted) {
      this.signal.aborted = true;
      this.signal.reason = reason || new Error('Aborted');
      for (const listener of this.signal._listeners) {
        try {
          listener({ type: 'abort', target: this.signal });
        } catch (e) {
          console.error('AbortController listener error:', e);
        }
      }
    }
  }
}

globalThis.AbortController = AbortController;
globalThis.AbortSignal = AbortSignal;

// ============================================================================
// Fetch API
// ============================================================================
globalThis.fetch = async function (url, options = {}) {
  const method = options.method || 'GET';
  const headers = options.headers || {};
  const body = options.body || null;

  // Convert Headers object to plain object if needed
  let headersObj = {};
  if (headers instanceof Headers) {
    headers.forEach((value, key) => {
      headersObj[key] = value;
    });
  } else {
    headersObj = headers;
  }

  const result = await Deno.core.ops.op_fetch(url.toString(), {
    method,
    headers: headersObj,
    body: typeof body === 'string' ? body : body ? JSON.stringify(body) : null,
  });

  return new Response(result.body, {
    status: result.status,
    statusText: result.statusText || '',
    headers: new Headers(result.headers),
  });
};

// Response class for fetch
class Response {
  constructor(body, init = {}) {
    this._body = body;
    this.status = init.status || 200;
    this.statusText = init.statusText || '';
    this.headers = init.headers || new Headers();
    this.ok = this.status >= 200 && this.status < 300;
  }

  async text() {
    return this._body;
  }

  async json() {
    return JSON.parse(this._body);
  }

  async arrayBuffer() {
    // Convert string to ArrayBuffer
    const encoder = new TextEncoder();
    return encoder.encode(this._body).buffer;
  }

  async blob() {
    throw new Error('Blob not supported');
  }
}

// Headers class for fetch
class Headers {
  constructor(init = {}) {
    this._headers = {};
    if (init) {
      if (typeof init.forEach === 'function') {
        init.forEach((value, key) => {
          this._headers[key.toLowerCase()] = value;
        });
      } else {
        Object.entries(init).forEach(([key, value]) => {
          this._headers[key.toLowerCase()] = value;
        });
      }
    }
  }

  get(name) {
    return this._headers[name.toLowerCase()] || null;
  }

  set(name, value) {
    this._headers[name.toLowerCase()] = value;
  }

  has(name) {
    return name.toLowerCase() in this._headers;
  }

  delete(name) {
    delete this._headers[name.toLowerCase()];
  }

  forEach(callback) {
    Object.entries(this._headers).forEach(([key, value]) => {
      callback(value, key, this);
    });
  }
}

globalThis.Response = Response;
globalThis.Headers = Headers;

// ============================================================================
// WebSocket API
// ============================================================================
class WebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;

  constructor(url, protocols) {
    this.url = url;
    this.protocols = protocols;
    this.readyState = WebSocket.CONNECTING;
    this.binaryType = 'blob';
    this._id = null;

    // Event handlers
    this.onopen = null;
    this.onclose = null;
    this.onerror = null;
    this.onmessage = null;

    // Connect asynchronously
    this._connect();
  }

  async _connect() {
    try {
      this._id = await Deno.core.ops.op_ws_connect(this.url);
      this.readyState = WebSocket.OPEN;

      // Register for message callbacks
      WebSocket._instances.set(this._id, this);

      if (this.onopen) {
        this.onopen({ type: 'open', target: this });
      }

      // Start receiving messages
      this._startReceiving();
    } catch (e) {
      this.readyState = WebSocket.CLOSED;
      if (this.onerror) {
        this.onerror({ type: 'error', error: e, target: this });
      }
    }
  }

  async _startReceiving() {
    while (this.readyState === WebSocket.OPEN) {
      try {
        const message = await Deno.core.ops.op_ws_recv(this._id);
        if (message === null) {
          // Connection closed
          this._handleClose(1000, '');
          break;
        }

        if (this.onmessage) {
          this.onmessage({ type: 'message', data: message, target: this });
        }
      } catch (e) {
        if (this.readyState === WebSocket.OPEN) {
          this._handleClose(1006, e.toString());
        }
        break;
      }
    }
  }

  _handleClose(code, reason) {
    this.readyState = WebSocket.CLOSED;
    WebSocket._instances.delete(this._id);

    if (this.onclose) {
      this.onclose({
        type: 'close',
        code,
        reason,
        wasClean: code === 1000,
        target: this,
      });
    }
  }

  send(data) {
    if (this.readyState !== WebSocket.OPEN) {
      throw new Error('WebSocket is not open');
    }

    const dataStr = typeof data === 'string' ? data : JSON.stringify(data);
    Deno.core.ops.op_ws_send(this._id, dataStr);
  }

  close(code = 1000, reason = '') {
    if (this.readyState === WebSocket.CLOSING || this.readyState === WebSocket.CLOSED) {
      return;
    }

    this.readyState = WebSocket.CLOSING;
    Deno.core.ops.op_ws_close(this._id, code, reason);
  }
}

WebSocket._instances = new Map();
globalThis.WebSocket = WebSocket;

// ============================================================================
// IndexedDB API (for tdweb persistence)
// ============================================================================
class IDBFactory {
  open(name, version = 1) {
    return new IDBOpenDBRequest(name, version);
  }

  deleteDatabase(name) {
    const request = new IDBRequest();
    (async () => {
      try {
        await Deno.core.ops.op_idb_delete_database(name);
        request._success(undefined);
      } catch (e) {
        request._error(e);
      }
    })();
    return request;
  }
}

class IDBRequest {
  constructor() {
    this.result = undefined;
    this.error = null;
    this.readyState = 'pending';
    this.onsuccess = null;
    this.onerror = null;
  }

  _success(result) {
    this.result = result;
    this.readyState = 'done';
    if (this.onsuccess) {
      this.onsuccess({ type: 'success', target: this });
    }
  }

  _error(error) {
    this.error = error;
    this.readyState = 'done';
    if (this.onerror) {
      this.onerror({ type: 'error', target: this });
    }
  }
}

class IDBOpenDBRequest extends IDBRequest {
  constructor(name, version) {
    super();
    this.onupgradeneeded = null;
    this.onblocked = null;
    this._name = name;
    this._version = version;
    this._open();
  }

  async _open() {
    try {
      const info = await Deno.core.ops.op_idb_open(this._name, this._version);
      const db = new IDBDatabase(this._name, this._version, info.objectStores || []);

      if (info.needsUpgrade) {
        if (this.onupgradeneeded) {
          const event = {
            type: 'upgradeneeded',
            target: this,
            oldVersion: info.oldVersion || 0,
            newVersion: this._version,
          };
          // Temporarily set result for upgrade
          this.result = db;
          this.onupgradeneeded(event);
        }
      }

      this._success(db);
    } catch (e) {
      this._error(e);
    }
  }
}

class IDBDatabase {
  constructor(name, version, objectStoreNames) {
    this.name = name;
    this.version = version;
    this.objectStoreNames = objectStoreNames;
    this.onclose = null;
    this.onerror = null;
    this.onversionchange = null;
  }

  createObjectStore(name, options = {}) {
    Deno.core.ops.op_idb_create_object_store(this.name, name, options);
    this.objectStoreNames.push(name);
    return new IDBObjectStore(this, name);
  }

  deleteObjectStore(name) {
    Deno.core.ops.op_idb_delete_object_store(this.name, name);
    const idx = this.objectStoreNames.indexOf(name);
    if (idx >= 0) this.objectStoreNames.splice(idx, 1);
  }

  transaction(storeNames, mode = 'readonly') {
    return new IDBTransaction(this, storeNames, mode);
  }

  close() {
    Deno.core.ops.op_idb_close(this.name);
  }
}

class IDBTransaction {
  constructor(db, storeNames, mode) {
    this.db = db;
    this.mode = mode;
    this.error = null;
    this.oncomplete = null;
    this.onerror = null;
    this.onabort = null;
    this._storeNames = Array.isArray(storeNames) ? storeNames : [storeNames];
  }

  objectStore(name) {
    if (!this._storeNames.includes(name)) {
      throw new Error(`Object store "${name}" not in transaction scope`);
    }
    return new IDBObjectStore(this.db, name, this);
  }

  abort() {
    if (this.onabort) {
      this.onabort({ type: 'abort', target: this });
    }
  }

  _complete() {
    if (this.oncomplete) {
      this.oncomplete({ type: 'complete', target: this });
    }
  }
}

class IDBObjectStore {
  constructor(db, name, transaction = null) {
    this._db = db;
    this.name = name;
    this._transaction = transaction;
  }

  get(key) {
    const request = new IDBRequest();
    (async () => {
      try {
        const value = await Deno.core.ops.op_idb_get(this._db.name, this.name, key);
        request._success(value);
      } catch (e) {
        request._error(e);
      }
    })();
    return request;
  }

  put(value, key) {
    const request = new IDBRequest();
    (async () => {
      try {
        await Deno.core.ops.op_idb_put(this._db.name, this.name, key, value);
        request._success(key);
      } catch (e) {
        request._error(e);
      }
    })();
    return request;
  }

  add(value, key) {
    return this.put(value, key);
  }

  delete(key) {
    const request = new IDBRequest();
    (async () => {
      try {
        await Deno.core.ops.op_idb_delete(this._db.name, this.name, key);
        request._success(undefined);
      } catch (e) {
        request._error(e);
      }
    })();
    return request;
  }

  clear() {
    const request = new IDBRequest();
    (async () => {
      try {
        await Deno.core.ops.op_idb_clear(this._db.name, this.name);
        request._success(undefined);
      } catch (e) {
        request._error(e);
      }
    })();
    return request;
  }

  getAll(query, count) {
    const request = new IDBRequest();
    (async () => {
      try {
        const values = await Deno.core.ops.op_idb_get_all(this._db.name, this.name, count);
        request._success(values);
      } catch (e) {
        request._error(e);
      }
    })();
    return request;
  }

  getAllKeys(query, count) {
    const request = new IDBRequest();
    (async () => {
      try {
        const keys = await Deno.core.ops.op_idb_get_all_keys(this._db.name, this.name, count);
        request._success(keys);
      } catch (e) {
        request._error(e);
      }
    })();
    return request;
  }

  count() {
    const request = new IDBRequest();
    (async () => {
      try {
        const count = await Deno.core.ops.op_idb_count(this._db.name, this.name);
        request._success(count);
      } catch (e) {
        request._error(e);
      }
    })();
    return request;
  }
}

globalThis.indexedDB = new IDBFactory();
globalThis.IDBFactory = IDBFactory;
globalThis.IDBRequest = IDBRequest;
globalThis.IDBOpenDBRequest = IDBOpenDBRequest;
globalThis.IDBDatabase = IDBDatabase;
globalThis.IDBTransaction = IDBTransaction;
globalThis.IDBObjectStore = IDBObjectStore;

// ============================================================================
// TextEncoder / TextDecoder (for binary data handling)
// ============================================================================
if (typeof globalThis.TextEncoder === 'undefined') {
  globalThis.TextEncoder = class TextEncoder {
    encode(str) {
      const arr = [];
      for (let i = 0; i < str.length; i++) {
        let c = str.charCodeAt(i);
        if (c < 128) {
          arr.push(c);
        } else if (c < 2048) {
          arr.push((c >> 6) | 192, (c & 63) | 128);
        } else {
          arr.push((c >> 12) | 224, ((c >> 6) & 63) | 128, (c & 63) | 128);
        }
      }
      return new Uint8Array(arr);
    }
  };
}

if (typeof globalThis.TextDecoder === 'undefined') {
  globalThis.TextDecoder = class TextDecoder {
    decode(arr) {
      if (!arr) return '';
      const bytes = arr instanceof Uint8Array ? arr : new Uint8Array(arr);
      let result = '';
      for (let i = 0; i < bytes.length; i++) {
        result += String.fromCharCode(bytes[i]);
      }
      return result;
    }
  };
}

// ============================================================================
// atob / btoa (Base64)
// ============================================================================
if (typeof globalThis.atob === 'undefined') {
  globalThis.atob = function (str) {
    return Deno.core.ops.op_atob(str);
  };
}

if (typeof globalThis.btoa === 'undefined') {
  globalThis.btoa = function (str) {
    return Deno.core.ops.op_btoa(str);
  };
}

// ============================================================================
// Crypto API (basic)
// ============================================================================
globalThis.crypto = {
  getRandomValues: function (array) {
    const bytes = Deno.core.ops.op_crypto_random(array.length);
    array.set(bytes);
    return array;
  },
};

// ============================================================================
// Performance API (basic)
// ============================================================================
globalThis.performance = {
  now: function () {
    return Deno.core.ops.op_performance_now();
  },
};

// ============================================================================
// Skill Bridge API
// ============================================================================
// These are exposed to skills via the `platform`, `db`, `store`, etc globals

globalThis.__db = {
  exec: function (sql, paramsJson) {
    return Deno.core.ops.op_db_exec(sql, paramsJson);
  },
  get: function (sql, paramsJson) {
    return Deno.core.ops.op_db_get(sql, paramsJson);
  },
  all: function (sql, paramsJson) {
    return Deno.core.ops.op_db_all(sql, paramsJson);
  },
  kvGet: function (key) {
    return Deno.core.ops.op_db_kv_get(key);
  },
  kvSet: function (key, valueJson) {
    return Deno.core.ops.op_db_kv_set(key, valueJson);
  },
};

globalThis.__store = {
  get: function (key) {
    return Deno.core.ops.op_store_get(key);
  },
  set: function (key, valueJson) {
    return Deno.core.ops.op_store_set(key, valueJson);
  },
  delete: function (key) {
    return Deno.core.ops.op_store_delete(key);
  },
  keys: function () {
    return Deno.core.ops.op_store_keys();
  },
};

globalThis.__net = {
  fetch: function (url, optionsJson) {
    return Deno.core.ops.op_net_fetch(url, optionsJson);
  },
};

globalThis.__platform = {
  os: function () {
    return Deno.core.ops.op_platform_os();
  },
  env: function (key) {
    return Deno.core.ops.op_platform_env(key);
  },
};

// High-level wrappers for skills (V8 bridge)
globalThis.db = {
  exec: function (sql, params) {
    return __db.exec(sql, params ? JSON.stringify(params) : undefined);
  },
  get: function (sql, params) {
    var result = __db.get(sql, params ? JSON.stringify(params) : undefined);
    return JSON.parse(result);
  },
  all: function (sql, params) {
    var result = __db.all(sql, params ? JSON.stringify(params) : undefined);
    return JSON.parse(result);
  },
  kvGet: function (key) {
    var result = __db.kvGet(key);
    return JSON.parse(result);
  },
  kvSet: function (key, value) {
    return __db.kvSet(key, JSON.stringify(value));
  },
};

globalThis.store = {
  get: function (key) {
    var result = __store.get(key);
    return JSON.parse(result);
  },
  set: function (key, value) {
    return __store.set(key, JSON.stringify(value));
  },
  delete: function (key) {
    return __store.delete(key);
  },
  keys: function () {
    var result = __store.keys();
    return JSON.parse(result);
  },
};

globalThis.net = {
  fetch: function (url, options) {
    var result = __net.fetch(url, options ? JSON.stringify(options) : '{}');
    return JSON.parse(result);
  },
};

globalThis.platform = {
  os: function () {
    return __platform.os();
  },
  env: function (key) {
    return __platform.env(key);
  },
};

// ============================================================================
// State Bridge API (for skills to publish state)
// ============================================================================
globalThis.__state = {
  get: function (key) {
    return Deno.core.ops.op_state_get(key);
  },
  set: function (key, valueJson) {
    return Deno.core.ops.op_state_set(key, valueJson);
  },
  setPartial: function (partialJson) {
    return Deno.core.ops.op_state_set_partial(partialJson);
  },
};

globalThis.state = {
  get: function (key) {
    var result = __state.get(key);
    return JSON.parse(result);
  },
  set: function (key, value) {
    return __state.set(key, JSON.stringify(value));
  },
  setPartial: function (partial) {
    return __state.setPartial(JSON.stringify(partial));
  },
};

// ============================================================================
// Data Bridge API (for skills to read/write files)
// ============================================================================
globalThis.__data = {
  read: function (filename) {
    return Deno.core.ops.op_data_read(filename);
  },
  write: function (filename, content) {
    return Deno.core.ops.op_data_write(filename, content);
  },
};

globalThis.data = {
  read: function (filename) {
    return __data.read(filename);
  },
  write: function (filename, content) {
    return __data.write(filename, content);
  },
};

// ============================================================================
// Cron Bridge API (placeholder - requires integration with CronScheduler)
// ============================================================================
globalThis.cron = {
  register: function (scheduleId, cronExpr) {
    console.warn('[cron] register not implemented in V8 runtime yet');
    return false;
  },
  unregister: function (scheduleId) {
    console.warn('[cron] unregister not implemented in V8 runtime yet');
    return false;
  },
  list: function () {
    console.warn('[cron] list not implemented in V8 runtime yet');
    return [];
  },
};

// ============================================================================
// Skills Bridge API (placeholder - requires integration with SkillRegistry)
// ============================================================================
globalThis.skills = {
  list: function () {
    console.warn('[skills] list not implemented in V8 runtime yet');
    return [];
  },
  callTool: function (skillId, toolName, args) {
    console.warn('[skills] callTool not implemented in V8 runtime yet');
    return { error: 'Not implemented' };
  },
};

// ============================================================================
// TDLib Bridge API (telegram skill only)
// ============================================================================
// Provides native TDLib access for the telegram skill.
// This is only available on desktop - Android uses Tauri invoke() instead.

globalThis.tdlib = {
  /**
   * Check if TDLib ops are available.
   * @returns {boolean} True on desktop, false on mobile/web.
   */
  isAvailable: function () {
    try {
      return typeof Deno?.core?.ops?.op_tdlib_is_available === 'function'
        ? Deno.core.ops.op_tdlib_is_available()
        : false;
    } catch (e) {
      return false;
    }
  },

  /**
   * Create a TDLib client with the given data directory.
   * @param {string} dataDir - Path to store TDLib data files.
   * @returns {Promise<number>} Client ID (always 1 for singleton).
   */
  createClient: async function (dataDir) {
    return await Deno.core.ops.op_tdlib_create_client(dataDir);
  },

  /**
   * Send a TDLib request and wait for the response.
   * @param {object} request - TDLib API request object with @type field.
   * @returns {Promise<object>} TDLib response object.
   */
  send: async function (request) {
    return await Deno.core.ops.op_tdlib_send(request);
  },

  /**
   * Receive the next TDLib update (with timeout).
   * @param {number} [timeoutMs=1000] - Timeout in milliseconds.
   * @returns {Promise<object|null>} Update object or null if timeout.
   */
  receive: async function (timeoutMs = 1000) {
    return await Deno.core.ops.op_tdlib_receive(timeoutMs);
  },

  /**
   * Destroy the TDLib client and clean up resources.
   * @returns {Promise<void>}
   */
  destroy: async function () {
    return await Deno.core.ops.op_tdlib_destroy();
  },
};

// ============================================================================
// Model Bridge API (local LLM inference)
// ============================================================================

globalThis.__model = {
  isAvailable: function () {
    try {
      return typeof Deno?.core?.ops?.op_model_is_available === 'function'
        ? Deno.core.ops.op_model_is_available()
        : false;
    } catch (e) {
      return false;
    }
  },
  getStatus: function () {
    return Deno.core.ops.op_model_get_status();
  },
  generate: async function (prompt, configJson) {
    return await Deno.core.ops.op_model_generate(prompt, configJson);
  },
  summarize: async function (text, maxTokens) {
    return await Deno.core.ops.op_model_summarize(text, maxTokens);
  },
};

globalThis.model = {
  /**
   * Check if local model is available (desktop only).
   * @returns {boolean}
   */
  isAvailable: function () {
    return __model.isAvailable();
  },

  /**
   * Get model status.
   * @returns {{ available: boolean, loaded: boolean, loading: boolean, downloadProgress?: number, error?: string, modelPath?: string }}
   */
  getStatus: function () {
    return __model.getStatus();
  },

  /**
   * Generate text from a prompt.
   * @param {string} prompt - Input prompt
   * @param {object} [options] - Generation options
   * @param {number} [options.maxTokens=2048] - Max output tokens
   * @param {number} [options.temperature=0.7] - Sampling temperature
   * @param {number} [options.topP=0.9] - Top-p sampling
   * @returns {Promise<string>}
   */
  generate: async function (prompt, options) {
    var config = {
      max_tokens: (options && options.maxTokens) || 2048,
      temperature: (options && options.temperature) || 0.7,
      top_p: (options && options.topP) || 0.9,
    };
    return await __model.generate(prompt, config);
  },

  /**
   * Summarize text locally.
   * @param {string} text - Text to summarize
   * @param {object} [options] - Options
   * @param {number} [options.maxTokens=500] - Target summary length
   * @returns {Promise<string>}
   */
  summarize: async function (text, options) {
    var maxTokens = (options && options.maxTokens) || 500;
    return await __model.summarize(text, maxTokens);
  },
};

console.log('[bootstrap] Model API initialized');
console.log('[bootstrap] V8 browser APIs initialized');

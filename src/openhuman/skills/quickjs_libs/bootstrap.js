/**
 * Bootstrap JavaScript for QuickJS Runtime
 *
 * Provides browser-like API shims for skill execution.
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
    __ops.console_log(args.map(String).join(' '));
  },
  info: function (...args) {
    __ops.console_log(args.map(String).join(' '));
  },
  warn: function (...args) {
    __ops.console_warn(args.map(String).join(' '));
  },
  error: function (...args) {
    __ops.console_error(args.map(String).join(' '));
  },
  debug: function (...args) {
    __ops.console_log('[DEBUG] ' + args.map(String).join(' '));
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
  __ops.timer_start(id, delay || 0, false);
  return id;
};

globalThis.setInterval = function (callback, delay, ...args) {
  const id = nextTimerId++;
  timerCallbacks.set(id, { callback, args, type: 'interval' });
  __ops.timer_start(id, delay || 0, true);
  return id;
};

globalThis.clearTimeout = function (id) {
  timerCallbacks.delete(id);
  __ops.timer_cancel(id);
};

globalThis.clearInterval = function (id) {
  timerCallbacks.delete(id);
  __ops.timer_cancel(id);
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

  // __ops.fetch expects a JSON string for options (not a JS object)
  const resultJson = await __ops.fetch(
    url.toString(),
    JSON.stringify({
      method,
      headers: headersObj,
      body: typeof body === 'string' ? body : body ? JSON.stringify(body) : null,
    })
  );

  // __ops.fetch returns a JSON string — parse it to access status/headers/body
  const parsed = JSON.parse(resultJson);

  return new Response(parsed.body, {
    status: parsed.status,
    statusText: parsed.statusText || '',
    headers: new Headers(parsed.headers || {}),
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
      this._id = await __ops.ws_connect(this.url);
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
        const message = await __ops.ws_recv(this._id);
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
      this.onclose({ type: 'close', code, reason, wasClean: code === 1000, target: this });
    }
  }

  send(data) {
    if (this.readyState !== WebSocket.OPEN) {
      throw new Error('WebSocket is not open');
    }

    const dataStr = typeof data === 'string' ? data : JSON.stringify(data);
    __ops.ws_send(this._id, dataStr);
  }

  close(code = 1000, reason = '') {
    if (this.readyState === WebSocket.CLOSING || this.readyState === WebSocket.CLOSED) {
      return;
    }

    this.readyState = WebSocket.CLOSING;
    __ops.ws_close(this._id, code, reason);
  }
}

WebSocket._instances = new Map();
globalThis.WebSocket = WebSocket;

// ============================================================================
// IndexedDB API (persistent local storage)
// ============================================================================
class IDBFactory {
  open(name, version = 1) {
    return new IDBOpenDBRequest(name, version);
  }

  deleteDatabase(name) {
    const request = new IDBRequest();
    (async () => {
      try {
        await __ops.idb_delete_database(name);
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
      const info = await __ops.idb_open(this._name, this._version);
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
    __ops.idb_create_object_store(this.name, name, options);
    this.objectStoreNames.push(name);
    return new IDBObjectStore(this, name);
  }

  deleteObjectStore(name) {
    __ops.idb_delete_object_store(this.name, name);
    const idx = this.objectStoreNames.indexOf(name);
    if (idx >= 0) this.objectStoreNames.splice(idx, 1);
  }

  transaction(storeNames, mode = 'readonly') {
    return new IDBTransaction(this, storeNames, mode);
  }

  close() {
    __ops.idb_close(this.name);
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
        const value = await __ops.idb_get(this._db.name, this.name, key);
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
        await __ops.idb_put(this._db.name, this.name, key, value);
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
        await __ops.idb_delete(this._db.name, this.name, key);
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
        await __ops.idb_clear(this._db.name, this.name);
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
        const values = await __ops.idb_get_all(this._db.name, this.name, count);
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
        const keys = await __ops.idb_get_all_keys(this._db.name, this.name, count);
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
        const count = await __ops.idb_count(this._db.name, this.name);
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
    return __ops.atob(str);
  };
}

if (typeof globalThis.btoa === 'undefined') {
  globalThis.btoa = function (str) {
    return __ops.btoa(str);
  };
}

// ============================================================================
// Crypto API (basic)
// ============================================================================
globalThis.crypto = {
  getRandomValues: function (array) {
    const bytes = __ops.crypto_random(array.length);
    array.set(bytes);
    return array;
  },
};

// ============================================================================
// Performance API (basic)
// ============================================================================
globalThis.performance = {
  now: function () {
    return __ops.performance_now();
  },
};

// ============================================================================
// Skill Bridge API
// ============================================================================
// These are exposed to skills via the `platform`, `db`, `state`, etc globals

globalThis.__db = {
  exec: function (sql, paramsJson) {
    return __ops.db_exec(sql, paramsJson);
  },
  get: function (sql, paramsJson) {
    return __ops.db_get(sql, paramsJson);
  },
  all: function (sql, paramsJson) {
    return __ops.db_all(sql, paramsJson);
  },
  kvGet: function (key) {
    return __ops.db_kv_get(key);
  },
  kvSet: function (key, valueJson) {
    return __ops.db_kv_set(key, valueJson);
  },
};

globalThis.__platform = {
  os: function () {
    return __ops.platform_os();
  },
  env: function (key) {
    return __ops.platform_env(key);
  },
};

// High-level wrappers for skills (QuickJS bridge)
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

globalThis.net = {
  fetch: async function (url, options) {
    const result = await __ops.fetch(url, options ? JSON.stringify(options) : '{}');
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
// Memory Bridge (for skills to send memory payloads to backend)
// ============================================================================
globalThis.memory = {
  /**
   * Insert a memory payload through the native memory bridge.
   * Provider is inferred from the current skill ID on the Rust side.
   * @param {object} metadata - Memory payload metadata.
   * @returns {boolean}
   */
  insert: function (metadata) {
    if (!metadata || typeof metadata !== 'object') {
      throw new Error('memory.insert requires an object payload');
    }

    __ops.memory_insert(JSON.stringify(metadata));
    return true;
  },
};

// ============================================================================
// State Bridge API (for skills to publish state)
// ============================================================================
globalThis.__state = {
  get: function (key) {
    return __ops.state_get(key);
  },
  set: function (key, valueJson) {
    return __ops.state_set(key, valueJson);
  },
  setPartial: function (partialJson) {
    return __ops.state_set_partial(partialJson);
  },
};

globalThis.state = {
  get: function (key) {
    var result = __ops.store_get(key);
    return JSON.parse(result);
  },
  set: function (key, value) {
    __ops.store_set(key, JSON.stringify(value));
    __state.set(key, JSON.stringify(value));
  },
  setPartial: function (partial) {
    var keys = Object.keys(partial);
    for (var i = 0; i < keys.length; i++) {
      __ops.store_set(keys[i], JSON.stringify(partial[keys[i]]));
    }
    __state.setPartial(JSON.stringify(partial));
  },
  delete: function (key) {
    return __ops.store_delete(key);
  },
  keys: function () {
    var result = __ops.store_keys();
    return JSON.parse(result);
  },
};

// ============================================================================
// Data Bridge API (for skills to read/write files)
// ============================================================================
globalThis.__data = {
  read: function (filename) {
    return __ops.data_read(filename);
  },
  write: function (filename, content) {
    return __ops.data_write(filename, content);
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
// OAuth Bridge API (credential management and authenticated proxy)
// ============================================================================
(function () {
  globalThis.__oauthCredential = null;

  globalThis.oauth = {
    /** Get the current OAuth credential, or null if not connected. */
    getCredential: function () {
      return globalThis.__oauthCredential;
    },

    /**
     * Make an authenticated API request proxied through the backend.
     * Path is relative to manifest's apiBaseUrl.
     */
    fetch: async function (path, options) {
      if (!globalThis.__oauthCredential) {
        return {
          status: 401,
          headers: {},
          body: JSON.stringify({ error: 'No OAuth credential. Complete OAuth setup first.' }),
        };
      }
      var backendUrl = __platform.env('BACKEND_URL') || 'https://api.tinyhumans.ai';
      var jwtToken = __ops.get_session_token() || '';
      var cleanPath = path.charAt(0) === '/' ? path.slice(1) : path;
      var proxyUrl =
        backendUrl + '/proxy/by-id/' + globalThis.__oauthCredential.credentialId + '/' + cleanPath;
      var method = (options && options.method) || 'GET';
      var headers = { 'Content-Type': 'application/json' };
      if (jwtToken) {
        headers['Authorization'] = 'Bearer ' + jwtToken;
      }
      if (options && options.headers) {
        for (var k in options.headers) {
          headers[k] = options.headers[k];
        }
      }
      var fetchOpts = {
        method: method,
        headers: headers,
        body: options ? options.body : undefined,
        timeout: options ? options.timeout : undefined,
      };

      console.log('[oauth.fetch] ' + method + ' ' + proxyUrl + ' (credentialId=' + globalThis.__oauthCredential.credentialId + ')');
      var result = await net.fetch(proxyUrl, fetchOpts);
      console.log('[oauth.fetch] response status=' + result.status + ' body_len=' + (result.body ? result.body.length : 0));
      return result;
    },

    /** Revoke the current OAuth credential server-side. */
    revoke: async function () {
      if (__oauthCredential) {
        try {
          var backendUrl = __platform.env('BACKEND_URL') || 'https://api.tinyhumans.ai';
          var jwtToken = __ops.get_session_token() || '';
          var revokeOpts = JSON.stringify({
            method: 'DELETE',
            headers: { 'Content-Type': 'application/json', Authorization: 'Bearer ' + jwtToken },
          });
          await net.fetch(
            backendUrl + '/auth/integrations/' + __oauthCredential.credentialId,
            revokeOpts
          );
        } catch (e) {
          /* best effort */
        }
      }
      __oauthCredential = null;
      return true;
    },

    /** Internal: set credential (called by runtime on oauth/complete). */
    __setCredential: function (cred) {
      __oauthCredential = cred;
    },
  };
})();

// ============================================================================
// Tools array (skills assign to this global)
// ============================================================================
globalThis.tools = [];

// ============================================================================
// Cron Bridge API (placeholder - requires integration with CronScheduler)
// ============================================================================
globalThis.cron = {
  register: function (scheduleId, cronExpr) {
    console.warn('[cron] register not implemented in QuickJS runtime yet');
    return false;
  },
  unregister: function (scheduleId) {
    console.warn('[cron] unregister not implemented in QuickJS runtime yet');
    return false;
  },
  list: function () {
    console.warn('[cron] list not implemented in QuickJS runtime yet');
    return [];
  },
};

// ============================================================================
// Skills Bridge API (placeholder - requires integration with SkillRegistry)
// ============================================================================
globalThis.skills = {
  list: function () {
    console.warn('[skills] list is intentionally unavailable in isolated runtime');
    return [];
  },
  callTool: function (skillId, toolName, args) {
    console.warn('[skills] callTool is disabled by runtime isolation policy');
    return { error: 'Cross-skill invocation is disabled' };
  },
};

// ============================================================================
// Model Bridge API (routes to cloud backend)
// ============================================================================

globalThis.model = {
  /**
   * Generate text from a prompt via the backend API.
   * @param {string} prompt - Input prompt
   * @param {object} [options] - Generation options
   * @param {number} [options.maxTokens=2048] - Max output tokens
   * @param {number} [options.temperature=0.7] - Sampling temperature
   * @returns {string}
   */
  generate: async function (prompt, options) {
    var backendUrl = __platform.env('BACKEND_URL') || 'https://api.tinyhumans.ai';
    var jwtToken = __ops.get_session_token() || '';
    var body = { prompt: prompt };
    if (options && options.maxTokens) body.maxTokens = options.maxTokens;
    if (options && options.temperature) body.temperature = options.temperature;
    var result = await net.fetch(backendUrl + '/api/ai/generate', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Authorization: 'Bearer ' + jwtToken },
      body: JSON.stringify(body),
      timeout: 30000,
    });
    var parsed = JSON.parse(result);
    if (parsed.status >= 400) {
      throw new Error('Backend returned ' + parsed.status + ': ' + parsed.body);
    }
    var data = JSON.parse(parsed.body);
    return data.text || '';
  },

  /**
   * Summarize text via the backend API.
   * @param {string} text - Text to summarize
   * @param {object} [options] - Options
   * @param {number} [options.maxTokens=500] - Target summary length
   * @returns {string}
   */
  summarize: async function (text, options) {
    var backendUrl = __platform.env('BACKEND_URL') || 'https://api.tinyhumans.ai';
    var jwtToken = __ops.get_session_token() || '';
    var body = { text: text };
    if (options && options.maxTokens) body.maxTokens = options.maxTokens;
    var result = await net.fetch(backendUrl + '/api/ai/summarize', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Authorization: 'Bearer ' + jwtToken },
      body: JSON.stringify(body),
      timeout: 30000,
    });
    var parsed = JSON.parse(result);
    if (parsed.status >= 400) {
      throw new Error('Backend returned ' + parsed.status + ': ' + parsed.body);
    }
    var data = JSON.parse(parsed.body);
    return data.summary || '';
  },
};

console.log('[bootstrap] Model API initialized');
console.log('[bootstrap] QuickJS browser APIs initialized');

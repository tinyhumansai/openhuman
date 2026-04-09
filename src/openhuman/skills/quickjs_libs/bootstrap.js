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
globalThis.fetch = function (url, options) {
  options = options || {};
  var method = options.method || 'GET';
  var headers = options.headers || {};
  var body = options.body || null;

  // Convert Headers object to plain object if needed
  var headersObj = {};
  if (headers instanceof Headers) {
    headers.forEach(function (value, key) {
      headersObj[key] = value;
    });
  } else {
    headersObj = headers;
  }

  // __ops.fetch expects a JSON string for options (not a JS object)
  var resultJson = __ops.fetch(
    url.toString(),
    JSON.stringify({
      method: method,
      headers: headersObj,
      body: typeof body === 'string' ? body : body ? JSON.stringify(body) : null,
    })
  );

  // __ops.fetch returns a JSON string — parse it to access status/headers/body
  var parsed = JSON.parse(resultJson);

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

  text() {
    return this._body;
  }

  json() {
    return JSON.parse(this._body);
  }

  arrayBuffer() {
    var encoder = new TextEncoder();
    return encoder.encode(this._body).buffer;
  }

  blob() {
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

/**
 * Base URL for OAuth proxy and webhook API calls.
 * Uses BACKEND_URL, then VITE_BACKEND_URL, then production default.
 * Trailing slashes are stripped.
 */
globalThis.__resolveBackendBaseUrl = () => {
  let raw = (__platform.env('BACKEND_URL') || __platform.env('VITE_BACKEND_URL') || '').trim();
  if (!raw) {
    return 'https://api.tinyhumans.ai';
  }
  while (raw.length > 0 && raw.charAt(raw.length - 1) === '/') {
    raw = raw.slice(0, -1);
  }
  return raw;
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
  fetch: function (url, options) {
    var result = __ops.fetch(url, options ? JSON.stringify(options) : '{}');
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
  /**
   * Desktop-style notification hook. The QuickJS host has no system tray UI here;
   * avoid logging title/body (may contain PII); shim exists so sync flows do not throw "not a function".
   */
  notify: function (_title, _body) {
    console.log('[platform.notify] notification requested');
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
     *
     * When a client key share is available (`__oauthClientKey`), uses the
     * encrypted proxy endpoint (`/proxy/encrypted/:id/`) with the
     * `X-Encryption-Key` header so the backend can reconstruct the full
     * encryption key and decrypt OAuth tokens server-side.
     */
    fetch: function (path, options) {
      if (!globalThis.__oauthCredential) {
        return {
          status: 401,
          headers: {},
          body: JSON.stringify({ error: 'No OAuth credential. Complete OAuth setup first.' }),
        };
      }
      const backendUrl = globalThis.__resolveBackendBaseUrl();
      const jwtToken = __ops.get_session_token() || '';
      const cleanPath = path.charAt(0) === '/' ? path.slice(1) : path;
      const credentialId = globalThis.__oauthCredential.credentialId;
      const clientKey = globalThis.__oauthClientKey || null;

      // Use encrypted proxy when client key share is available
      let proxyUrl;
      if (clientKey) {
        proxyUrl = `${backendUrl}/proxy/encrypted/${credentialId}/${cleanPath}`;
      } else {
        proxyUrl = `${backendUrl}/proxy/by-id/${credentialId}/${cleanPath}`;
      }

      var method = (options && options.method) || 'GET';
      var headers = { 'Content-Type': 'application/json' };
      if (jwtToken) {
        headers['Authorization'] = 'Bearer ' + jwtToken;
      }
      if (clientKey) {
        headers['X-Encryption-Key'] = clientKey;
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

      console.log('[oauth.fetch] ' + method + ' ' + proxyUrl + ' (credentialId=' + credentialId + ', encrypted=' + !!clientKey + ', Notion-Version=' + (headers['Notion-Version'] || 'none') + ')');
      var result = net.fetch(proxyUrl, fetchOpts);
      console.log('[oauth.fetch] response status=' + result.status + ' body_len=' + (result.body ? result.body.length : 0));

      // Auto-clear invalid/expired credentials so the user is prompted to re-auth
      if (result.status === 401 || result.status === 403) {
        console.warn('[oauth.fetch] Got ' + result.status + ' — clearing invalid credential for re-auth');
        globalThis.__oauthCredential = null;
        if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {
          globalThis.state.set('__oauth_credential', '');
          globalThis.state.setPartial({
            connection_status: 'error',
            connection_error: 'Integration token expired or invalid. Please reconnect.',
            auth_status: 'not_authenticated',
          });
        }
      }

      return result;
    },

    /** Revoke the current OAuth credential server-side. */
    revoke: function () {
      if (__oauthCredential) {
        try {
          const backendUrl = globalThis.__resolveBackendBaseUrl();
          const jwtToken = __ops.get_session_token() || '';
          const revokeOpts = {
            method: 'DELETE',
            headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${jwtToken}` },
          };
          net.fetch(`${backendUrl}/auth/integrations/${__oauthCredential.credentialId}`, revokeOpts);
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
// Advanced Auth Bridge API (self-hosted / text credential management)
// ============================================================================
(function () {
  globalThis.__authCredential = null;

  globalThis.auth = {
    /**
     * Get the full auth credential object, or null if not set.
     * Shape: { mode: "managed"|"self_hosted"|"text", credentials: {...} }
     */
    getCredential: function () {
      return globalThis.__authCredential;
    },

    /**
     * Get the auth mode string, or null.
     * @returns {"managed"|"self_hosted"|"text"|null}
     */
    getMode: function () {
      return globalThis.__authCredential ? globalThis.__authCredential.mode : null;
    },

    /**
     * Get just the credentials map (e.g. { client_id, client_secret, url, content }).
     * @returns {Object|null}
     */
    getCredentials: function () {
      return globalThis.__authCredential ? globalThis.__authCredential.credentials : null;
    },

    /**
     * Make an HTTP request with credentials auto-injected.
     * For self_hosted: auto-adds Authorization header from client_id/client_secret.
     * For managed: delegates to oauth.fetch.
     * For text: no auto-injection (skill should handle manually).
     */
    fetch: function (url, options) {
      if (!globalThis.__authCredential) {
        return {
          status: 401,
          headers: {},
          body: JSON.stringify({ error: 'No auth credential. Complete auth setup first.' }),
        };
      }

      var mode = globalThis.__authCredential.mode;
      var creds = globalThis.__authCredential.credentials;

      // Managed mode: delegate to oauth.fetch for proxy behavior
      if (mode === 'managed' && typeof globalThis.oauth !== 'undefined') {
        return globalThis.oauth.fetch(url, options);
      }

      var headers = {};
      if (options && options.headers) {
        for (var k in options.headers) {
          headers[k] = options.headers[k];
        }
      }
      if (!headers['Content-Type']) {
        headers['Content-Type'] = 'application/json';
      }

      // Check for existing Authorization header (case-insensitive)
      var hasAuth = false;
      for (var hk in headers) {
        if (hk.toLowerCase() === 'authorization') { hasAuth = true; break; }
      }

      // Self-hosted: auto-inject basic auth if client_id + client_secret present
      if (mode === 'self_hosted' && creds.client_id && creds.client_secret && !hasAuth) {
        headers['Authorization'] = 'Basic ' + btoa(creds.client_id + ':' + creds.client_secret);
        hasAuth = true;
      }

      // Self-hosted with access_token or refresh_token: inject Bearer token
      if (mode === 'self_hosted' && !hasAuth && creds.access_token) {
        headers['Authorization'] = 'Bearer ' + creds.access_token;
        hasAuth = true;
      }

      // Do NOT auto-inject session JWT for text mode or when auth is already set.
      // Text mode credentials are opaque — the skill handles auth manually.

      var fetchOpts = {
        method: (options && options.method) || 'GET',
        headers: headers,
        body: options ? options.body : undefined,
        timeout: options ? options.timeout : undefined,
      };

      console.log('[auth.fetch] ' + fetchOpts.method + ' ' + url + ' (mode=' + mode + ')');
      var result = net.fetch(url, fetchOpts);
      console.log('[auth.fetch] response status=' + result.status);

      // Auto-clear on 401/403 so user is prompted to re-auth
      if (result.status === 401 || result.status === 403) {
        console.warn('[auth.fetch] Got ' + result.status + ' — clearing invalid credential');
        globalThis.__authCredential = null;
        if (typeof globalThis.state !== 'undefined' && globalThis.state.set) {
          globalThis.state.set('__auth_credential', '');
          globalThis.state.setPartial({
            connection_status: 'error',
            connection_error: 'Auth credential expired or invalid. Please reconnect.',
            auth_status: 'not_authenticated',
          });
        }
      }

      return result;
    },

    /**
     * Internal: set credential (called by runtime on auth/complete).
     * Skills should not call this directly.
     */
    __setCredential: function (cred) {
      globalThis.__authCredential = cred;
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

// Model API — removed (inference handled at the Rust/agent layer, not in skills)

// ============================================================================
// Webhook / Tunnel API (skill-scoped)
// ============================================================================
// All operations are scoped to the calling skill. The Rust bridge injects the
// skill_id automatically — JS code cannot impersonate another skill.

globalThis.webhook = {
  /**
   * Register this skill to receive webhooks for a tunnel UUID.
   * Rejects if the tunnel is already owned by a different skill.
   * @param {string} tunnelUuid - The tunnel UUID (from createTunnel or backend)
   * @param {string} [tunnelName] - Human-readable name for display
   * @param {string} [backendTunnelId] - Backend MongoDB _id for CRUD
   */
  register: function (tunnelUuid, tunnelName, backendTunnelId) {
    __ops.webhook_register(
      tunnelUuid,
      tunnelName || null,
      backendTunnelId || null
    );
  },

  /**
   * Unregister this skill from a tunnel.
   * Rejects if the tunnel is not owned by this skill.
   * @param {string} tunnelUuid
   */
  unregister: function (tunnelUuid) {
    __ops.webhook_unregister(tunnelUuid);
  },

  /**
   * List only this skill's registered tunnel mappings.
   * Never includes other skills' tunnels.
   * @returns {Array<{tunnel_uuid: string, skill_id: string, tunnel_name: string|null}>}
   */
  list: function () {
    var json = __ops.webhook_list();
    return JSON.parse(json);
  },

  /**
   * Create a new tunnel via the backend API, automatically registered to
   * this skill.
   * @param {string} name - Tunnel name
   * @param {string} [description] - Optional description
   * @returns {Promise<{id: string, uuid: string, webhookUrl: string}>}
   */
  createTunnel: async function (name, description) {
    const backendUrl = globalThis.__resolveBackendBaseUrl();
    const jwtToken = __ops.get_session_token() || '';

    var result = await net.fetch(`${backendUrl}/webhooks/core`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: 'Bearer ' + jwtToken,
      },
      body: JSON.stringify({ name: name, description: description || '' }),
      timeout: 15000,
    });

    var parsed = JSON.parse(result);
    if (parsed.status >= 400) {
      throw new Error('Failed to create tunnel: ' + parsed.status + ' ' + parsed.body);
    }
    var data = JSON.parse(parsed.body);
    var tunnel = data.data || data.tunnel || data;

    // Auto-register this tunnel to the calling skill
    if (tunnel.uuid) {
      webhook.register(tunnel.uuid, name, tunnel._id || tunnel.id || null);
    }

    // Build webhook URL for the caller
    tunnel.webhookUrl = backendUrl.replace(/\/$/, '') + '/webhooks/ingress/' + tunnel.uuid;

    console.log('[webhook] Created tunnel: ' + name + ' → ' + tunnel.webhookUrl);
    return tunnel;
  },

  /**
   * List locally registered tunnels scoped to this skill.
   * Returns the local webhook registrations, not data from the backend API.
   * @returns {Promise<Array>} Array of local tunnel registration objects.
   */
  listTunnels: async function () {
    return webhook.list();
  },

  /**
   * Delete a tunnel. Fails if the tunnel is not owned by this skill.
   * @param {string} tunnelUuid - The tunnel UUID to delete
   */
  deleteTunnel: async function (tunnelUuid) {
    var registration = null;
    webhook.list().forEach(function (reg) {
      if (reg.tunnel_uuid === tunnelUuid) {
        registration = reg;
      }
    });
    if (!registration) {
      throw new Error('[webhook] Tunnel is not registered to this skill: ' + tunnelUuid);
    }
    if (!registration.backend_tunnel_id) {
      throw new Error(
        '[webhook] Missing backend tunnel id for deleteTunnel; re-create or re-register this tunnel'
      );
    }

    // Delete from backend first
    const backendUrl = globalThis.__resolveBackendBaseUrl();
    const jwtToken = __ops.get_session_token() || '';

    var result = await net.fetch(`${backendUrl}/webhooks/core/${registration.backend_tunnel_id}`, {
      method: 'DELETE',
      headers: { Authorization: 'Bearer ' + jwtToken },
      timeout: 10000,
    });

    var parsed = JSON.parse(result);
    if (parsed.status >= 400 && parsed.status !== 404) {
      throw new Error('[webhook] Backend delete failed with status ' + parsed.status);
    }

    // Backend confirmed deletion (or 404 = already gone) — now safe to
    // remove the local registration.
    webhook.unregister(tunnelUuid);

    console.log('[webhook] Deleted tunnel: ' + tunnelUuid);
  },
};

console.log('[bootstrap] Webhook API initialized');
console.log('[bootstrap] QuickJS browser APIs initialized');

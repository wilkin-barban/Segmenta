// Segmenta browser API adapter.
//
// Firefox exposes the promise based browser.* namespace, Chromium exposes the
// callback based chrome.* namespace. Chrome also returns promises for most APIs
// since MV3, so both shapes are supported. The rest of the extension only talks
// to the helpers below.
(function (global) {
  'use strict';

  const HOST_NAME = 'io.github.wilkin.segmenta';
  const NATIVE_HOST = HOST_NAME;

  // Public, fixed extension namespace. Kept for parity with the documented
  // contract and for external callers that read it directly.
  global.segmentaApi = global.browser !== undefined ? global.browser : global.chrome;

  function resolveRuntime() {
    const candidate = global.browser;
    if (candidate && candidate.runtime && candidate.runtime.id) {
      return candidate;
    }
    const fallback = global.chrome;
    if (fallback && fallback.runtime && fallback.runtime.id) {
      return fallback;
    }
    return global.segmentaApi || null;
  }

  function resolveI18n() {
    const browserApi = global.browser;
    if (browserApi && browserApi.i18n) {
      return browserApi.i18n;
    }
    const chromeApi = global.chrome;
    if (chromeApi && chromeApi.i18n) {
      return chromeApi.i18n;
    }
    return null;
  }

  function lastError(api) {
    const runtime = api && api.runtime;
    return runtime && runtime.lastError ? runtime.lastError : null;
  }

  // Wraps both callback style and promise style APIs in a single promise.
  function invoke(api, target, method, args) {
    const owner = target || api;
    const fn = owner ? owner[method] : null;
    if (typeof fn !== 'function') {
      return Promise.reject(new Error('segmenta: browser api unavailable: ' + method));
    }
    return new Promise((resolve, reject) => {
      let settled = false;
      const finish = (value) => {
        if (settled) {
          return;
        }
        settled = true;
        const error = lastError(api);
        if (error) {
          reject(new Error(error.message || 'segmenta: browser api error'));
          return;
        }
        resolve(value);
      };
      let returned;
      try {
        returned = fn.apply(owner, args.concat([finish]));
      } catch (err) {
        if (!settled) {
          settled = true;
          reject(err);
        }
        return;
      }
      if (returned && typeof returned.then === 'function') {
        returned.then((value) => {
          // Chrome MV3 may expose a promise that resolves undefined while still
          // delivering the reply through the callback; never let that win.
          if (typeof value !== 'undefined') {
            finish(value);
          }
        }, (err) => {
          if (!settled) {
            settled = true;
            reject(err);
          }
        });
        return;
      }
      // A callback is always appended above, so callback-style APIs
      // (chrome.runtime.sendNativeMessage(host, message, callback) declares arity 2)
      // must settle through that callback. Auto-resolving by arity would answer
      // `undefined`; only a synchronous API that actually returned a value resolves
      // from its return.
      if (typeof returned !== 'undefined' && fn.length <= args.length) {
        finish(returned);
      }
    });
  }

  /**
   * Wraps a fire-and-forget call (`alarms.create` and friends). Some browsers
   * answer through a callback, some return an ignored promise and some return
   * nothing at all, so every one of those settles here instead of hanging.
   */
  function invokeVoid(target, method, args) {
    const owner = target || resolveRuntime();
    const fn = owner ? owner[method] : null;
    if (typeof fn !== 'function') {
      return Promise.reject(new Error('segmenta: browser api unavailable: ' + method));
    }
    return new Promise((resolve, reject) => {
      let settled = false;
      const finish = (err) => {
        if (settled) {
          return;
        }
        settled = true;
        if (err) {
          reject(err);
          return;
        }
        resolve(undefined);
      };
      let returned;
      try {
        returned = fn.apply(owner, args.concat([() => {
          const error = lastError(owner);
          finish(error ? new Error(error.message || 'segmenta: browser api error') : null);
        }]));
      } catch (err) {
        finish(err);
        return;
      }
      if (returned && typeof returned.then === 'function') {
        returned.then(() => finish(null), (err) => finish(err || new Error('segmenta: browser api error')));
        return;
      }
      // Firefox returns undefined from alarms.create/clear: the call already landed.
      const error = lastError(owner);
      finish(error ? new Error(error.message || 'segmenta: browser api error') : null);
    });
  }

  /**
   * Sends a message to the background service worker.
   * The background answers with {ok:true, data} or {ok:false, error}.
   */
  function sendMessage(message) {
    const api = resolveRuntime();
    if (!api || !api.runtime) {
      return Promise.reject(new Error('segmenta: runtime unavailable'));
    }
    return invoke(api, api.runtime, 'sendMessage', [message]);
  }

  /**
   * Sends one JSON-RPC 2.0 envelope to the fixed native host.
   */
  function sendNative(message) {
    const api = resolveRuntime();
    if (!api || !api.runtime) {
      return Promise.reject(new Error('segmenta: runtime unavailable'));
    }
    return invoke(api, api.runtime, 'sendNativeMessage', [NATIVE_HOST, message]);
  }

  function getMessage(key, substitutions) {
    const i18n = resolveI18n();
    if (!i18n || typeof i18n.getMessage !== 'function') {
      return '';
    }
    const text = substitutions === undefined
      ? i18n.getMessage(key)
      : i18n.getMessage(key, substitutions);
    return typeof text === 'string' ? text : '';
  }

  function addListener(listener) {
    const api = resolveRuntime();
    if (!api || !api.runtime || !api.runtime.onMessage) {
      return false;
    }
    api.runtime.onMessage.addListener(listener);
    return true;
  }

  function resolveNamespace(api, name) {
    const root = api || resolveRuntime();
    return root && root[name] ? root[name] : null;
  }

  function resolveStorage(api) {
    const root = api || resolveRuntime();
    if (root && root.storage) {
      return root.storage.local || root.storage;
    }
    return null;
  }

  function createEventWrapper(namespaceName, eventName) {
    return {
      addListener(callback) {
        const ns = resolveNamespace(null, namespaceName);
        if (ns && ns[eventName] && typeof ns[eventName].addListener === 'function') {
          ns[eventName].addListener(callback);
          return true;
        }
        return false;
      },
      removeListener(callback) {
        const ns = resolveNamespace(null, namespaceName);
        if (ns && ns[eventName] && typeof ns[eventName].removeListener === 'function') {
          ns[eventName].removeListener(callback);
          return true;
        }
        return false;
      },
    };
  }

  const downloads = {
    onCreated: createEventWrapper('downloads', 'onCreated'),
    // Pausing keeps the bytes the browser already received, so a capture can be
    // confirmed without ever losing data; cancel/erase are only used once the
    // daemon owns the file.
    pause(downloadId) {
      const ns = resolveNamespace(null, 'downloads');
      return invoke(resolveRuntime(), ns, 'pause', [downloadId]);
    },
    resume(downloadId) {
      const ns = resolveNamespace(null, 'downloads');
      return invoke(resolveRuntime(), ns, 'resume', [downloadId]);
    },
    cancel(downloadId) {
      const ns = resolveNamespace(null, 'downloads');
      return invoke(resolveRuntime(), ns, 'cancel', [downloadId]);
    },
    erase(query) {
      const ns = resolveNamespace(null, 'downloads');
      return invoke(resolveRuntime(), ns, 'erase', [query]);
    },
  };

  // The confirmation window is our own extension page: `windows` needs no extra
  // permission and the page never sees anything it cannot display.
  const windows = {
    onRemoved: createEventWrapper('windows', 'onRemoved'),
    create(createData) {
      const ns = resolveNamespace(null, 'windows');
      return invoke(resolveRuntime(), ns, 'create', [createData]);
    },
    /** Existence check for the capture watchdog: a recycled id resolves as gone. */
    get(windowId) {
      const ns = resolveNamespace(null, 'windows');
      return invoke(resolveRuntime(), ns, 'get', [windowId]);
    },
    remove(windowId) {
      const ns = resolveNamespace(null, 'windows');
      return invoke(resolveRuntime(), ns, 'remove', [windowId]);
    },
  };

  // `alarms` is the only timer that survives MV3 service-worker suspension, so a
  // paused browser download is always guarded by one.
  const alarms = {
    onAlarm: createEventWrapper('alarms', 'onAlarm'),
    create(name, alarmInfo) {
      const ns = resolveNamespace(null, 'alarms');
      return invokeVoid(ns, 'create', [name, alarmInfo]);
    },
    clear(name) {
      const ns = resolveNamespace(null, 'alarms');
      return invoke(resolveRuntime(), ns, 'clear', [name]);
    },
  };

  // Runtime lifecycle events the background needs after a suspension or a restart.
  const runtime = {
    onStartup: createEventWrapper('runtime', 'onStartup'),
  };

  const contextMenus = {
    create(createProperties) {
      const ns = resolveNamespace(null, 'contextMenus');
      return invoke(resolveRuntime(), ns, 'create', [createProperties]);
    },
    onClicked: createEventWrapper('contextMenus', 'onClicked'),
  };

  const storage = {
    get(keys) {
      const st = resolveStorage(resolveRuntime());
      return invoke(resolveRuntime(), st, 'get', [keys]);
    },
    set(items) {
      const st = resolveStorage(resolveRuntime());
      return invoke(resolveRuntime(), st, 'set', [items]);
    },
  };

  const api = {
    hostName: HOST_NAME,
    nativeHost: NATIVE_HOST,
    sendMessage: sendMessage,
    sendNative: sendNative,
    getMessage: getMessage,
    addListener: addListener,
    resolveRuntime: resolveRuntime,
    downloads: downloads,
    windows: windows,
    alarms: alarms,
    runtime: runtime,
    contextMenus: contextMenus,
    storage: storage,
  };

  Object.defineProperty(api, 'api', {
    enumerable: true,
    get: resolveRuntime,
  });

  global.SegmentaApi = api;
  if (typeof module !== 'undefined' && module.exports) {
    module.exports = api;
  }
})(typeof globalThis !== 'undefined' ? globalThis : this);

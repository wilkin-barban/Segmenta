// Segmenta page-context sniffer (MAIN world, document_start).
//
// Hooks fetch and XMLHttpRequest to spot HLS/DASH manifests. Detected URLs are
// only ever delivered through the MessagePort handed over by content.js, after
// checking event.source === window; this file never posts to a wildcard target
// and never downloads anything by itself.
(function (global) {
  'use strict';

  const FLAG = '__segmentaSnifferInstalled';
  const CHANNEL = 'segmenta:sniffer:v1';
  const MANIFEST_PATTERN = /\.(m3u8|mpd)([?#].*)?$/i;

  if (global[FLAG]) {
    return;
  }
  global[FLAG] = true;

  let port = null;

  global.addEventListener(
    'message',
    (event) => {
      if (event.source !== global) {
        return;
      }
      if (!event.ports || event.ports.length !== 1) {
        return;
      }
      const data = event.data;
      if (!data || data.channel !== CHANNEL || data.type !== 'segmenta:handshake') {
        return;
      }
      port = event.ports[0];
    },
    false,
  );

  function absoluteUrl(value) {
    if (typeof value !== 'string' || !value) {
      return null;
    }
    if (!MANIFEST_PATTERN.test(value)) {
      return null;
    }
    let parsed;
    try {
      parsed = new URL(value, global.location.href);
    } catch (err) {
      return null;
    }
    if (parsed.protocol !== 'https:') {
      return null;
    }
    return parsed.href;
  }

  function report(value) {
    if (!port) {
      return;
    }
    const url = absoluteUrl(value);
    if (!url) {
      return;
    }
    try {
      port.postMessage({ channel: CHANNEL, type: 'segmenta:media', url: url });
    } catch (err) {
      port = null;
    }
  }

  const nativeFetch = global.fetch;
  if (typeof nativeFetch === 'function') {
    global.fetch = function segmentaFetch(input) {
      try {
        if (typeof input === 'string') {
          report(input);
        } else if (input && typeof input.url === 'string') {
          report(input.url);
        }
      } catch (err) {
        // sniffing must never break the page
      }
      return nativeFetch.apply(this, arguments);
    };
  }

  const Xhr = global.XMLHttpRequest;
  if (Xhr && Xhr.prototype && typeof Xhr.prototype.open === 'function') {
    const nativeOpen = Xhr.prototype.open;
    Xhr.prototype.open = function segmentaOpen(method, url) {
      try {
        report(url);
      } catch (err) {
        // sniffing must never break the page
      }
      return nativeOpen.apply(this, arguments);
    };
  }
})(typeof globalThis !== 'undefined' ? globalThis : this);

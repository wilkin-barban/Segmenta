// Panel state tests: the quality list and the "sent to Segmenta" handoff (S9).
//
// No npm dependencies, so this file carries a tiny fake DOM: only the handful of
// node operations panel.js actually uses (createElement, appendChild, removeChild,
// firstChild, textContent, setAttribute, addEventListener, querySelectorAll,
// attachShadow) is implemented.
import assert from 'node:assert/strict';
import { loadShared } from './helpers.mjs';

const TASK_ID = '123e4567-e89b-12d3-a456-426614174000';

const LABELS = {
  panelTitle: 'Segmenta',
  panelClose: 'Cerrar',
  panelLoading: 'Consultando calidades...',
  qualityLabel: 'Calidad de video',
  audioOnly: 'Solo audio',
  playlistLimitLabel: 'Limite de la playlist',
  playlistLimitHint: 'Entre 1 y 500.',
  startDownload: 'Descargar',
  streamDetected: 'Flujo detectado',
  statusPreparing: 'Preparando la descarga...',
  statusCompleted: 'Descarga completada',
  errorGeneric: 'No se pudo completar la operacion.',
  sentTitle: 'Enviado a Segmenta',
  openSegmenta: 'Abrir Segmenta',
  viewProgress: 'Ver progreso',
};

const PROBE = {
  kind: 'video',
  title: 'Demo / Video',
  duration_seconds: 213,
  thumbnail: null,
  formats: [
    { format_id: '137', label: '1080p', height: 1080, ext: 'mp4', kind: 'video' },
    { format_id: '248', label: '1080p', height: 1080, ext: 'webm', kind: 'video' },
    { format_id: '22', label: '720p', height: 720, ext: 'mp4', kind: 'video' },
    { format_id: '135', label: '480p', height: 480, ext: 'mp4', kind: 'video' },
    { format_id: '134', label: '360p', height: 360, ext: 'mp4', kind: 'video' },
    { format_id: '160', label: '240p', height: 240, ext: 'mp4', kind: 'video' },
  ],
  audio: [{ format_id: '251', label: 'webm', ext: 'webm', kind: 'audio' }],
  qualities: [
    { quality: '1080', label: '1080p', height: 1080, format_id: '137' },
    { quality: '720', label: '720p', height: 720, format_id: '22' },
    { quality: '360', label: '360p', height: 360, format_id: '134' },
    { quality: 'audio', label: 'audio', height: null, format_id: '251' },
  ],
  default_quality: '720',
  playlist: { count: 0, entries: [] },
};

class FakeNode {
  constructor(tag, document) {
    this.tagName = String(tag).toUpperCase();
    this.ownerDocument = document;
    this.children = [];
    this.listeners = {};
    this.attributes = {};
    this.style = {};
    this.className = '';
    this.textContent = '';
    this.hidden = false;
    this.disabled = false;
    this.checked = false;
    this.value = '';
    this.type = '';
    this.name = '';
    this.id = '';
    this.title = '';
    this.parentNode = null;
  }

  get firstChild() {
    return this.children.length ? this.children[0] : null;
  }

  appendChild(node) {
    if (node && node.parentNode) {
      node.parentNode.removeChild(node);
    }
    if (node) {
      node.parentNode = this;
    }
    this.children.push(node);
    return node;
  }

  removeChild(node) {
    this.children = this.children.filter((child) => child !== node);
    if (node) {
      node.parentNode = null;
    }
    return node;
  }

  setAttribute(name, value) {
    this.attributes[name] = String(value);
  }

  getAttribute(name) {
    return this.attributes[name];
  }

  addEventListener(type, handler) {
    if (!this.listeners[type]) {
      this.listeners[type] = [];
    }
    this.listeners[type].push(handler);
  }

  dispatch(type) {
    for (const handler of this.listeners[type] || []) {
      handler({ type: type, target: this });
    }
  }

  // Only the `input` selector is used by the panel.
  querySelectorAll() {
    const found = [];
    const walk = (node) => {
      for (const child of node.children || []) {
        if (child && child.tagName === 'INPUT') {
          found.push(child);
        }
        walk(child);
      }
    };
    walk(this);
    return found;
  }

  attachShadow() {
    this.shadowRoot = new FakeNode('shadow-root', this.ownerDocument);
    return this.shadowRoot;
  }
}

function createDocument() {
  const document = {
    createElement(tag) {
      return new FakeNode(tag, document);
    },
    createTextNode(value) {
      const node = new FakeNode('#text', document);
      node.textContent = String(value);
      return node;
    },
    addEventListener() {},
    readyState: 'complete',
  };
  document.body = new FakeNode('body', document);
  return document;
}

function findAll(node, predicate, found) {
  const out = found || [];
  if (!node) {
    return out;
  }
  if (predicate(node)) {
    out.push(node);
  }
  for (const child of node.children || []) {
    findAll(child, predicate, out);
  }
  findAll(node.shadowRoot, predicate, out);
  return out;
}

function byClass(root, className) {
  const matches = findAll(root, (node) => node.className === className);
  assert.equal(matches.length, 1, `expected exactly one .${className}`);
  return matches[0];
}

function radios(root) {
  return findAll(root, (node) => node.tagName === 'INPUT' && node.type === 'radio');
}

export default function suite({ test }) {
  test('the panel renders only the grouped ladder and preselects 720p', async () => {
    await loadShared('SegmentaProtocol', 'protocol.js');
    const PANEL = await loadShared('SegmentaPanel', 'panel.js');
    const document = createDocument();
    const previous = globalThis.document;
    globalThis.document = document;
    try {
      const panel = PANEL.createPanel({ t: (key) => LABELS[key] || key });
      panel.open(PROBE);

      const options = radios(panel.root);
      assert.deepEqual(
        options.map((input) => input.value),
        ['quality:1080', 'quality:720', 'quality:360', 'quality:audio'],
      );
      const checked = options.filter((input) => input.checked);
      assert.equal(checked.length, 1);
      assert.equal(checked[0].value, 'quality:720');
      const labels = findAll(panel.root, (node) => node.className === 'segmenta-option-label');
      assert.deepEqual(
        labels.map((node) => node.textContent),
        ['1080p', '720p', '360p', 'Solo audio'],
      );
    } finally {
      globalThis.document = previous;
    }
  });

  test('the handoff state replaces the format list with both task actions', async () => {
    await loadShared('SegmentaProtocol', 'protocol.js');
    const PANEL = await loadShared('SegmentaPanel', 'panel.js');
    const document = createDocument();
    const previous = globalThis.document;
    globalThis.document = document;
    try {
      const calls = { probe: 0, start: [], open: [] };
      const panel = PANEL.createPanel({
        t: (key) => LABELS[key] || key,
        onProbe: () => {
          calls.probe += 1;
        },
        onStart: (request) => {
          calls.start.push(request);
        },
        onOpenTask: (taskId) => {
          calls.open.push(taskId);
        },
      });
      panel.open(PROBE);

      // "Descargar" carries the selected height and no raw format id.
      byClass(panel.root, 'segmenta-start').dispatch('click');
      assert.deepEqual(calls.start, [{ mode: 'video', height: 720 }]);

      // A successful add switches the card to the sent state.
      panel.setSent(TASK_ID);
      assert.equal(byClass(panel.root, 'segmenta-sent').hidden, false);
      assert.equal(byClass(panel.root, 'segmenta-targets').hidden, true);
      assert.equal(byClass(panel.root, 'segmenta-start').hidden, true);
      assert.deepEqual(radios(panel.root), []);
      assert.equal(byClass(panel.root, 'segmenta-formats').children.length, 0);
      assert.equal(
        byClass(panel.root, 'segmenta-sent-title').textContent,
        'Enviado a Segmenta',
      );

      const openSegmenta = byClass(panel.root, 'segmenta-open-task');
      const viewProgress = byClass(panel.root, 'segmenta-open-progress');
      assert.equal(openSegmenta.textContent, 'Abrir Segmenta');
      assert.equal(viewProgress.textContent, 'Ver progreso');

      // Both actions open the very same task and never add it again.
      openSegmenta.dispatch('click');
      viewProgress.dispatch('click');
      assert.deepEqual(calls.open, [TASK_ID, TASK_ID]);
      assert.equal(calls.start.length, 1);
      assert.equal(calls.probe, 0);
    } finally {
      globalThis.document = previous;
    }
  });

  test('a new probe brings the format list back after the handoff', async () => {
    await loadShared('SegmentaProtocol', 'protocol.js');
    const PANEL = await loadShared('SegmentaPanel', 'panel.js');
    const document = createDocument();
    const previous = globalThis.document;
    globalThis.document = document;
    try {
      const panel = PANEL.createPanel({ t: (key) => LABELS[key] || key });
      panel.open(PROBE);
      panel.setSent(TASK_ID);
      panel.open(PROBE);
      assert.equal(byClass(panel.root, 'segmenta-sent').hidden, true);
      assert.equal(byClass(panel.root, 'segmenta-targets').hidden, false);
      assert.equal(byClass(panel.root, 'segmenta-start').hidden, false);
      assert.equal(radios(panel.root).length, 4);
    } finally {
      globalThis.document = previous;
    }
  });
}

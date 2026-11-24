// Popup downloads: the recent list and its quick actions (S1, T5 of Fase 2).
//
// The message surface is validated in shared/validate.js and built in shared/protocol.js;
// a fake DOM drives popup.js so the list, the progress text and the buttons are checked
// without a browser.
import assert from 'node:assert/strict';
import vm from 'node:vm';
import { loadShared, readExt } from './helpers.mjs';

const TASK_ID = '123e4567-e89b-12d3-a456-426614174000';
const OTHER_ID = '223e4567-e89b-12d3-a456-426614174000';

/** Objects sent from the popup live in another VM realm; compare plain copies. */
function plain(value) {
  return JSON.parse(JSON.stringify(value));
}

const LABELS = {
  popupTitle: 'Segmenta',
  popupUrlLabel: 'URL',
  popupUrlPlaceholder: 'https://...',
  popupModeLabel: 'Modo',
  popupModeVideo: 'Video',
  popupModeAudio: 'Audio',
  popupStart: 'Enviar',
  downloadsTitle: 'Descargas recientes',
  downloadsEmpty: 'No hay descargas todavía.',
  downloadsError: 'No se pudo consultar el daemon.',
  downloadsRefresh: 'Actualizar',
  downloadsPaused: 'En pausa',
  downloadsCompleted: 'Completada',
  actionPause: 'Pausar',
  actionResume: 'Reanudar',
  actionCancel: 'Cancelar',
};

/** Minimal DOM: only the operations popup.js uses. */
function fakeDom() {
  const nodes = new Map();
  function element(tag) {
    const node = {
      tagName: tag,
      children: [],
      attributes: {},
      listeners: {},
      className: '',
      textContent: '',
      title: '',
      type: '',
      disabled: false,
      value: '',
      get firstChild() {
        return node.children[0] || null;
      },
      appendChild(child) {
        node.children.push(child);
        return child;
      },
      removeChild(child) {
        node.children = node.children.filter((entry) => entry !== child);
        return child;
      },
      setAttribute(name, value) {
        node.attributes[name] = value;
      },
      addEventListener(name, handler) {
        (node.listeners[name] = node.listeners[name] || []).push(handler);
      },
    };
    return node;
  }
  for (const id of [
    'title', 'url-label', 'url', 'mode-label', 'mode-video', 'mode-audio', 'start', 'status',
    'downloads-title', 'downloads-list', 'downloads-refresh',
  ]) {
    nodes.set(id, element('div'));
  }
  nodes.set('url', Object.assign(element('input'), { value: '' }));
  nodes.get('start').disabled = false;
  return {
    nodes,
    document: {
      readyState: 'complete',
      getElementById: (id) => nodes.get(id) || null,
      createElement: element,
      querySelectorAll: () => [],
      addEventListener: () => {},
    },
  };
}

/** Load popup.js against a fake DOM and a fake extension API. */
async function bootPopup(replies) {
  const dom = fakeDom();
  const sent = [];
  const api = {
    getMessage: (key) => LABELS[key] || key,
    sendMessage: async (message) => {
      sent.push(message);
      const reply = replies(message);
      if (reply instanceof Error) {
        throw reply;
      }
      return reply;
    },
  };
  const context = {
    document: dom.document,
    SegmentaApi: api,
    SegmentaProtocol: { isAllowedUrl: () => true },
    console,
  };
  context.globalThis = context;
  vm.createContext(context);
  // `readyState` is `complete`, so popup.js boots itself during the evaluation.
  vm.runInContext(readExt('chromium', 'popup.js'), context);
  await new Promise((resolve) => setImmediate(resolve));
  return { dom, sent };
}

export default function suite({ test }) {
  test('lists recent downloads with their state', async () => {
  const { dom, sent } = await bootPopup((message) => {
    if (message.type !== 'segmenta.list') {
      return { ok: true };
    }
    return {
      ok: true,
      result: [
        { task_id: TASK_ID, filename: 'a.bin', status: 'downloading', downloaded_bytes: 50, total_bytes: 100 },
        { task_id: OTHER_ID, filename: 'b.bin', status: 'paused', downloaded_bytes: 0, total_bytes: 0 },
      ],
    };
  });
  assert.deepEqual(plain(sent[0]), { type: 'segmenta.list', filter: 'all', limit: 10 });
  const list = dom.nodes.get('downloads-list');
  assert.equal(list.children.length, 2);
  assert.equal(list.children[0].children[0].textContent, 'a.bin');
  assert.equal(list.children[0].children[1].textContent, '50%');
  assert.equal(list.children[1].children[1].textContent, 'En pausa');
});

  test('offers only the actions the state allows and sends the right message', async () => {
  const { dom, sent } = await bootPopup((message) => {
    if (message.type === 'segmenta.list') {
      return {
        ok: true,
        result: [
          { task_id: TASK_ID, filename: 'a.bin', status: 'downloading', downloaded_bytes: 1, total_bytes: 2 },
          { task_id: OTHER_ID, filename: 'b.bin', status: 'paused', downloaded_bytes: 0, total_bytes: 0 },
        ],
      };
    }
    return { ok: true };
  });
  const list = dom.nodes.get('downloads-list');
  const running = list.children[0];
  const paused = list.children[1];
  const labels = (row) => row.children.slice(2).map((node) => node.textContent);
  assert.deepEqual(labels(running), ['Pausar', 'Cancelar']);
  assert.deepEqual(labels(paused), ['Reanudar', 'Cancelar']);

  // Click "pause" on the running row.
  const button = running.children[2];
  await button.listeners.click[0]();
  const action = sent.find((message) => message.type === 'segmenta.action');
  assert.deepEqual(plain(action), { type: 'segmenta.action', action: 'pause', taskId: TASK_ID });
});

  test('reports an empty list and a daemon failure without inventing rows', async () => {
  const empty = await bootPopup(() => ({ ok: true, result: [] }));
  assert.equal(
    empty.dom.nodes.get('downloads-list').children[0].textContent,
    'No hay descargas todavía.'
  );

  const failing = await bootPopup(() => new Error('host down'));
  assert.equal(
    failing.dom.nodes.get('downloads-list').children[0].textContent,
    'No se pudo consultar el daemon.'
  );
});

  test('validation refuses unknown filters, actions and task ids', async () => {
  const validate = await loadShared('SegmentaValidate', 'validate.js');
  const accept = validate.validateMessage({ type: 'segmenta.list', filter: 'active', limit: 5 }, { fromPopup: true });
  assert.equal(accept.ok, true);
  assert.deepEqual(plain(accept.value), { type: 'segmenta.list', filter: 'active', limit: 5 });
  assert.equal(validate.validateMessage({ type: 'segmenta.list', filter: 'mine' }).ok, false);
  assert.equal(validate.validateMessage({ type: 'segmenta.list', limit: 51 }).ok, false);
  assert.equal(
    validate.validateMessage({ type: 'segmenta.action', action: 'delete', taskId: TASK_ID }).ok,
    false
  );
  assert.equal(
    validate.validateMessage({ type: 'segmenta.action', action: 'pause', taskId: '../etc' }).ok,
    false
  );
  const action = validate.validateMessage({ type: 'segmenta.action', action: 'cancel', taskId: TASK_ID });
  assert.equal(action.ok, true);
  assert.deepEqual(plain(action.value), { type: 'segmenta.action', action: 'cancel', taskId: TASK_ID });
});

  test('the protocol builds the daemon requests the popup needs', async () => {
  const protocol = await loadShared('SegmentaProtocol', 'protocol.js');
  const list = protocol.buildDownloadList({ filter: 'active', limit: 5, id: 7 });
  assert.equal(list.method, 'download.list');
  assert.deepEqual(list.params, { filter: 'active', limit: 5, offset: 0 });
  assert.throws(() => protocol.buildDownloadList({ limit: 0 }), /limit/);

  const pause = protocol.buildDownloadAction({ action: 'pause', taskId: TASK_ID });
  assert.equal(pause.method, 'download.pause');
  assert.deepEqual(pause.params, { task_id: TASK_ID });

  const cancel = protocol.buildDownloadAction({ action: 'cancel', taskId: TASK_ID });
  assert.equal(cancel.method, 'download.cancel');
  assert.deepEqual(cancel.params, { task_id: TASK_ID, delete_file: false });
  assert.throws(() => protocol.buildDownloadAction({ action: 'delete', taskId: TASK_ID }), /action/);
});
}

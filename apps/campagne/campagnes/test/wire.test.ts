// The controller on top of the real shell client, with a fake fetch and a fake
// event stream written here (no socket, no import of the shell's test fakes):
// what the Micro-UI puts on the wire, and a full create → refresh round trip.

import { createShellClient, type EventSourceLike } from '@dnd-helper/micro-ui-shell';
import { describe, expect, it } from 'vitest';
import { createCampagnesController, type Campagne } from '../src/controller';
import { flush } from './fakes';

const BASE = 'http://127.0.0.1:7878';
const CMD = '11111111-1111-4111-8111-111111111111';
const PARTITION = 'Campagne/22222222-2222-4222-8222-222222222222';

const json = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

const entry = (state: string): Record<string, unknown> => ({
  command: CMD,
  dataCapability: 'campagne.creerCampagne@1',
  by: 'gm',
  partition: PARTITION,
  position: 0,
  basedOn: { version: 3 },
  state,
  confirmation: null,
  parked: null,
  requeuedFrom: null,
});

const result = (status: string, extra: Record<string, unknown> = {}): Record<string, unknown> => ({
  commandId: CMD,
  status,
  dataVersion: status === 'applied' ? 5 : null,
  violations: [],
  reviewId: null,
  ...extra,
});

class FakeStream implements EventSourceLike {
  readyState = 1;
  private readonly listeners = new Map<string, ((event: { data?: unknown }) => void)[]>();
  addEventListener(type: string, listener: (event: { data?: unknown }) => void): void {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }
  close(): void {
    this.readyState = 2;
  }
  push(version: number): void {
    for (const l of this.listeners.get('dataVersion') ?? []) l({ data: JSON.stringify({ dataVersion: version }) });
  }
}

interface Call {
  method: string;
  path: string;
  body: Record<string, unknown> | undefined;
}

function server(state: { list: Campagne[]; asOf: number; submit: () => Response; lookup: () => Response }) {
  const calls: Call[] = [];
  const streams: FakeStream[] = [];
  const fetchFake: typeof globalThis.fetch = (input, init) => {
    const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url);
    const method = init?.method ?? 'GET';
    const body = typeof init?.body === 'string' ? (JSON.parse(init.body) as Record<string, unknown>) : undefined;
    calls.push({ method, path: url.pathname, body });
    if (method === 'POST' && url.pathname === '/capabilities/campagne.listerCampagnes') return Promise.resolve(json({ asOf: state.asOf, data: state.list }));
    if (method === 'POST' && url.pathname === '/commands') return Promise.resolve(state.submit());
    if (method === 'GET' && url.pathname === `/commands/${CMD}`) return Promise.resolve(state.lookup());
    return Promise.resolve(json({ error: 'no-route' }, 404));
  };
  const client = createShellClient({
    baseUrl: BASE,
    fetch: fetchFake,
    eventSource: () => {
      const stream = new FakeStream();
      streams.push(stream);
      return stream;
    },
    poll: { initialMs: 1, maxMs: 2, factor: 2 },
  });
  return { calls, streams, client };
}

const accepted = (state = 'queued'): Response =>
  json({ commandId: CMD, partition: PARTITION, replayed: false, warnings: [], entry: entry(state), messages: [] }, 202);

async function until(check: () => boolean): Promise<void> {
  for (let i = 0; i < 1000 && !check(); i += 1) await new Promise((resolve) => setTimeout(resolve, 2));
  expect(check()).toBe(true);
}

function mount(s: ReturnType<typeof server>) {
  const contexts: (string | null)[] = [];
  const controller = createCampagnesController({ shell: s.client, setContext: (_n, v) => contexts.push(v), confirm: () => true, newIdempotencyKey: () => 'wire-key' });
  return { controller, contexts };
}

describe('on the wire', () => {
  it('reads the list through the identifier route with no variable of its own', async () => {
    const s = server({ list: [{ id: 'a', name: 'Alpha' }], asOf: 3, submit: () => accepted(), lookup: () => json({ result: result('applied') }) });
    const { controller } = mount(s);
    await until(() => controller.getState().list.kind === 'ready');
    const read = s.calls.find((c) => c.path.startsWith('/capabilities/'))!;
    expect(read).toMatchObject({ method: 'POST', path: '/capabilities/campagne.listerCampagnes', body: { variables: {} } });
    controller.dispose();
    s.client.dispose();
  });

  it('submits the raw name to /commands with a key and no target', async () => {
    const s = server({ list: [], asOf: 3, submit: () => accepted(), lookup: () => json({ result: result('applied') }) });
    const { controller } = mount(s);
    controller.setName('  Alpha  ');
    await controller.submitCreate();
    const sent = s.calls.find((c) => c.path === '/commands')!;
    expect(sent.body).toEqual({ dataCapability: 'campagne.creerCampagne@1', payload: { name: '  Alpha  ' }, idempotencyKey: 'wire-key' });
    expect(sent.body).not.toHaveProperty('target');
    expect(controller.getState().create.phase).toMatchObject({ kind: 'settled', status: 'applied', dataVersion: 5 });
    controller.dispose();
    s.client.dispose();
  });

  it('submits a confirmed archive with the target id and an empty payload', async () => {
    const s = server({ list: [{ id: 'a', name: 'Alpha' }], asOf: 3, submit: () => accepted(), lookup: () => json({ result: result('applied') }) });
    const { controller } = mount(s);
    await until(() => controller.getState().list.kind === 'ready');
    await controller.requestArchive('a');
    const sent = s.calls.find((c) => c.path === '/commands')!;
    expect(sent.body).toEqual({ dataCapability: 'campagne.archiverCampagne@1', target: { id: 'a' }, payload: {}, idempotencyKey: 'wire-key' });
    controller.dispose();
    s.client.dispose();
  });

  it('shows the refused command and the list unchanged when the DataGuard rejects (rules 13–15)', async () => {
    const s = server({
      list: [{ id: 'a', name: 'Alpha' }],
      asOf: 3,
      submit: () => accepted(),
      lookup: () => json({ result: result('rejected', { violations: ['campaign-name-required'] }) }),
    });
    const { controller } = mount(s);
    await until(() => controller.getState().list.kind === 'ready');
    controller.setName('   ');
    await controller.submitCreate();
    const { create, list } = controller.getState();
    expect(create.fieldErrors.name).toEqual(['Le nom de la campagne est obligatoire.']);
    expect(create.name).toBe('   ');
    expect(list.kind === 'ready' && list.rows).toEqual([{ id: 'a', name: 'Alpha' }]);
    expect(s.calls.filter((c) => c.path.startsWith('/capabilities/'))).toHaveLength(1);
    controller.dispose();
    s.client.dispose();
  });

  it('refreshes by itself when the dataVersion moves, keeping the typed text', async () => {
    const state = { list: [{ id: 'a', name: 'Alpha' }] as Campagne[], asOf: 3, submit: () => accepted(), lookup: () => json({ result: result('applied') }) };
    const s = server(state);
    const { controller } = mount(s);
    await until(() => controller.getState().list.kind === 'ready');
    controller.setName('Brouillon');
    // Another tab wrote: the stream announces version 4 and the next read returns it.
    state.list = [{ id: 'b', name: 'Beta' }, ...state.list];
    state.asOf = 4;
    s.streams[0]!.push(4);
    await until(() => {
      const list = controller.getState().list;
      return list.kind === 'ready' && list.rows.length === 2;
    });
    const list = controller.getState().list;
    expect(list.kind === 'ready' && list.asOf).toBe(4);
    expect(controller.getState().create.name).toBe('Brouillon');
    await flush();
    controller.dispose();
    s.client.dispose();
  });
});

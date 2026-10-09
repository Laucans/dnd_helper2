// The frozen screen `apps/screens/campagne.json`, proved two ways.
//
// 1. As a file. `compositions-resolve` reads `.microUi` values and string layout items, and contract E
//    carries ids as object keys, so that gate resolves nothing here (SPEC rule 7). The match of every
//    layout key to a `micro-ui.json`, and of every prop name to a key of that manifest's `props`, is
//    checked directly below, and a screen whose key resolves to nothing is shown to fail.
// 2. As a page. A minimal host, inside this test only, reads the file from disk, mounts each Micro-UI
//    through its public entry point on ONE real shell client, holds the `$ctx` map and gives
//    `setContext` to the Micro-UI that selects. Nothing is served: the transport is a fake loopback
//    `fetch` and a fake `EventSource` (issue #47, option B), so the reads that go out are the ones the
//    Micro-UIs make by identifier, and the page never reloads: it only hears of a new `dataVersion`.
//
// There is no DOM test environment in this repository and none is added: the party level is mounted
// on the smallest element it touches.

import { readdirSync, readFileSync } from 'node:fs';
import { basename } from 'node:path';
import { createShellClient, type EventSourceLike, type ShellClient, type Timers } from '@dnd-helper/micro-ui-shell';
// eslint-disable-next-line no-restricted-imports -- the screen host composes Micro-UIs; it is not one
import { CONTEXT_NAME, createCampagnesController, type CampagnesController } from '@dnd-helper/ui-campagnes';
// eslint-disable-next-line no-restricted-imports -- the screen host composes Micro-UIs; it is not one
import { mount as mountNiveau, type MountedNiveauDuGroupe } from '@dnd-helper/ui-campagne-niveau-du-groupe';
// eslint-disable-next-line no-restricted-imports -- the screen host composes Micro-UIs; it is not one
import { createPjsController, type PjsController } from '@dnd-helper/ui-campagne-pjs';
import { describe, expect, it } from 'vitest';

// ---------------------------------------------------------------------------------------------
// The file, read as it is on disk.
// ---------------------------------------------------------------------------------------------

const repo = new URL('../../../../', import.meta.url);

type Layout = Record<string, Record<string, string>>[];

interface ScreenFile {
  screen: string;
  generatedFrom: string;
  version: number;
  layout: Layout;
}

interface ManifestFile {
  microUi: string;
  props: Record<string, string>;
}

function readScreen(): { raw: string; screen: ScreenFile } {
  const raw = readFileSync(new URL('apps/screens/campagne.json', repo), 'utf8');
  return { raw, screen: JSON.parse(raw) as ScreenFile };
}

/** Every `micro-ui.json` under `apps/`, by its `microUi`. */
function readManifests(): Map<string, ManifestFile> {
  const manifests = new Map<string, ManifestFile>();
  const apps = new URL('apps/', repo);
  for (const path of readdirSync(apps, { recursive: true, encoding: 'utf8' })) {
    if (basename(path) !== 'micro-ui.json' || path.includes('node_modules')) continue;
    const manifest = JSON.parse(readFileSync(new URL(path, apps), 'utf8')) as ManifestFile;
    // Two manifests for one id would make a layout key ambiguous: that is a defect, not a last-one-wins.
    if (manifests.has(manifest.microUi)) throw new Error(`${manifest.microUi} is declared by more than one micro-ui.json`);
    manifests.set(manifest.microUi, manifest);
  }
  return manifests;
}

/** What is wrong with a screen's layout against the manifests; empty when it resolves. */
function unresolved(screen: ScreenFile, manifests: Map<string, ManifestFile>): string[] {
  const problems: string[] = [];
  for (const item of screen.layout) {
    const keys = Object.keys(item);
    if (keys.length !== 1) problems.push(`an item has ${String(keys.length)} keys, not 1`);
    for (const key of keys) {
      const manifest = manifests.get(key);
      if (manifest === undefined) {
        problems.push(`${key}: no micro-ui.json declares it`);
        continue;
      }
      for (const prop of Object.keys(item[key] ?? {})) {
        if (!(prop in manifest.props)) problems.push(`${key}.${prop}: not a key of the manifest's props`);
      }
    }
  }
  return problems;
}

const CTX = /^\$ctx\.([A-Za-z][A-Za-z0-9_]*)$/;

describe('apps/screens/campagne.json (contract E)', () => {
  const { raw, screen } = readScreen();
  const manifests = readManifests();

  it('is the campagne screen, versioned, with its origin', () => {
    expect(screen.screen).toBe('campagne');
    expect(Number.isInteger(screen.version)).toBe(true);
    expect(screen.version).toBeGreaterThanOrEqual(1);
    expect(typeof screen.generatedFrom).toBe('string');
    expect(screen.generatedFrom.trim()).not.toBe('');
  });

  it('lays out the campaigns, the PCs and the party level, in that order, one key each', () => {
    expect(screen.layout.map((item) => Object.keys(item))).toEqual([['Campagnes'], ['Pjs'], ['NiveauDuGroupe']]);
  });

  it('resolves every layout key to a micro-ui.json and every prop to a key of its props', () => {
    expect(unresolved(screen, manifests)).toEqual([]);
  });

  it('would fail on a layout key no manifest declares, or on a prop it does not have', () => {
    const unknownKey: ScreenFile = { ...screen, layout: [...screen.layout, { Inconnu: { campagneId: '$ctx.campagne' } }] };
    expect(unresolved(unknownKey, manifests)).toEqual(['Inconnu: no micro-ui.json declares it']);
    const unknownProp: ScreenFile = { ...screen, layout: [{ Pjs: { sessionId: '$ctx.campagne' } }] };
    expect(unresolved(unknownProp, manifests)).toEqual(["Pjs.sessionId: not a key of the manifest's props"]);
  });

  it('shares one selected-campaign context, the one the Campagnes Micro-UI writes', () => {
    const names = new Set<string>();
    for (const item of screen.layout) {
      for (const props of Object.values(item)) {
        for (const value of Object.values(props)) {
          const match = CTX.exec(value);
          expect(match, `${value} is a $ctx reference`).not.toBeNull();
          names.add(match![1]!);
        }
      }
    }
    expect([...names]).toEqual([CONTEXT_NAME]);
  });

  it('names Micro-UIs only: no Capability, DataCapability, query, table or hash', () => {
    expect(raw).not.toMatch(/\.graphql|\.sql|[0-9a-f]{64}/);
    for (const item of screen.layout) {
      for (const key of Object.keys(item)) expect(key).toMatch(/^[A-Z][A-Za-z0-9]*$/);
    }
    for (const value of Object.values(screen.layout).flatMap((item) => Object.values(item).flatMap((props) => Object.values(props)))) {
      expect(value).toMatch(CTX);
    }
  });
});

// ---------------------------------------------------------------------------------------------
// The doubles: no socket, no real timer, no DOM.
// ---------------------------------------------------------------------------------------------

const A = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
const B = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';

interface Pc {
  id: string;
  name: string;
  class: string;
  level: number;
}

/** The Data the Capabilities would read, and the version it is at. */
class FakeServer {
  version = 1;
  campaigns = [
    { id: B, name: 'Les Mouettes' },
    { id: A, name: 'Les Brumes' },
  ];
  pcs = new Map<string, Pc[]>([
    [
      A,
      [
        { id: 'pc-a1', name: 'Ysolde', class: 'Barde', level: 3 },
        { id: 'pc-a2', name: 'Brannoc', class: 'Guerrier', level: 4 },
      ],
    ],
    [B, [{ id: 'pc-b1', name: 'Cyrielle', class: 'Magicienne', level: 7 }]],
  ]);

  /** Applies a change to the Data and moves to the next version, as the applier does. */
  commit(change: () => void): number {
    change();
    this.version += 1;
    return this.version;
  }

  answer(path: string, variables: Record<string, unknown>): Response {
    const json = (body: unknown, status = 200): Response =>
      new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
    const asOf = this.version;
    if (path === '/capabilities/campagne.listerCampagnes') return json({ asOf, data: this.campaigns });

    const id = typeof variables['campagneId'] === 'string' ? variables['campagneId'] : '';
    const pcs = this.pcs.get(id);
    // An unknown or archived campaign: the one not-found answer.
    if (pcs === undefined || !this.campaigns.some((c) => c.id === id)) return json({ error: 'not-found' }, 404);
    if (path === '/capabilities/campagne.listerPjs') return json({ asOf, data: pcs });
    if (path === '/capabilities/campagne.niveauDuGroupe') {
      const count = pcs.length;
      const sum = pcs.reduce((total, pc) => total + pc.level, 0);
      const level = count === 0 ? null : Math.floor((2 * sum + count) / (2 * count)); // round-half-up, integers only
      return json({ asOf, data: { level, pcCount: count, model: 'v1', asOf } });
    }
    throw new TypeError(`unexpected request ${path}`);
  }
}

interface Call {
  method: string;
  path: string;
  body: string | undefined;
}

class FakeEventSource implements EventSourceLike {
  readyState = 1;
  private readonly handlers = new Map<string, ((event: { data?: unknown }) => void)[]>();

  addEventListener(type: string, listener: (event: { data?: unknown }) => void): void {
    this.handlers.set(type, [...(this.handlers.get(type) ?? []), listener]);
  }

  close(): void {
    this.readyState = 2;
  }

  /** The server announces a `dataVersion`. */
  push(version: number): void {
    for (const handler of this.handlers.get('dataVersion') ?? []) handler({ data: JSON.stringify({ dataVersion: version }) });
  }
}

const settle = (): Promise<void> => new Promise((resolve) => setImmediate(resolve));

class FakeClock implements Timers {
  private time = 0;
  private nextId = 1;
  private timers: { id: number; at: number; fn: () => void }[] = [];

  now(): number {
    return this.time;
  }

  setTimeout(fn: () => void, ms: number): unknown {
    const timer = { id: this.nextId++, at: this.time + ms, fn };
    this.timers.push(timer);
    return timer.id;
  }

  clearTimeout(handle: unknown): void {
    this.timers = this.timers.filter((t) => t.id !== handle);
  }

  /** Lets resolved promises run, fires the timers due within `ms` in order, and lets promises run again. */
  async advance(ms: number): Promise<void> {
    const target = this.time + ms;
    await settle();
    for (;;) {
      const due = this.timers.filter((t) => t.at <= target).sort((a, b) => a.at - b.at || a.id - b.id)[0];
      if (due === undefined) break;
      this.timers = this.timers.filter((t) => t !== due);
      this.time = Math.max(this.time, due.at);
      due.fn();
      await settle();
    }
    this.time = target;
    await settle();
  }

  tick(): Promise<void> {
    return this.advance(0);
  }
}

/** The smallest element the party-level Micro-UI touches (`ownerDocument`, `dataset`, `append`...). */
class FakeElement {
  readonly children: FakeElement[] = [];
  readonly dataset: Record<string, string> = {};
  private parent: FakeElement | null = null;
  private text = '';

  constructor(readonly tag: string) {}

  get textContent(): string {
    return this.text;
  }

  set textContent(value: string) {
    this.text = value;
  }

  get ownerDocument(): { createElement(tag: string): FakeElement } {
    return { createElement: (tag) => new FakeElement(tag) };
  }

  setAttribute(): void {}

  append(child: FakeElement): void {
    child.parent = this;
    this.children.push(child);
  }

  remove(): void {
    this.parent?.children.splice(this.parent.children.indexOf(this), 1);
    this.parent = null;
  }
}

// ---------------------------------------------------------------------------------------------
// The host: reads the frozen file, mounts each Micro-UI, holds `$ctx`.
// ---------------------------------------------------------------------------------------------

interface Entry {
  /** The Micro-UI that writes the context; the others read it. */
  provider: boolean;
  /** Prop name to `$ctx.<name>`, as the file says. */
  props: Record<string, string>;
  /** Pushed the resolved props of a consumer whenever a name it reads changes. */
  apply?: (props: Record<string, string | null>) => void;
  dispose(): void;
}

interface ScreenHost {
  ctx: Map<string, string | null>;
  /** Every `setContext(name, value)`, in order. */
  writes: [string, string | null][];
  campagnes: CampagnesController;
  pjs: PjsController;
  niveau: MountedNiveauDuGroupe;
  niveauElement: FakeElement;
  dispose(): void;
}

function mountScreen(screen: ScreenFile, shell: ShellClient): ScreenHost {
  const ctx = new Map<string, string | null>();
  const writes: [string, string | null][] = [];
  const entries: Entry[] = [];
  const niveauElement = new FakeElement('div');
  const resolve = (props: Record<string, string>): Record<string, string | null> =>
    Object.fromEntries(Object.entries(props).map(([prop, ref]) => [prop, ctx.get(CTX.exec(ref)![1]!) ?? null]));

  const setContext = (name: string, value: string | null): void => {
    ctx.set(name, value);
    writes.push([name, value]);
    for (const entry of entries) {
      if (entry.provider || entry.apply === undefined) continue;
      if (Object.values(entry.props).some((ref) => CTX.exec(ref)![1] === name)) entry.apply(resolve(entry.props));
    }
  };

  let campagnes: CampagnesController | undefined;
  let pjs: PjsController | undefined;
  let niveau: MountedNiveauDuGroupe | undefined;

  for (const item of screen.layout) {
    const [microUi, props] = Object.entries(item)[0]!;
    switch (microUi) {
      case 'Campagnes': {
        const controller = createCampagnesController({ shell, setContext, confirm: () => true });
        campagnes = controller;
        entries.push({ provider: true, props, dispose: () => controller.dispose() });
        break;
      }
      case 'Pjs': {
        const controller = createPjsController(resolve(props)['campagneId'] ?? null, { shell });
        pjs = controller;
        entries.push({
          provider: false,
          props,
          apply: (next) => controller.setCampagne(next['campagneId'] ?? null),
          dispose: () => controller.dispose(),
        });
        break;
      }
      case 'NiveauDuGroupe': {
        const mounted = mountNiveau(niveauElement as unknown as HTMLElement, { campagneId: resolve(props)['campagneId'] ?? undefined }, { shell });
        niveau = mounted;
        entries.push({
          provider: false,
          props,
          apply: (next) => mounted.update({ campagneId: next['campagneId'] ?? undefined }),
          dispose: () => mounted.unmount(),
        });
        break;
      }
      default:
        throw new Error(`the screen lays out ${microUi}, which this host cannot mount`);
    }
  }
  if (campagnes === undefined || pjs === undefined || niveau === undefined) throw new Error('the screen lacks a Micro-UI');
  return {
    ctx,
    writes,
    campagnes,
    pjs,
    niveau,
    niveauElement,
    dispose: () => {
      for (const entry of entries) entry.dispose();
    },
  };
}

// ---------------------------------------------------------------------------------------------
// The page.
// ---------------------------------------------------------------------------------------------

async function openPage() {
  const server = new FakeServer();
  const calls: Call[] = [];
  const clock = new FakeClock();
  const sources: FakeEventSource[] = [];
  const fakeFetch: typeof globalThis.fetch = (input, init) => {
    const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url);
    const body = typeof init?.body === 'string' ? init.body : undefined;
    calls.push({ method: init?.method ?? 'GET', path: url.pathname, body });
    const variables = body === undefined ? {} : ((JSON.parse(body) as { variables?: Record<string, unknown> }).variables ?? {});
    return Promise.resolve(server.answer(url.pathname, variables));
  };
  const shell = createShellClient({
    baseUrl: 'http://127.0.0.1:7878',
    fetch: fakeFetch,
    eventSource: () => {
      const source = new FakeEventSource();
      sources.push(source);
      return source;
    },
    timers: clock,
  });
  const { screen } = readScreen();
  const page = mountScreen(screen, shell);
  await clock.tick();

  const reads = (capability: string): Call[] => calls.filter((c) => c.method === 'POST' && c.path === `/capabilities/${capability}`);
  const slot = (name: string): string => page.niveauElement.children[0]!.children.find((c) => c.dataset['slot'] === name)!.textContent;
  const niveauState = (): string | undefined => page.niveauElement.children[0]!.dataset['state'];
  /** The server commits, announces the version, and the page hears of it: no reload. */
  const announce = async (change: () => void): Promise<void> => {
    const version = server.commit(change);
    sources[0]!.push(version);
    await clock.tick();
  };
  return { server, calls, clock, sources, page, reads, slot, niveauState, announce };
}

/** The variables a read was sent with: the shell wraps them as `{"variables": ...}`. */
const variablesOf = (call: Call | undefined): unknown =>
  call?.body === undefined ? undefined : (JSON.parse(call.body) as { variables: unknown }).variables;

describe('the screen as a page', () => {
  it('mounts the three Micro-UIs from the file, on one shell, and lists the campaigns', async () => {
    const p = await openPage();
    expect(p.sources).toHaveLength(1); // one shell, one dataVersion stream, for the three of them
    expect(p.reads('campagne.listerCampagnes')).toHaveLength(1);
    const list = p.page.campagnes.getState().list;
    expect(list.kind).toBe('ready');
    if (list.kind === 'ready') expect(list.rows.map((row) => row.name)).toEqual(['Les Mouettes', 'Les Brumes']);
  });

  it('with no campaign selected, the PC list and the party level show no data, no error, and read nothing', async () => {
    const p = await openPage();
    expect(p.page.ctx.get(CONTEXT_NAME) ?? null).toBeNull();
    expect(p.page.writes).toEqual([]);
    expect(p.reads('campagne.listerPjs')).toEqual([]);
    expect(p.reads('campagne.niveauDuGroupe')).toEqual([]);
    expect(p.page.pjs.getState().campagneId).toBeNull();
    expect(p.page.pjs.getState().list.kind).toBe('idle');
    // The party level shows neither a level nor a count, and does not claim a failed read.
    expect(['level', 'no-level', 'loading']).not.toContain(p.niveauState());
    expect(p.slot('count')).toBe('');
    expect(p.slot('level')).not.toBe('Niveau du groupe indisponible');
  });

  it('selecting a campaign shows its PCs and its party level, with one read each by identifier', async () => {
    const p = await openPage();
    p.page.campagnes.select(A);
    expect(p.page.ctx.get(CONTEXT_NAME)).toBe(A);
    await p.clock.tick();

    expect(p.reads('campagne.listerPjs').map(variablesOf)).toEqual([{ campagneId: A }]);
    expect(p.reads('campagne.niveauDuGroupe').map(variablesOf)).toEqual([{ campagneId: A }]);
    const list = p.page.pjs.getState().list;
    expect(list.kind).toBe('rows');
    if (list.kind === 'rows') expect(list.rows.map((row) => [row.name, row.level])).toEqual([['Ysolde', 3], ['Brannoc', 4]]);
    expect(p.niveauState()).toBe('level');
    expect([p.slot('level'), p.slot('count')]).toEqual(['4', '2 PJ']); // levels [3, 4]: 3.5 rounds up
  });

  it('selecting another campaign replaces both, and reads nothing more for the first', async () => {
    const p = await openPage();
    p.page.campagnes.select(A);
    await p.clock.tick();
    p.page.campagnes.select(B);
    await p.clock.tick();

    expect(p.page.ctx.get(CONTEXT_NAME)).toBe(B);
    expect(p.reads('campagne.listerPjs').map(variablesOf)).toEqual([{ campagneId: A }, { campagneId: B }]);
    expect(p.reads('campagne.niveauDuGroupe').map(variablesOf)).toEqual([{ campagneId: A }, { campagneId: B }]);
    const list = p.page.pjs.getState().list;
    if (list.kind !== 'rows') throw new Error(`expected rows, got ${list.kind}`);
    expect(list.rows.map((row) => row.name)).toEqual(['Cyrielle']);
    expect([p.slot('level'), p.slot('count')]).toEqual(['7', '1 PJ']);
  });

  it('a new dataVersion refreshes the open page by itself: no reload, no new mount', async () => {
    const p = await openPage();
    p.page.campagnes.select(A);
    await p.clock.tick();
    const { pjs, niveau, niveauElement } = p.page;

    // A PC is edited to level 1 and another is added: levels [1, 4, 2] give 2.
    await p.announce(() => {
      const pcs = p.server.pcs.get(A)!;
      pcs[0]!.level = 1;
      pcs.push({ id: 'pc-a3', name: 'Cyrielle', class: 'Magicienne', level: 2 });
    });
    const list = p.page.pjs.getState().list;
    if (list.kind !== 'rows') throw new Error(`expected rows, got ${list.kind}`);
    expect(list.rows.map((row) => [row.name, row.level])).toEqual([['Ysolde', 1], ['Brannoc', 4], ['Cyrielle', 2]]);
    expect([p.slot('level'), p.slot('count')]).toEqual(['2', '3 PJ']);
    expect(p.niveauState()).toBe('level');

    // The same host, the same Micro-UIs, the same element, the same stream.
    expect(p.page.pjs).toBe(pjs);
    expect(p.page.niveau).toBe(niveau);
    expect(p.page.niveauElement).toBe(niveauElement);
    expect(niveauElement.children).toHaveLength(1);
    expect(p.sources).toHaveLength(1);

    // The last PCs archived: the party is back to "no party level", never 0.
    await p.announce(() => {
      p.server.pcs.set(A, []);
    });
    expect(p.niveauState()).toBe('no-level');
    expect([p.slot('level'), p.slot('count')]).toEqual(['Pas de niveau de groupe', '0 PJ']);
    expect(p.page.pjs.getState().list.kind).toBe('rows');
  });

  it('when the selected campaign leaves the list, the context is cleared and both Micro-UIs stop reading', async () => {
    const p = await openPage();
    p.page.campagnes.select(B);
    await p.clock.tick();
    expect(p.niveauState()).toBe('level');

    // B is archived: it is no longer listed, and no PC list or party level answers for it.
    await p.announce(() => {
      p.server.campaigns = p.server.campaigns.filter((c) => c.id !== B);
    });
    expect(p.page.ctx.get(CONTEXT_NAME)).toBeNull();
    expect(p.page.writes.at(-1)).toEqual([CONTEXT_NAME, null]);
    expect(p.page.campagnes.getState().selected).toBeNull();
    expect(p.page.pjs.getState().campagneId).toBeNull();
    expect(p.page.pjs.getState().list.kind).toBe('idle');
    expect(['level', 'no-level', 'loading']).not.toContain(p.niveauState());
    expect(p.slot('count')).toBe('');

    // Another change: only the campaign list is read again.
    const pjsReads = p.reads('campagne.listerPjs').length;
    const niveauReads = p.reads('campagne.niveauDuGroupe').length;
    const listReads = p.reads('campagne.listerCampagnes').length;
    await p.announce(() => {
      p.server.campaigns = [...p.server.campaigns];
    });
    expect(p.reads('campagne.listerPjs')).toHaveLength(pjsReads);
    expect(p.reads('campagne.niveauDuGroupe')).toHaveLength(niveauReads);
    expect(p.reads('campagne.listerCampagnes')).toHaveLength(listReads + 1);
  });
});

// All the behaviour of the Campagnes Micro-UI, with no DOM. It shows what the
// DataGuard and the Data layer decided and decides nothing itself: no name
// rule, no sorting, no filtering, no verdict on a command (SPEC rules 6, 12, 25).

import {
  mapViolations,
  type MappedViolation,
  type PendingState,
  type ShellClient,
  type SubmitInput,
  type Submitted,
  type TerminalStatus,
  type WatchEvent,
} from '@dnd-helper/micro-ui-shell';
import { ARCHIVER_CAMPAGNE, CONTEXT_NAME, CREER_CAMPAGNE, LISTER_CAMPAGNES } from './identifiers';
import { VIOLATIONS } from './violations';

export interface Campagne {
  readonly id: string;
  readonly name: string;
}

/** What the controller needs of the shell. `act` is absent on purpose: nothing here confirms a command (rule 28). */
export type ShellPort = Pick<ShellClient, 'watch' | 'submit' | 'awaitResult'>;

export interface CampagnesDeps {
  shell: ShellPort;
  /** Host-provided (owner decision, #27). Called with `CONTEXT_NAME` only. */
  setContext: (name: string, value: string | null) => void;
  /** One explicit GM confirmation before an archive. */
  confirm: (message: string) => boolean | Promise<boolean>;
  newIdempotencyKey?: () => string;
}

export type ListView =
  | { kind: 'loading' }
  /** No good list yet: a read failure, never an empty list (rule 8). */
  | { kind: 'failed'; message: string }
  /** `rows` may be empty (rule 7); a later failure keeps the rows (rule 34). */
  | { kind: 'ready'; rows: readonly Campagne[]; asOf: number | null; failure: string | null; stale: boolean };

export type CommandPhase =
  | { kind: 'idle' }
  | { kind: 'sending' }
  | { kind: 'pending'; state: PendingState; commandId: string }
  | {
      kind: 'settled';
      status: TerminalStatus;
      dataVersion: number | null;
      /** One entry per violation id of a rejection, in server order: the id as returned, and its readable message. */
      violations: readonly MappedViolation[];
    }
  /** The command may or may not have reached the server: the key is kept, so a retry replays it. */
  | { kind: 'send-failed'; message: string }
  | { kind: 'lost'; commandId: string };

export interface CreateView {
  /** Raw, never trimmed. */
  name: string;
  phase: CommandPhase;
  fieldErrors: { name: readonly string[] };
  formErrors: readonly string[];
}

export interface CampagnesState {
  list: ListView;
  create: CreateView;
  /** Keyed by campaign id. */
  archive: Readonly<Record<string, CommandPhase>>;
  selected: string | null;
}

export interface CampagnesController {
  getState(): CampagnesState;
  subscribe(listener: (state: CampagnesState) => void): () => void;
  setName(name: string): void;
  submitCreate(): Promise<void>;
  requestArchive(id: string): Promise<void>;
  select(id: string): void;
  dispose(): void;
}

const READ_FAILED = 'La lecture des campagnes a échoué.';
const SEND_FAILED = 'L’envoi a échoué. Réessayer reprend la même commande, sans doublon.';

/** What the GM reads for a violation id the map does not know, or for a refusal that carries none. */
const genericViolation = (id: string | null): string => (id === null ? 'La commande a été refusée.' : `Refus non décrit : ${id}`);

const busy = (phase: CommandPhase | undefined): boolean => phase?.kind === 'sending' || phase?.kind === 'pending';

type Settled = Extract<CommandPhase, { kind: 'settled' }>;

export function createCampagnesController(deps: CampagnesDeps): CampagnesController {
  const { shell, setContext } = deps;
  const newKey = deps.newIdempotencyKey ?? ((): string => globalThis.crypto.randomUUID());
  const abort = new AbortController();
  const listeners = new Set<(state: CampagnesState) => void>();
  let disposed = false;

  let state: CampagnesState = {
    list: { kind: 'loading' },
    create: { name: '', phase: { kind: 'idle' }, fieldErrors: { name: [] }, formErrors: [] },
    archive: {},
    selected: null,
  };

  /** One key per create attempt: reused by a retry of the same name, dropped on a terminal result. */
  let createAttempt: { key: string; name: string } | null = null;
  /** One key per row being archived. */
  const archiveKeys = new Map<string, string>();
  /** Rows whose confirmation dialog is open: a second click must not open a second one. */
  const confirming = new Set<string>();

  function update(next: CampagnesState): void {
    if (disposed) return;
    state = next;
    for (const listener of [...listeners]) listener(state);
  }

  const patchCreate = (patch: Partial<CreateView>): void => {
    update({ ...state, create: { ...state.create, ...patch } });
  };

  const patchArchive = (id: string, phase: CommandPhase): void => {
    update({ ...state, archive: { ...state.archive, [id]: phase } });
  };

  // --- list ---------------------------------------------------------------

  function emitContext(value: string | null): void {
    if (!disposed) setContext(CONTEXT_NAME, value);
  }

  function onListEvent(event: WatchEvent<readonly Campagne[]>): void {
    if (disposed) return;
    const list = state.list;
    switch (event.kind) {
      case 'data': {
        // The shown version never goes backwards (rule 32). An equal one is the
        // same data again: it is taken, so it can clear a failure.
        if (list.kind === 'ready' && list.asOf !== null && event.asOf !== null && event.asOf < list.asOf) return;
        const rows = event.data;
        const asOf = event.asOf ?? (list.kind === 'ready' ? list.asOf : null);
        const present = new Set(rows.map((row) => row.id));
        // A terminal archive status has no row left to sit on.
        const archive = Object.fromEntries(
          Object.entries(state.archive).filter(([id, phase]) => present.has(id) || busy(phase)),
        );
        for (const id of [...archiveKeys.keys()]) if (!present.has(id) && !busy(state.archive[id])) archiveKeys.delete(id);
        const vanished = state.selected !== null && !present.has(state.selected);
        update({
          ...state,
          list: { kind: 'ready', rows, asOf, failure: null, stale: false },
          archive,
          selected: vanished ? null : state.selected,
        });
        // A stale id is never emitted (rule 39).
        if (vanished) emitContext(null);
        return;
      }
      case 'stale':
        if (list.kind === 'ready') update({ ...state, list: { ...list, stale: true } });
        else update({ ...state, list: { kind: 'failed', message: READ_FAILED } });
        return;
      case 'error':
      case 'not-found':
        // A list read has no not-found case: it is a failure, not an empty list (rule 8).
        if (list.kind === 'ready') update({ ...state, list: { ...list, failure: READ_FAILED } });
        else update({ ...state, list: { kind: 'failed', message: READ_FAILED } });
        return;
    }
  }

  const unwatch = shell.watch<readonly Campagne[]>(LISTER_CAMPAGNES, {}, onListEvent);

  // --- commands -----------------------------------------------------------

  /** Follows a submitted command to its terminal state. Returns null when it was lost or the controller disposed. */
  async function follow(first: Submitted['first'], setPhase: (phase: CommandPhase) => void): Promise<Settled | null> {
    // A replay of a command that already finished comes back settled.
    if (first.kind === 'settled') return { kind: 'settled', status: first.result.status, dataVersion: first.result.dataVersion, violations: mapViolations(first.result, VIOLATIONS, genericViolation) };
    let commandId = first.commandId;
    setPhase({ kind: 'pending', state: first.state, commandId });

    for (;;) {
      const outcome = await shell.awaitResult(commandId, {
        signal: abort.signal,
        violationMessages: VIOLATIONS,
        onState: (view) => {
          if (view.kind === 'pending') setPhase({ kind: 'pending', state: view.state, commandId: view.commandId });
        },
      });
      if (disposed) return null;
      switch (outcome.kind) {
        case 'settled':
          return { kind: 'settled', status: outcome.result.status, dataVersion: outcome.result.dataVersion, violations: mapViolations(outcome.result, VIOLATIONS, genericViolation) };
        case 'not-found':
          setPhase({ kind: 'lost', commandId: outcome.commandId });
          return null;
        case 'still-pending':
          // A timeout is not an outcome: keep waiting. An abort means dispose.
          if (outcome.reason === 'aborted') return null;
          commandId = outcome.commandId;
          break;
      }
    }
  }

  // --- create -------------------------------------------------------------

  function setName(name: string): void {
    if (disposed) return;
    const create = state.create;
    if (name === create.name) return;
    if (createAttempt !== null && createAttempt.name !== name) createAttempt = null;
    // Errors and a finished status described the previous text; a pending command is left alone.
    const finished = !busy(create.phase);
    patchCreate({
      name,
      ...(finished ? { phase: { kind: 'idle' }, fieldErrors: { name: [] }, formErrors: [] } : {}),
    });
  }

  async function submitCreate(): Promise<void> {
    if (disposed || busy(state.create.phase)) return;
    const name = state.create.name;
    if (createAttempt === null || createAttempt.name !== name) createAttempt = { key: newKey(), name };
    const attempt = createAttempt;
    patchCreate({ phase: { kind: 'sending' }, fieldErrors: { name: [] }, formErrors: [] });

    const input: SubmitInput = {
      dataCapability: CREER_CAMPAGNE.id,
      version: CREER_CAMPAGNE.version,
      mode: CREER_CAMPAGNE.mode,
      target: { aggregate: CREER_CAMPAGNE.aggregate },
      payload: { name },
      idempotencyKey: attempt.key,
      signal: abort.signal,
    };

    let settled: Settled | null;
    try {
      const sent = await shell.submit(input);
      settled = await follow(sent.first, (phase) => {
        patchCreate({ phase });
      });
    } catch {
      if (disposed) return;
      patchCreate({ phase: { kind: 'send-failed', message: SEND_FAILED } });
      return;
    }
    if (settled === null) return;

    // A terminal result ends the attempt: the next submit is a new one (rule 17).
    createAttempt = null;
    if (settled.status === 'applied') {
      // The GM may have typed on while the command ran: only the submitted text is cleared (rule 19).
      patchCreate({ phase: settled, ...(state.create.name === name ? { name: '' } : {}) });
      return;
    }
    patchCreate({ phase: settled, ...splitViolations(settled) });
  }

  // --- archive ------------------------------------------------------------

  async function requestArchive(id: string): Promise<void> {
    if (disposed || busy(state.archive[id]) || confirming.has(id)) return;
    const row = state.list.kind === 'ready' ? state.list.rows.find((r) => r.id === id) : undefined;
    const subject = row === undefined ? 'cette campagne' : `la campagne « ${row.name} »`;

    confirming.add(id);
    let confirmed: boolean;
    try {
      confirmed = await deps.confirm(`Archiver ${subject} et ses personnages joueurs ? Il n’existe pas de restauration.`);
    } finally {
      confirming.delete(id);
    }
    if (!confirmed || disposed) return;
    // The row may have left the list while the dialog was open: nothing is left to archive.
    if (row !== undefined && state.list.kind === 'ready' && !state.list.rows.some((r) => r.id === id)) return;

    let key = archiveKeys.get(id);
    if (key === undefined) {
      key = newKey();
      archiveKeys.set(id, key);
    }
    patchArchive(id, { kind: 'sending' });

    const input: SubmitInput = {
      dataCapability: ARCHIVER_CAMPAGNE.id,
      version: ARCHIVER_CAMPAGNE.version,
      mode: ARCHIVER_CAMPAGNE.mode,
      target: { aggregate: ARCHIVER_CAMPAGNE.aggregate, id },
      payload: {},
      idempotencyKey: key,
      signal: abort.signal,
    };

    let settled: Settled | null;
    try {
      const sent = await shell.submit(input);
      settled = await follow(sent.first, (phase) => {
        patchArchive(id, phase);
      });
    } catch {
      if (disposed) return;
      patchArchive(id, { kind: 'send-failed', message: SEND_FAILED });
      return;
    }
    if (settled === null) return;
    archiveKeys.delete(id);
    // The row leaves through the next list read, never locally (rule 30).
    patchArchive(id, settled);
  }

  // --- selection ----------------------------------------------------------

  function select(id: string): void {
    if (disposed || id === state.selected) return;
    // Only an id the list returned can be emitted (rule 36).
    if (state.list.kind !== 'ready' || !state.list.rows.some((row) => row.id === id)) return;
    update({ ...state, selected: id });
    emitContext(id);
  }

  function dispose(): void {
    if (disposed) return;
    abort.abort();
    unwatch();
    disposed = true;
    listeners.clear();
  }

  return {
    getState: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    setName,
    submitCreate,
    requestArchive,
    select,
    dispose,
  };
}

/** `name`-field violations go to the field, every other one to the form, none swallowed (rule 14). */
function splitViolations(settled: Settled): Pick<CreateView, 'fieldErrors' | 'formErrors'> {
  const onName = settled.violations.filter((v) => v.field === 'name').map((v) => v.message);
  const onForm = settled.violations.filter((v) => v.field !== 'name').map((v) => v.message);
  return { fieldErrors: { name: onName }, formErrors: onForm };
}

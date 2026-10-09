// All the behaviour of the Micro-UI, over the page's one ShellClient and no
// DOM. It holds no rule: a value goes out as typed, a refusal comes back from
// the DataGuard, and the list is whatever `lister-pjs` returns, in its order.

import {
  ActionNotOfferedError,
  ClientDisposedError,
  mapViolations,
  type AwaitOutcome,
  ProtocolError,
  TransportError,
  type CommandResult,
  type CommandView,
  type JsonValue,
  type MappedViolation,
  type ShellClient,
  type SubmitInput,
  type TerminalStatus,
  type Unsubscribe,
  type WatchEvent,
} from '@dnd-helper/micro-ui-shell';
import { COPY } from './copy';
import { AJOUTER_PJ, ARCHIVER_PJ, LISTER_PJS, MODIFIER_PJ, type PcRow } from './identifiers';
import { fieldOfMessage, fieldOfViolation, pcPayload, PJ_VIOLATIONS, type PcField, type PcFormValues } from './violations';

export interface PjsDeps {
  shell: ShellClient;
  /** One idempotency key per form attempt. Defaults to `crypto.randomUUID()`. */
  newKey?: () => string;
  /** How long to wait before asking again after the connection dropped mid-wait. Defaults to 5 s. */
  retryMs?: number;
}

export interface Violation {
  /** The id as received; `null` for a rejection that carried none. */
  id: string | null;
  text: string;
}

export type ListState =
  | { kind: 'idle' }
  | { kind: 'loading' }
  | { kind: 'not-found' }
  | { kind: 'rows'; rows: PcRow[]; asOf: number | null; stale: boolean; error: boolean }
  | { kind: 'error' };

export type OfferedAction = 'confirm_overwrite' | 'cancel';

export type CommandUi =
  | { kind: 'idle' }
  | { kind: 'sending' }
  | {
      kind: 'awaiting';
      commandId: string;
      yourValue: JsonValue;
      ahead: JsonValue | null;
      aheadField: PcField | null;
      actions: OfferedAction[];
    }
  | { kind: 'done'; status: TerminalStatus };

export interface FormState {
  values: PcFormValues;
  fieldErrors: Partial<Record<PcField, Violation[]>>;
  formErrors: Violation[];
  command: CommandUi;
  transportError: boolean;
}

export type EditState = FormState & { pcId: string; basedOn: number };

export type RowNotice =
  | { kind: 'rejected'; violations: Violation[] }
  | { kind: 'transport' }
  | { kind: 'expired' }
  | { kind: 'cancelled' };

export interface PjsState {
  campagneId: string | null;
  list: ListState;
  add: FormState;
  edit: EditState | null;
  archiving: ReadonlySet<string>;
  rowNotices: Readonly<Record<string, RowNotice>>;
}

export interface PjsController {
  getState(): PjsState;
  subscribe(listener: (state: PjsState) => void): Unsubscribe;
  setCampagne(campagneId: string | null): void;
  setAddField(field: PcField, value: string): void;
  submitAdd(): Promise<void>;
  openEdit(pcId: string): void;
  setEditField(field: PcField, value: string): void;
  submitEdit(): Promise<void>;
  closeEdit(): void;
  confirmEdit(): Promise<void>;
  cancelEdit(): Promise<void>;
  archive(pcId: string): Promise<void>;
  dispose(): void;
}

type Which = 'add' | 'edit';
type Pending = Extract<CommandView, { kind: 'pending' }>;

interface Hooks {
  onId(commandId: string): void;
  onPending(commandId: string, view: Pending): void;
  onSettled(commandId: string, result: CommandResult, violations: MappedViolation[]): void;
  /** `commandId` is null when the send itself failed, before the command had an id. */
  onTransport(commandId: string | null): void;
  /** The wait lost its connection (false) or got it back (true) while the command is alive in the queue. */
  onLink(ok: boolean): void;
}

const emptyValues = (): PcFormValues => ({ nom: '', classe: '', niveau: '' });

const emptyForm = (): FormState => ({
  values: emptyValues(),
  fieldErrors: {},
  formErrors: [],
  command: { kind: 'idle' },
  transportError: false,
});

export const isBusy = (f: FormState): boolean => f.command.kind === 'sending' || f.command.kind === 'awaiting';

const isOffered = (a: string): a is OfferedAction => a === 'confirm_overwrite' || a === 'cancel';

function isRecord(v: JsonValue | undefined): v is { [key: string]: JsonValue } {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

/** The value declared ahead of an edit: the message that says so, else the entry's projection. */
function declaredAhead(view: Pending, commandId: string): { value: JsonValue; field: PcField | null } | null {
  for (let i = view.messages.length - 1; i >= 0; i -= 1) {
    const m = view.messages[i];
    if (m === undefined) continue;
    // Messages reach the UI per author; one about another command is not ours (rule 46).
    if (m.command !== undefined && m.command !== commandId) continue;
    if (m.message === 'ValueDeclaredAhead' && m.declaredAhead !== undefined) {
      return { value: m.declaredAhead.value, field: fieldOfMessage(m.field) };
    }
  }
  const ahead = view.entry.projection?.['pendingAhead'];
  if (Array.isArray(ahead)) {
    const first = ahead[0];
    if (isRecord(first) && first['value'] !== undefined) return { value: first['value'], field: null };
  }
  return null;
}

const CLOSED_MAX = 200;

function pause(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal.aborted) {
      resolve();
      return;
    }
    const done = (): void => {
      clearTimeout(timer);
      signal.removeEventListener('abort', done);
      resolve();
    };
    const timer = setTimeout(done, ms);
    signal.addEventListener('abort', done, { once: true });
  });
}

export function createPjsController(campagneId: string | null, deps: PjsDeps): PjsController {
  const { shell } = deps;
  const newKey = deps.newKey ?? ((): string => crypto.randomUUID());
  const retryMs = deps.retryMs ?? 5000;

  let state: PjsState = freshState(null);
  const listeners = new Set<(s: PjsState) => void>();
  let unwatch: Unsubscribe | null = null;
  let disposed = false;
  /** Bumped when the page moves to another campaign or goes away: older flows stop touching the state. */
  let epoch = 0;
  const aborts = new Set<AbortController>();
  const inflight = new Map<string, AbortController>();
  /** Commands that reached a terminal state here: the first one is final (rule 43). */
  const closed = new Set<string>();
  const keys: Record<Which, string | null> = { add: null, edit: null };
  const current: Record<Which, string | null> = { add: null, edit: null };
  const archiveKeys = new Map<string, string>();

  function freshState(id: string | null): PjsState {
    return { campagneId: id, list: { kind: 'idle' }, add: emptyForm(), edit: null, archiving: new Set(), rowNotices: {} };
  }

  function set(next: PjsState): void {
    state = next;
    for (const l of [...listeners]) l(state);
  }

  function patchForm(which: Which, patch: Partial<FormState>): void {
    if (which === 'add') set({ ...state, add: { ...state.add, ...patch } });
    else if (state.edit !== null) set({ ...state, edit: { ...state.edit, ...patch } });
  }

  const formOf = (which: Which): FormState | null => (which === 'add' ? state.add : state.edit);

  /** Forms and row actions exist for a campaign that is not unknown to the Data layer (rule 14). */
  const actionable = (): boolean =>
    state.campagneId !== null && state.list.kind !== 'not-found' && state.list.kind !== 'idle';

  // --- list ---------------------------------------------------------------

  function applyListEvent(e: WatchEvent<PcRow[]>): void {
    const prev = state.list;
    switch (e.kind) {
      case 'data':
        if (!Array.isArray(e.data)) {
          set({ ...state, list: { kind: 'error' } });
          return;
        }
        set({ ...state, list: { kind: 'rows', rows: e.data, asOf: e.asOf, stale: false, error: false } });
        return;
      case 'not-found':
        set({ ...state, list: { kind: 'not-found' } });
        return;
      case 'stale':
        if (prev.kind === 'rows') set({ ...state, list: { ...prev, stale: true } });
        return;
      case 'error':
        set({ ...state, list: prev.kind === 'rows' ? { ...prev, error: true } : { kind: 'error' } });
        return;
    }
  }

  function startWatch(id: string): void {
    const mine = epoch;
    set({ ...state, list: { kind: 'loading' } });
    unwatch = shell.watch<PcRow[]>(LISTER_PJS, { campagneId: id }, (e) => {
      if (mine === epoch) applyListEvent(e);
    });
  }

  function stopAll(): void {
    epoch += 1;
    unwatch?.();
    unwatch = null;
    for (const ac of aborts) ac.abort();
    aborts.clear();
    inflight.clear();
    closed.clear();
  }

  function setCampagne(id: string | null): void {
    if (disposed || id === state.campagneId) return;
    stopAll();
    keys.add = null;
    keys.edit = null;
    current.add = null;
    current.edit = null;
    archiveKeys.clear();
    set(freshState(id));
    if (id !== null) startWatch(id);
  }

  // --- commands -----------------------------------------------------------

  function remember(commandId: string): void {
    closed.add(commandId);
    if (closed.size > CLOSED_MAX) {
      const oldest = closed.values().next().value;
      if (oldest !== undefined) closed.delete(oldest);
    }
  }

  /** A command applied: the refusals shown so far describe a state that no longer holds. */
  function clearMarks(): void {
    set({
      ...state,
      add: { ...state.add, fieldErrors: {}, formErrors: [] },
      edit: state.edit === null ? null : { ...state.edit, fieldErrors: {}, formErrors: [] },
      rowNotices: {},
    });
  }

  function settle(commandId: string, result: CommandResult, violations: MappedViolation[], hooks: Hooks): void {
    if (closed.has(commandId)) return;
    remember(commandId);
    const ac = inflight.get(commandId);
    inflight.delete(commandId);
    if (result.status === 'applied') clearMarks();
    hooks.onSettled(commandId, result, violations);
    // Stop the wait loop; it has nothing left to observe.
    ac?.abort();
  }

  function handleView(commandId: string, view: CommandView, hooks: Hooks): void {
    if (view.kind === 'settled') settle(commandId, view.result, mapViolations(view.result, PJ_VIOLATIONS), hooks);
    else if (view.kind === 'pending') hooks.onPending(commandId, view);
  }

  /**
   * Submit, then wait for the terminal state. A wait that times out is renewed
   * for as long as the page lives: a parked edit can last 24 hours. A network
   * failure while the command is alive in the queue is waited out, never turned
   * into a second command; a failure of the send itself leaves the form
   * retryable with the same key.
   */
  async function runCommand(input: Omit<SubmitInput, 'signal'>, hooks: Hooks): Promise<void> {
    const mine = epoch;
    const ac = new AbortController();
    aborts.add(ac);
    let commandId: string | null = null;
    try {
      const sent = await shell.submit({ ...input, signal: ac.signal });
      if (mine !== epoch) return;
      const id = sent.commandId;
      commandId = id;
      inflight.set(id, ac);
      hooks.onId(id);
      handleView(id, sent.first, hooks);
      let linkLost = false;
      for (;;) {
        if (mine !== epoch || closed.has(id) || ac.signal.aborted) return;
        let out: AwaitOutcome;
        try {
          out = await shell.awaitResult(id, {
            signal: ac.signal,
            violationMessages: PJ_VIOLATIONS,
            onState: (view) => {
              if (mine === epoch) handleView(id, view, hooks);
            },
          });
        } catch (e) {
          if (mine !== epoch || ac.signal.aborted) return;
          if (!(e instanceof TransportError || e instanceof ProtocolError)) throw e;
          if (!linkLost) {
            linkLost = true;
            hooks.onLink(false);
          }
          await pause(retryMs, ac.signal);
          continue;
        }
        if (mine !== epoch) return;
        if (linkLost) {
          linkLost = false;
          hooks.onLink(true);
        }
        if (out.kind === 'settled') {
          settle(id, out.result, out.violationMessages, hooks);
          return;
        }
        if (out.kind === 'not-found') {
          hooks.onTransport(id);
          return;
        }
        if (out.reason === 'aborted') return;
      }
    } catch (e) {
      if (mine !== epoch) return;
      if (e instanceof TransportError || e instanceof ProtocolError) {
        hooks.onTransport(commandId);
        return;
      }
      if (e instanceof ClientDisposedError) return;
      throw e;
    } finally {
      aborts.delete(ac);
      if (commandId !== null && inflight.get(commandId) === ac) inflight.delete(commandId);
    }
  }

  function violationNotes(violations: MappedViolation[]): Pick<FormState, 'fieldErrors' | 'formErrors'> & { all: Violation[] } {
    const fieldErrors: Partial<Record<PcField, Violation[]>> = {};
    const formErrors: Violation[] = [];
    const all: Violation[] = [];
    for (const v of violations) {
      const field = fieldOfViolation(v.field);
      const note: Violation = field === null ? { id: v.id, text: COPY.refused(v.id) } : { id: v.id, text: v.message };
      all.push(note);
      if (field === null) formErrors.push(note);
      else (fieldErrors[field] ??= []).push(note);
    }
    return { fieldErrors, formErrors, all };
  }

  function formHooks(which: Which): Hooks {
    return {
      onId: (commandId) => {
        current[which] = commandId;
      },
      onPending: (commandId, view) => {
        // A view of a command this form did not send is not ours (rule 46).
        if (current[which] !== commandId) return;
        const f = formOf(which);
        if (f === null || f.command.kind === 'done') return;
        if (which === 'edit' && view.state === 'awaiting_confirmation') {
          const ahead = declaredAhead(view, commandId);
          patchForm(which, {
            command: {
              kind: 'awaiting',
              commandId,
              // Never ask the GM to overwrite blind: fall back to what they typed.
              yourValue: view.yourValue ?? view.entry.yourValue ?? { name: f.values.nom, class: f.values.classe, level: f.values.niveau },
              ahead: ahead?.value ?? null,
              aheadField: ahead?.field ?? null,
              actions: view.actions.filter(isOffered),
            },
          });
        } else {
          patchForm(which, { command: { kind: 'sending' } });
        }
      },
      onSettled: (commandId, result, violations) => {
        // The GM left this command (closed the form, moved on): it is not this form's any more.
        if (current[which] !== commandId) return;
        keys[which] = null;
        current[which] = null;
        if (result.status === 'applied') {
          if (which === 'add') {
            set({ ...state, add: { ...emptyForm(), command: { kind: 'done', status: 'applied' } } });
          } else {
            set({ ...state, edit: null });
          }
          return;
        }
        const f = formOf(which);
        const { fieldErrors, formErrors } =
          result.status === 'rejected' ? violationNotes(violations) : { fieldErrors: f?.fieldErrors ?? {}, formErrors: f?.formErrors ?? [] };
        patchForm(which, { fieldErrors, formErrors, command: { kind: 'done', status: result.status }, transportError: false });
      },
      onTransport: (commandId) => {
        if (commandId !== null && current[which] !== commandId) return;
        patchForm(which, { command: { kind: 'idle' }, transportError: true });
      },
      onLink: (ok) => {
        patchForm(which, { transportError: !ok });
      },
    };
  }

  function setField(which: Which, field: PcField, value: string): void {
    const f = formOf(which);
    if (disposed || f === null || isBusy(f)) return;
    const fieldErrors = { ...f.fieldErrors };
    delete fieldErrors[field];
    // A new submit after an edit of the form is a new attempt: a new key.
    keys[which] = null;
    patchForm(which, {
      values: { ...f.values, [field]: value },
      fieldErrors,
      command: f.command.kind === 'done' ? { kind: 'idle' } : f.command,
    });
  }

  async function submitAdd(): Promise<void> {
    const id = state.campagneId;
    if (disposed || id === null || !actionable() || isBusy(state.add)) return;
    const key = (keys.add ??= newKey());
    const values = state.add.values;
    patchForm('add', { command: { kind: 'sending' }, transportError: false });
    await runCommand(
      {
        dataCapability: AJOUTER_PJ.dataCapability,
        version: AJOUTER_PJ.version,
        mode: AJOUTER_PJ.mode,
        payload: { campagneId: id, ...pcPayload(values) },
        idempotencyKey: key,
      },
      formHooks('add'),
    );
  }

  function openEdit(pcId: string): void {
    if (disposed || !actionable() || (state.edit !== null && isBusy(state.edit))) return;
    const list = state.list;
    if (list.kind !== 'rows') return;
    const row = list.rows.find((r) => r.id === pcId);
    if (row === undefined) return;
    // `basedOn` is the dataVersion the PC was read at; it is not moved by later refreshes.
    const basedOn = list.asOf ?? shell.knownDataVersion() ?? 0;
    keys.edit = null;
    current.edit = null;
    set({
      ...state,
      edit: { ...emptyForm(), values: { nom: row.name, classe: row.class, niveau: String(row.level) }, pcId, basedOn },
    });
  }

  function closeEdit(): void {
    const e = state.edit;
    if (disposed || e === null || e.command.kind === 'sending') return;
    // Leaving a parked edit is the GM's choice: it stays in the queue and lapses by itself.
    const id = current.edit;
    if (id !== null) inflight.get(id)?.abort();
    keys.edit = null;
    current.edit = null;
    set({ ...state, edit: null });
  }

  async function submitEdit(): Promise<void> {
    const e = state.edit;
    if (disposed || e === null || !actionable() || isBusy(e)) return;
    const key = (keys.edit ??= newKey());
    patchForm('edit', { command: { kind: 'sending' }, transportError: false });
    await runCommand(
      {
        dataCapability: MODIFIER_PJ.dataCapability,
        version: MODIFIER_PJ.version,
        mode: MODIFIER_PJ.mode,
        target: { aggregate: MODIFIER_PJ.aggregate, id: e.pcId },
        payload: pcPayload(e.values),
        basedOn: { version: e.basedOn },
        idempotencyKey: key,
      },
      formHooks('edit'),
    );
  }

  /** Confirm or cancel the parked edit: only on the author's gesture, and only while offered. */
  async function actOnEdit(action: OfferedAction): Promise<void> {
    const e = state.edit;
    const commandId = current.edit;
    if (disposed || e === null || commandId === null || e.command.kind !== 'awaiting') return;
    if (!e.command.actions.includes(action)) return;
    const mine = epoch;
    const hooks = formHooks('edit');
    try {
      handleView(commandId, await shell.act(commandId, action), hooks);
    } catch (err) {
      if (mine !== epoch) return;
      if (err instanceof ActionNotOfferedError) {
        // The command moved on (expired, cancelled, applied): show what the queue says, never a success.
        try {
          handleView(commandId, await shell.lookup(commandId), hooks);
        } catch (lookupErr) {
          if (mine !== epoch) return;
          if (lookupErr instanceof TransportError || lookupErr instanceof ProtocolError) {
            patchForm('edit', { transportError: true });
            return;
          }
          throw lookupErr;
        }
        return;
      }
      if (err instanceof TransportError || err instanceof ProtocolError) {
        patchForm('edit', { transportError: true });
        return;
      }
      if (err instanceof ClientDisposedError) return;
      throw err;
    }
  }

  async function archive(pcId: string): Promise<void> {
    if (disposed || !actionable() || state.archiving.has(pcId)) return;
    const key = archiveKeys.get(pcId) ?? newKey();
    archiveKeys.set(pcId, key);
    const notices = { ...state.rowNotices };
    delete notices[pcId];
    set({ ...state, archiving: new Set([...state.archiving, pcId]), rowNotices: notices });

    const stop = (notice: RowNotice | null): void => {
      const archiving = new Set(state.archiving);
      archiving.delete(pcId);
      set({ ...state, archiving, rowNotices: notice === null ? state.rowNotices : { ...state.rowNotices, [pcId]: notice } });
    };
    await runCommand(
      {
        dataCapability: ARCHIVER_PJ.dataCapability,
        version: ARCHIVER_PJ.version,
        mode: ARCHIVER_PJ.mode,
        target: { aggregate: ARCHIVER_PJ.aggregate, id: pcId },
        payload: {},
        idempotencyKey: key,
      },
      {
        onId: () => undefined,
        onPending: () => undefined,
        onSettled: (_commandId, result, violations) => {
          archiveKeys.delete(pcId);
          // An applied archive, a no-op on an already archived PC included, is a success.
          if (result.status === 'applied') stop(null);
          else if (result.status === 'rejected') stop({ kind: 'rejected', violations: violationNotes(violations).all });
          else stop({ kind: result.status });
        },
        onTransport: () => {
          stop({ kind: 'transport' });
        },
        onLink: () => undefined,
      },
    );
  }

  function dispose(): void {
    if (disposed) return;
    disposed = true;
    stopAll();
    listeners.clear();
  }

  setCampagne(campagneId);

  return {
    getState: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    setCampagne,
    setAddField: (field, value) => {
      setField('add', field, value);
    },
    submitAdd,
    openEdit,
    setEditField: (field, value) => {
      setField('edit', field, value);
    },
    submitEdit,
    closeEdit,
    confirmEdit: () => actOnEdit('confirm_overwrite'),
    cancelEdit: () => actOnEdit('cancel'),
    archive,
    dispose,
  };
}

import { ActionNotOfferedError, TransportError, type AwaitOutcome, type Message } from '@dnd-helper/micro-ui-shell';
import { describe, expect, it } from 'vitest';
import { createPjsController } from '../src/controller';
import { viewOf } from '../src/view-model';
import {
  FakeShell,
  deferred,
  flush,
  pendingView,
  queued,
  row,
  settledOutcome,
  settledView,
  submitted,
} from './fake-shell';

const CAMP = 'camp-1';

function setup(campagneId: string | null = CAMP) {
  const shell = new FakeShell();
  let n = 0;
  const ctrl = createPjsController(campagneId, { shell, newKey: () => `key-${++n}` });
  return { shell, ctrl, st: () => ctrl.getState() };
}

function loaded(rows = [row('p1', 'Aria', 'Mage', 4), row('p2', 'Borek', 'Clerc', 3)], asOf: number | null = 7) {
  const s = setup();
  s.shell.emit({ kind: 'data', data: rows, asOf });
  return s;
}

function fillAdd(ctrl: ReturnType<typeof setup>['ctrl'], nom = 'Cyrielle', classe = 'Rôdeuse', niveau = '5') {
  ctrl.setAddField('nom', nom);
  ctrl.setAddField('classe', classe);
  ctrl.setAddField('niveau', niveau);
}

const rejectAt = (shell: FakeShell, id: string, ...violations: string[]): void => {
  shell.script(id, () => Promise.resolve(settledOutcome(id, 'rejected', violations)));
};

describe('list', () => {
  it('watches lister-pjs for the campagneId prop and shows the rows in the order received', () => {
    const { shell, st } = setup();
    expect(shell.watches).toHaveLength(1);
    expect(shell.watches[0]).toMatchObject({ capability: 'campagne.listerPjs', variables: { campagneId: CAMP } });
    expect(st().list.kind).toBe('loading');
    // Deliberately not alphabetical, not by level: the UI neither sorts nor filters.
    const rows = [row('p9', 'Zed', 'Mage', 2), row('p1', 'Aria', 'Mage', 9), row('p5', 'Mira', 'Moine', 2)];
    shell.emit({ kind: 'data', data: rows, asOf: 3 });
    const list = st().list;
    expect(list.kind === 'rows' && list.rows.map((r) => r.id)).toEqual(['p9', 'p1', 'p5']);
    expect(viewOf(st()).rows.map((r) => r.id)).toEqual(['p9', 'p1', 'p5']);
  });

  it('shows an empty list as its own state, distinct from not-found and from an error', () => {
    const a = setup();
    a.shell.emit({ kind: 'data', data: [], asOf: 1 });
    const b = setup();
    b.shell.emit({ kind: 'not-found' });
    const c = setup();
    c.shell.emit({ kind: 'error', error: new TransportError('network') });
    expect([viewOf(a.st()).banner, viewOf(b.st()).banner, viewOf(c.st()).banner]).toEqual(['empty', 'not-found', 'error']);
    expect(viewOf(a.st()).add).not.toBeNull();
  });

  it('offers no add, edit or archive in the not-found state, and sends nothing', async () => {
    const { shell, ctrl, st } = loaded();
    shell.emit({ kind: 'not-found' });
    expect(viewOf(st())).toMatchObject({ banner: 'not-found', rows: [], add: null, edit: null });
    fillAdd(ctrl);
    await ctrl.submitAdd();
    await ctrl.archive('p1');
    ctrl.openEdit('p1');
    expect(shell.submits).toHaveLength(0);
    expect(st().edit).toBeNull();
  });

  it('never shows a failed read as an empty list', () => {
    const { shell, st } = setup();
    shell.emit({ kind: 'error', error: new TransportError('http', 500) });
    expect(st().list.kind).toBe('error');
    shell.emit({ kind: 'data', data: [row('p1', 'Aria')], asOf: 2 });
    shell.emit({ kind: 'error', error: new TransportError('network') });
    expect(viewOf(st())).toMatchObject({ banner: 'error' });
    expect(viewOf(st()).rows).toHaveLength(1);
  });

  it('keeps the rows and says they may be late when a read comes back stale', () => {
    const { shell, st } = loaded();
    shell.emit({ kind: 'stale', asOf: 7, wanted: 9 });
    const v = viewOf(st());
    expect(v.banner).toBe('stale');
    expect(v.rows).toHaveLength(2);
  });

  it('treats an answer that is not a list as a read failure', () => {
    const { shell, st } = setup();
    shell.emit({ kind: 'data', data: { oops: true } as never, asOf: 1 });
    expect(st().list.kind).toBe('error');
  });

  it('re-watches when the campaign changes and resets both forms; null reads nothing', () => {
    const { shell, ctrl, st } = loaded();
    fillAdd(ctrl);
    ctrl.openEdit('p1');
    ctrl.setCampagne('camp-2');
    expect(shell.watches[0]?.live).toBe(false);
    expect(shell.watches[1]).toMatchObject({ variables: { campagneId: 'camp-2' }, live: true });
    expect(st().add.values).toEqual({ nom: '', classe: '', niveau: '' });
    expect(st().edit).toBeNull();
    ctrl.setCampagne(null);
    expect(shell.watches).toHaveLength(2);
    expect(shell.watches[1]?.live).toBe(false);
    expect(st().list.kind).toBe('idle');
    expect(viewOf(st())).toMatchObject({ add: null, banner: 'idle' });
  });

  it('issues no read for a null campagneId from the start', () => {
    const { shell, st } = setup(null);
    expect(shell.watches).toHaveLength(0);
    expect(st().list.kind).toBe('idle');
  });
});

describe('add', () => {
  it('sends the payload as typed, to the add DataCapability, with no target and no basedOn', async () => {
    const { shell, ctrl } = loaded();
    shell.submitQueue.push(queued('c1'));
    // Surrounding spaces and a decomposed accent: nothing is trimmed or normalised.
    fillAdd(ctrl, '  Cyrielle  ', 'Rôdeuse', '5');
    void ctrl.submitAdd();
    await flush();
    expect(shell.submits).toHaveLength(1);
    const sent = shell.submits[0]!;
    expect(sent).toMatchObject({ dataCapability: 'campagne.ajouterPj', version: 1, mode: 'relative', idempotencyKey: 'key-1' });
    expect(sent.payload).toEqual({ campagneId: CAMP, nom: '  Cyrielle  ', classe: 'Rôdeuse', niveau: 5 });
    expect(sent.payload['nom']).toBe('  Cyrielle  ');
    expect(sent.target).toBeUndefined();
    expect(sent.basedOn).toBeUndefined();
  });

  it('omits an empty level and sends empty text as it is; the DataGuard refuses both', async () => {
    const { shell, ctrl } = loaded();
    shell.submitQueue.push(queued('c1'));
    void ctrl.submitAdd();
    await flush();
    expect(shell.submits[0]?.payload).toEqual({ campagneId: CAMP, nom: '', classe: '' });
  });

  it.each([
    ['5.0', '5.0'],
    ['abc', 'abc'],
    [' 5', ' 5'],
    ['21', 21],
    ['-3', -3],
    ['5', 5],
  ])('sends the level %j as %j: an integer unclamped, anything else as typed', async (typed, expected) => {
    const { shell, ctrl } = loaded();
    shell.submitQueue.push(queued('c1'));
    fillAdd(ctrl, 'Aria', 'Mage', typed);
    void ctrl.submitAdd();
    await flush();
    expect(shell.submits[0]?.payload['niveau']).toBe(expected);
  });

  it('shows the form pending while the command waits in the queue, with no refusal', async () => {
    const { shell, ctrl, st } = loaded();
    shell.submitQueue.push(queued('c1'));
    fillAdd(ctrl);
    void ctrl.submitAdd();
    await flush();
    expect(st().add.command.kind).toBe('sending');
    expect(viewOf(st()).add).toMatchObject({ busy: true, canSubmit: false, formErrors: [] });
  });

  it('clears the form after an applied add and keeps nothing of the old attempt', async () => {
    const { shell, ctrl, st } = loaded();
    shell.submitQueue.push(queued('c1'));
    shell.script('c1', () => Promise.resolve(settledOutcome('c1', 'applied')));
    fillAdd(ctrl);
    await ctrl.submitAdd();
    expect(st().add.values).toEqual({ nom: '', classe: '', niveau: '' });
    expect(st().add.command).toEqual({ kind: 'done', status: 'applied' });
  });

  it('keeps what the GM typed after a rejected add', async () => {
    const { shell, ctrl, st } = loaded();
    shell.submitQueue.push(queued('c1'));
    rejectAt(shell, 'c1', 'pc-level-range');
    fillAdd(ctrl, 'Aria', 'Mage', '99');
    await ctrl.submitAdd();
    expect(st().add.values).toEqual({ nom: 'Aria', classe: 'Mage', niveau: '99' });
  });

  it('takes a settled first view as the result without waiting', async () => {
    const { shell, ctrl, st } = loaded();
    shell.submitQueue.push(submitted(settledView('c1', 'rejected', ['pc-name-required']), 'c1'));
    await ctrl.submitAdd();
    expect(shell.awaits).toHaveLength(0);
    expect(st().add.fieldErrors.nom?.[0]?.id).toBe('pc-name-required');
  });

  it('does not re-read for a rejection with dataVersion null', async () => {
    const { shell, ctrl, st } = loaded();
    shell.submitQueue.push(queued('c1'));
    rejectAt(shell, 'c1', 'pc-level-range');
    const before = st().list;
    await ctrl.submitAdd();
    expect(shell.watches).toHaveLength(1);
    expect(shell.reads).toHaveLength(0);
    expect(st().list).toBe(before);
  });
});

describe('idempotency', () => {
  it('reuses the key after a network failure and mints a new one after a field edit', async () => {
    const { shell, ctrl } = loaded();
    shell.submitQueue.push(new TransportError('network'), new TransportError('network'), queued('c1'));
    fillAdd(ctrl);
    await ctrl.submitAdd();
    await ctrl.submitAdd();
    expect(shell.submits.map((s) => s.idempotencyKey)).toEqual(['key-1', 'key-1']);
    ctrl.setAddField('nom', 'Autre');
    void ctrl.submitAdd();
    await flush();
    expect(shell.submits[2]?.idempotencyKey).toBe('key-2');
  });

  it('keeps the form editable and says the send failed after a transport error', async () => {
    const { shell, ctrl, st } = loaded();
    shell.submitQueue.push(new TransportError('network'));
    fillAdd(ctrl);
    await ctrl.submitAdd();
    expect(st().add).toMatchObject({ transportError: true, command: { kind: 'idle' } });
    expect(viewOf(st()).add?.canSubmit).toBe(true);
  });

  it('mints a new key after a terminal result', async () => {
    const { shell, ctrl } = loaded();
    shell.submitQueue.push(queued('c1'), queued('c2'));
    rejectAt(shell, 'c1', 'pc-name-unique-in-campaign');
    fillAdd(ctrl);
    await ctrl.submitAdd();
    void ctrl.submitAdd();
    await flush();
    expect(shell.submits.map((s) => s.idempotencyKey)).toEqual(['key-1', 'key-2']);
  });

  it('sends one command for a double click', async () => {
    const { shell, ctrl } = loaded();
    shell.submitQueue.push(queued('c1'), queued('c2'));
    fillAdd(ctrl);
    void ctrl.submitAdd();
    void ctrl.submitAdd();
    await flush();
    expect(shell.submits).toHaveLength(1);
  });

  it('ignores field edits while the command is in flight', async () => {
    const { shell, ctrl, st } = loaded();
    shell.submitQueue.push(queued('c1'));
    fillAdd(ctrl);
    void ctrl.submitAdd();
    await flush();
    ctrl.setAddField('nom', 'Changé');
    expect(st().add.values.nom).toBe('Cyrielle');
  });
});

describe('violations', () => {
  it('marks the input each id belongs to, all of them together', async () => {
    const { shell, ctrl, st } = loaded();
    shell.submitQueue.push(queued('c1'));
    rejectAt(shell, 'c1', 'pc-name-required', 'pc-class-required', 'pc-level-range');
    await ctrl.submitAdd();
    const f = st().add;
    expect(f.fieldErrors.nom?.map((v) => v.id)).toEqual(['pc-name-required']);
    expect(f.fieldErrors.classe?.map((v) => v.id)).toEqual(['pc-class-required']);
    expect(f.fieldErrors.niveau?.map((v) => v.id)).toEqual(['pc-level-range']);
    expect(f.formErrors).toEqual([]);
  });

  it('marks the name for a duplicate name', async () => {
    const { shell, ctrl, st } = loaded();
    shell.submitQueue.push(queued('c1'));
    rejectAt(shell, 'c1', 'pc-name-unique-in-campaign');
    fillAdd(ctrl);
    await ctrl.submitAdd();
    expect(st().add.fieldErrors.nom?.[0]?.id).toBe('pc-name-unique-in-campaign');
  });

  it('shows campaign-active, pc-active and an unknown id at form level with the id as received', async () => {
    const { shell, ctrl, st } = loaded();
    shell.submitQueue.push(queued('c1'));
    // `pc-name-required-x` shares a prefix with a mapped id: it must not be mapped by prefix.
    rejectAt(shell, 'c1', 'campaign-active', 'pc-active', 'pc-name-required-x', 'brand-new-rule');
    await ctrl.submitAdd();
    const f = st().add;
    expect(f.fieldErrors).toEqual({});
    expect(f.formErrors.map((v) => v.id)).toEqual(['campaign-active', 'pc-active', 'pc-name-required-x', 'brand-new-rule']);
    for (const v of f.formErrors) expect(v.text).toContain(v.id!);
  });

  it('shows a rejection without ids at form level', async () => {
    const { shell, ctrl, st } = loaded();
    shell.submitQueue.push(queued('c1'));
    rejectAt(shell, 'c1');
    await ctrl.submitAdd();
    expect(st().add.formErrors).toHaveLength(1);
    expect(st().add.command).toEqual({ kind: 'done', status: 'rejected' });
  });

  it('clears the mark of a field when that field is edited, and the others stay', async () => {
    const { shell, ctrl, st } = loaded();
    shell.submitQueue.push(queued('c1'));
    rejectAt(shell, 'c1', 'pc-name-required', 'pc-level-range');
    await ctrl.submitAdd();
    ctrl.setAddField('nom', 'Aria');
    expect(st().add.fieldErrors.nom).toBeUndefined();
    expect(st().add.fieldErrors.niveau).toHaveLength(1);
  });

  it('clears every mark when a later command applies', async () => {
    const { shell, ctrl, st } = loaded();
    shell.submitQueue.push(queued('c1'), queued('c2'));
    rejectAt(shell, 'c1', 'campaign-active', 'pc-level-range');
    await ctrl.submitAdd();
    shell.script('c2', () => Promise.resolve(settledOutcome('c2', 'applied')));
    await ctrl.submitAdd();
    expect(st().add.fieldErrors).toEqual({});
    expect(st().add.formErrors).toEqual([]);
  });
});

describe('edit', () => {
  const sendEdit = async (s: ReturnType<typeof loaded>, id = 'c1') => {
    s.shell.submitQueue.push(queued(id));
    void s.ctrl.submitEdit();
    await flush();
  };

  it('opens pre-filled with the current name, class and level', () => {
    const s = loaded();
    s.ctrl.openEdit('p1');
    expect(s.st().edit).toMatchObject({ pcId: 'p1', values: { nom: 'Aria', classe: 'Mage', niveau: '4' } });
  });

  it('sends the PC id, the version it was read at, and neither campagneId nor anything else', async () => {
    const s = loaded();
    s.ctrl.openEdit('p1');
    s.ctrl.setEditField('niveau', '5');
    await sendEdit(s);
    const sent = s.shell.submits[0]!;
    expect(sent).toMatchObject({
      dataCapability: 'campagne.modifierPJ',
      version: 1,
      mode: 'confirm_on_stale',
      target: { aggregate: 'PJ', id: 'p1' },
      basedOn: { version: 7 },
    });
    expect(sent.payload).toEqual({ nom: 'Aria', classe: 'Mage', niveau: 5 });
  });

  it('keeps basedOn as it was when the form opened, whatever refreshes follow', async () => {
    const s = loaded();
    s.ctrl.openEdit('p1');
    s.shell.emit({ kind: 'data', data: [row('p1', 'Aria', 'Mage', 4)], asOf: 12 });
    await sendEdit(s);
    expect(s.shell.submits[0]?.basedOn).toEqual({ version: 7 });
  });

  it('falls back to the known dataVersion, then to 0, when the read carries no asOf', () => {
    const a = loaded(undefined, null);
    a.shell.known = 4;
    a.ctrl.openEdit('p1');
    expect(a.st().edit?.basedOn).toBe(4);
    const b = loaded(undefined, null);
    b.ctrl.openEdit('p2');
    expect(b.st().edit?.basedOn).toBe(0);
  });

  it('closes the form after an applied edit and leaves the list to the refresh', async () => {
    const s = loaded();
    s.ctrl.openEdit('p1');
    s.shell.script('c1', () => Promise.resolve(settledOutcome('c1', 'applied')));
    await sendEdit(s);
    expect(s.st().edit).toBeNull();
    const list = s.st().list;
    expect(list.kind === 'rows' && list.rows[0]?.level).toBe(4);
  });

  it('keeps the form and the typed values, form-level, when the PC was archived meanwhile', async () => {
    const s = loaded();
    s.ctrl.openEdit('p1');
    s.ctrl.setEditField('nom', 'Aria bis');
    s.shell.emit({ kind: 'data', data: [row('p2', 'Borek')], asOf: 9 });
    expect(s.st().edit).not.toBeNull();
    s.shell.script('c1', () => Promise.resolve(settledOutcome('c1', 'rejected', ['pc-active'])));
    await sendEdit(s);
    expect(s.st().edit?.values.nom).toBe('Aria bis');
    expect(s.st().edit?.formErrors.map((v) => v.id)).toEqual(['pc-active']);
  });

  it('cannot be closed while a command of the form is in flight', async () => {
    const s = loaded();
    s.ctrl.openEdit('p1');
    void sendEdit(s);
    await flush();
    s.ctrl.closeEdit();
    expect(s.st().edit).not.toBeNull();
    expect(viewOf(s.st()).edit?.canClose).toBe(false);
  });
});

describe('awaiting confirmation', () => {
  const ahead: Message = {
    message: 'ValueDeclaredAhead',
    to: ['gm'],
    command: 'c1',
    field: 'PJ.level',
    declaredAhead: { value: { name: 'Aria', class: 'Mage', level: 6 }, by: 'gm', command: 'c0' },
    actions: ['confirm_overwrite', 'cancel', 'edit'],
  };
  const other: Message = { ...ahead, command: 'someone-else', declaredAhead: { value: { name: 'X', class: 'X', level: 1 }, by: 'gm', command: 'c9' } };
  const yours = { name: 'Aria', class: 'Mage', level: 9 };

  /** An edit whose first observed state is awaiting_confirmation. */
  async function parked(actions: ('confirm_overwrite' | 'cancel' | 'edit')[] = ['confirm_overwrite', 'cancel', 'edit']) {
    const s = loaded();
    // How the wait for c1 ends; unresolved until a test says so.
    const end = deferred<AwaitOutcome>();
    s.shell.script('c1', () => end.promise);
    s.ctrl.openEdit('p1');
    s.ctrl.setEditField('niveau', '9');
    const view = pendingView('c1', 'awaiting_confirmation', { yourValue: yours, messages: [other, ahead], actions });
    s.shell.submitQueue.push(submitted(view, 'c1'));
    void s.ctrl.submitEdit();
    await flush();
    return Object.assign(s, { end });
  }

  it('shows yourValue beside the value ahead, from the message, ignoring other commands’ messages', async () => {
    const s = await parked();
    expect(s.st().edit?.command).toMatchObject({
      kind: 'awaiting',
      commandId: 'c1',
      yourValue: yours,
      ahead: { name: 'Aria', class: 'Mage', level: 6 },
      aheadField: 'niveau',
      actions: ['confirm_overwrite', 'cancel'],
    });
  });

  it('reads the value ahead from the projection when no message carries it', async () => {
    const s = loaded();
    s.ctrl.openEdit('p1');
    const view = pendingView('c1', 'awaiting_confirmation', {
      yourValue: yours,
      actions: ['confirm_overwrite', 'cancel'],
      projection: { pendingAhead: [{ command: 'c0', value: { name: 'Aria', class: 'Mage', level: 7 } }] },
    });
    s.shell.submitQueue.push(submitted(view, 'c1'));
    void s.ctrl.submitEdit();
    await flush();
    expect(s.st().edit?.command).toMatchObject({ kind: 'awaiting', ahead: { level: 7 } });
  });

  it('confirms nothing by itself, even after time passes, and leaves the list unchanged', async () => {
    const s = await parked();
    const rowsBefore = viewOf(s.st()).rows;
    await flush();
    await flush();
    expect(s.shell.acts).toEqual([]);
    expect(viewOf(s.st()).rows).toEqual(rowsBefore);
    expect(JSON.stringify(viewOf(s.st()).rows)).not.toContain('"level":9');
  });

  it('confirms on the GM’s gesture, then follows the command to its end', async () => {
    const s = await parked();
    s.shell.actHandler = () => Promise.resolve(pendingView('c1', 'confirmed'));
    await s.ctrl.confirmEdit();
    expect(s.shell.acts).toEqual([{ id: 'c1', action: 'confirm_overwrite' }]);
    expect(s.st().edit?.command.kind).toBe('sending');
    s.end.resolve(settledOutcome('c1', 'applied'));
    await flush();
    expect(s.st().edit).toBeNull();
  });

  it('cancels on the GM’s gesture and changes nothing', async () => {
    const s = await parked();
    s.shell.actHandler = () => Promise.resolve(settledView('c1', 'cancelled'));
    await s.ctrl.cancelEdit();
    expect(s.shell.acts).toEqual([{ id: 'c1', action: 'cancel' }]);
    expect(s.st().edit?.command).toEqual({ kind: 'done', status: 'cancelled' });
    expect(viewOf(s.st()).edit).toMatchObject({ notice: 'cancelled', awaiting: null });
    expect(s.shell.submits).toHaveLength(1);
  });

  it('offers nothing the latest view did not offer, and sends no request for it', async () => {
    const s = await parked(['cancel']);
    expect(viewOf(s.st()).edit?.awaiting).toMatchObject({ canConfirm: false, canCancel: true });
    await s.ctrl.confirmEdit();
    expect(s.shell.acts).toEqual([]);
  });

  it('shows the queue’s state, not a success, when the shell refuses a confirm that is no longer offered', async () => {
    const s = await parked();
    s.shell.actHandler = (id, action) => Promise.reject(new ActionNotOfferedError(id, action));
    s.shell.lookupHandler = () => Promise.resolve(settledView('c1', 'expired'));
    await s.ctrl.confirmEdit();
    expect(s.shell.lookups).toEqual(['c1']);
    expect(s.st().edit?.command).toEqual({ kind: 'done', status: 'expired' });
    expect(s.st().edit).not.toBeNull();
  });

  it('shows an expired edit as terminal, with no confirm, and the list as it was', async () => {
    const s = await parked();
    const shown = () => viewOf(s.st()).rows.map(({ id, name, class: cls, level }) => ({ id, name, cls, level }));
    const before = shown();
    s.end.resolve(settledOutcome('c1', 'expired'));
    await flush();
    expect(s.st().edit?.command).toEqual({ kind: 'done', status: 'expired' });
    expect(viewOf(s.st()).edit).toMatchObject({ notice: 'expired', awaiting: null });
    expect(shown()).toEqual(before);
    await s.ctrl.confirmEdit();
    expect(s.shell.acts).toEqual([]);
  });

  it('ignores a second terminal message for a command that already ended', async () => {
    const s = await parked();
    s.shell.actHandler = () => Promise.resolve(settledView('c1', 'cancelled'));
    await s.ctrl.cancelEdit();
    s.end.resolve(settledOutcome('c1', 'applied'));
    await flush();
    expect(s.st().edit?.command).toEqual({ kind: 'done', status: 'cancelled' });
  });

  it('renews the wait when it times out, for as long as the page lives, and never renews a hold', async () => {
    const s = loaded();
    s.shell.submitQueue.push(queued('c1'));
    s.shell.script(
      'c1',
      () => Promise.resolve({ kind: 'still-pending', commandId: 'c1', reason: 'timeout', last: null }),
      () => Promise.resolve({ kind: 'still-pending', commandId: 'c1', reason: 'timeout', last: null }),
      () => Promise.resolve(settledOutcome('c1', 'applied')),
    );
    fillAddAndSend(s);
    await flush();
    expect(s.shell.awaits.filter((a) => a.id === 'c1')).toHaveLength(3);
    expect(s.st().add.command).toEqual({ kind: 'done', status: 'applied' });
    expect(s.shell.acts).toEqual([]);
  });

  it('ignores a confirm gesture when nothing is awaiting', async () => {
    const s = loaded();
    s.ctrl.openEdit('p1');
    await s.ctrl.confirmEdit();
    expect(s.shell.acts).toEqual([]);
  });
});

function fillAddAndSend(s: ReturnType<typeof loaded>): void {
  fillAdd(s.ctrl);
  void s.ctrl.submitAdd();
}

describe('archive', () => {
  it('archives by id with an empty payload, no reason and no confirmation step', async () => {
    const s = loaded();
    s.shell.submitQueue.push(queued('c1'));
    s.shell.script('c1', () => Promise.resolve(settledOutcome('c1', 'applied')));
    await s.ctrl.archive('p1');
    expect(s.shell.submits[0]).toMatchObject({
      dataCapability: 'campagne.archiverPJ',
      version: 1,
      mode: 'overwrite',
      target: { aggregate: 'PJ', id: 'p1' },
      payload: {},
    });
    expect(s.shell.submits[0]?.basedOn).toBeUndefined();
  });

  it('shows an applied archive with dataVersion null (already archived) as a success', async () => {
    const s = loaded();
    s.shell.submitQueue.push(queued('c1'));
    s.shell.script('c1', () => Promise.resolve(settledOutcome('c1', 'applied', [], null)));
    await s.ctrl.archive('p1');
    expect(s.st().archiving.size).toBe(0);
    expect(s.st().rowNotices).toEqual({});
  });

  it('sends one command for a double click and marks the row while it runs', async () => {
    const s = loaded();
    s.shell.submitQueue.push(queued('c1'), queued('c2'));
    void s.ctrl.archive('p1');
    void s.ctrl.archive('p1');
    await flush();
    expect(s.shell.submits).toHaveLength(1);
    expect(viewOf(s.st()).rows[0]).toMatchObject({ archiving: true, canArchive: false, canEdit: false });
    expect(viewOf(s.st()).rows[1]).toMatchObject({ archiving: false, canArchive: true });
  });

  it('reports a rejected archive on its row, id as received', async () => {
    const s = loaded();
    s.shell.submitQueue.push(queued('c1'));
    rejectAt(s.shell, 'c1', 'some-rule');
    await s.ctrl.archive('p1');
    expect(s.st().rowNotices['p1']).toMatchObject({ kind: 'rejected', violations: [{ id: 'some-rule' }] });
  });

  it('retries after a network failure with the same key', async () => {
    const s = loaded();
    s.shell.submitQueue.push(new TransportError('network'), queued('c1'));
    await s.ctrl.archive('p1');
    expect(s.st().rowNotices['p1']).toEqual({ kind: 'transport' });
    void s.ctrl.archive('p1');
    await flush();
    expect(s.shell.submits.map((x) => x.idempotencyKey)).toEqual(['key-1', 'key-1']);
  });

  it('touches no other row and leaves an open edit form alone', async () => {
    const s = loaded();
    s.ctrl.openEdit('p2');
    s.ctrl.setEditField('nom', 'Borek le Brun');
    s.shell.submitQueue.push(queued('c1'));
    s.shell.script('c1', () => Promise.resolve(settledOutcome('c1', 'applied')));
    await s.ctrl.archive('p1');
    expect(s.st().edit).toMatchObject({ pcId: 'p2', values: { nom: 'Borek le Brun' } });
    expect(s.shell.submits).toHaveLength(1);
  });
});

describe('dispose', () => {
  it('aborts waits, stops watching and ignores later gestures', async () => {
    const s = loaded();
    s.shell.submitQueue.push(queued('c1'));
    fillAdd(s.ctrl);
    void s.ctrl.submitAdd();
    await flush();
    const signal = s.shell.awaits[0]?.opts.signal;
    expect(signal?.aborted).toBe(false);
    s.ctrl.dispose();
    expect(signal?.aborted).toBe(true);
    expect(s.shell.watches[0]?.live).toBe(false);
    const seen: unknown[] = [];
    s.ctrl.subscribe((x) => seen.push(x));
    await s.ctrl.submitAdd();
    s.ctrl.setCampagne('camp-9');
    s.shell.emit({ kind: 'data', data: [], asOf: 1 });
    expect(seen).toEqual([]);
    expect(s.shell.submits).toHaveLength(1);
    expect(s.shell.disposed).toBe(false);
  });
});

describe('vocabulary', () => {
  it('has no way to say a command was refused for concurrency', async () => {
    const s = await (async () => {
      const x = loaded();
      x.ctrl.openEdit('p1');
      x.shell.submitQueue.push(submitted(pendingView('c1', 'awaiting_confirmation', { yourValue: {}, actions: ['cancel'] }), 'c1'));
      void x.ctrl.submitEdit();
      await flush();
      return x;
    })();
    const shown = JSON.stringify(viewOf(s.st()));
    expect(shown).not.toMatch(/conflict|concurren/i);
  });
});

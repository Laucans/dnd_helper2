import { TransportError, type AwaitOptions } from '@dnd-helper/micro-ui-shell';
import { describe, expect, it } from 'vitest';
import { CMD, deferred, flush, pendingView, rig, settledOutcome, settledView, submitted } from './fakes';

const queuedAndApplied = (r: ReturnType<typeof rig>): void => {
  r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome('applied'));
};

describe('create (SPEC rules 12–19)', () => {
  it.each([['empty', ''], ['whitespace only', '   '], ['101 characters', 'x'.repeat(101)], ['padded', '  Alpha  ']])(
    'sends a %s name verbatim: no client-side rule',
    async (_label, name) => {
      const r = rig();
      queuedAndApplied(r);
      r.controller.setName(name);
      await r.controller.submitCreate();
      expect(r.shell.submits).toHaveLength(1);
      const sent = r.shell.submits[0]!;
      expect(sent.payload).toEqual({ name });
      expect(sent.dataCapability).toBe('campagne.creerCampagne');
      expect(sent.version).toBe(1);
      expect(sent.mode).toBe('relative');
      expect(typeof sent.idempotencyKey).toBe('string');
    },
  );

  it('makes one submit for a double submit while the first is pending (rule 18)', async () => {
    const r = rig();
    const gate = deferred<ReturnType<typeof settledOutcome>>();
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(gate.promise);
    r.controller.setName('Alpha');
    const first = r.controller.submitCreate();
    await flush();
    await r.controller.submitCreate();
    await r.controller.submitCreate();
    expect(r.shell.submits).toHaveLength(1);
    gate.resolve(settledOutcome('applied'));
    await first;
    expect(r.shell.submits).toHaveLength(1);
  });

  it('reuses the key for a retry of the same name after a send failure', async () => {
    const r = rig();
    r.shell.replySubmit(new TransportError('network'), submitted(pendingView('queued'))).replyAwait(settledOutcome('applied'));
    r.controller.setName('Alpha');
    await r.controller.submitCreate();
    expect(r.controller.getState().create.phase.kind).toBe('send-failed');
    expect(r.controller.getState().create.name).toBe('Alpha');
    await r.controller.submitCreate();
    expect(r.shell.submits.map((s) => s.idempotencyKey)).toEqual(['key-1', 'key-1']);
    expect(r.keys).toEqual(['key-1']);
  });

  it('mints a new key when the name changes', async () => {
    const r = rig();
    r.shell.replySubmit(new TransportError('network'), submitted(pendingView('queued'))).replyAwait(settledOutcome('applied'));
    r.controller.setName('Alpha');
    await r.controller.submitCreate();
    r.controller.setName('Alphab');
    await r.controller.submitCreate();
    expect(r.shell.submits.map((s) => s.idempotencyKey)).toEqual(['key-1', 'key-2']);
  });

  it('keeps the key when the same text is typed back to the same value', async () => {
    const r = rig();
    r.shell.replySubmit(new TransportError('network'), submitted(pendingView('queued'))).replyAwait(settledOutcome('applied'));
    r.controller.setName('Alpha');
    await r.controller.submitCreate();
    r.controller.setName('Alpha');
    await r.controller.submitCreate();
    expect(r.keys).toEqual(['key-1']);
  });

  it('mints a new key for a submit after a terminal result', async () => {
    const r = rig();
    r.shell
      .replySubmit(submitted(pendingView('queued')), submitted(pendingView('queued')))
      .replyAwait(settledOutcome('rejected', ['campaign-name-required']), settledOutcome('applied'));
    r.controller.setName('');
    await r.controller.submitCreate();
    await r.controller.submitCreate();
    expect(r.shell.submits.map((s) => s.idempotencyKey)).toEqual(['key-1', 'key-2']);
  });

  it('clears the form after applied', async () => {
    const r = rig();
    queuedAndApplied(r);
    r.controller.setName('Alpha');
    await r.controller.submitCreate();
    const { create } = r.controller.getState();
    expect(create.name).toBe('');
    expect(create.phase).toMatchObject({ kind: 'settled', status: 'applied', dataVersion: 7 });
  });

  it('does not erase text typed while the command was running', async () => {
    const r = rig();
    const gate = deferred<ReturnType<typeof settledOutcome>>();
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(gate.promise);
    r.controller.setName('Alpha');
    const run = r.controller.submitCreate();
    await flush();
    r.controller.setName('Beta');
    gate.resolve(settledOutcome('applied'));
    await run;
    expect(r.controller.getState().create.name).toBe('Beta');
  });

  it.each([
    ['campaign-name-required', 'Le nom de la campagne est obligatoire.'],
    ['campaign-name-length', 'Le nom de la campagne est trop long.'],
  ])('shows %s on the name field and keeps the text (rules 14, 15)', async (id, message) => {
    const r = rig();
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome('rejected', [id]));
    r.controller.setName('   ');
    await r.controller.submitCreate();
    const { create } = r.controller.getState();
    expect(create.fieldErrors.name).toEqual([message]);
    expect(create.formErrors).toEqual([]);
    expect(create.name).toBe('   ');
    expect(create.phase).toMatchObject({ kind: 'settled', status: 'rejected', dataVersion: null });
    expect(create.phase.kind === 'settled' && create.phase.violations.map((v) => v.id)).toEqual([id]);
  });

  it('shows an unknown violation id as a general error, never swallowed', async () => {
    const r = rig();
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome('rejected', ['campaign-exotic']));
    r.controller.setName('Alpha');
    await r.controller.submitCreate();
    const { create } = r.controller.getState();
    expect(create.fieldErrors.name).toEqual([]);
    expect(create.formErrors).toEqual(['Refus non décrit : campaign-exotic']);
  });

  it('shows a rejection with no violation id as a refusal', async () => {
    const r = rig();
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome('rejected', []));
    r.controller.setName('Alpha');
    await r.controller.submitCreate();
    const { create } = r.controller.getState();
    expect(create.formErrors).toEqual(['La commande a été refusée.']);
    expect(create.phase).toMatchObject({ kind: 'settled', status: 'rejected' });
  });

  it('splits a mixed rejection between the field and the form', async () => {
    const r = rig();
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome('rejected', ['campaign-name-length', 'other']));
    r.controller.setName('Alpha');
    await r.controller.submitCreate();
    const { create } = r.controller.getState();
    expect(create.fieldErrors.name).toHaveLength(1);
    expect(create.formErrors).toEqual(['Refus non décrit : other']);
  });

  it.each(['expired', 'cancelled'] as const)('keeps the text after %s', async (status) => {
    const r = rig();
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome(status));
    r.controller.setName('Alpha');
    await r.controller.submitCreate();
    const { create } = r.controller.getState();
    expect(create.name).toBe('Alpha');
    expect(create.phase).toMatchObject({ kind: 'settled', status, dataVersion: null });
  });

  it('shows queued, then awaiting_confirmation, and never acts on it (rule 28)', async () => {
    const r = rig();
    const seen: string[] = [];
    r.controller.subscribe((s) => {
      if (s.create.phase.kind === 'pending') seen.push(s.create.phase.state);
    });
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait((opts: AwaitOptions) => {
      opts.onState?.(pendingView('awaiting_confirmation'));
      return settledOutcome('applied');
    });
    r.controller.setName('Alpha');
    await r.controller.submitCreate();
    expect(seen).toEqual(['queued', 'awaiting_confirmation']);
    expect('act' in r.shell).toBe(false);
  });

  it('shows a state with no label of its own as pending, not as an error', async () => {
    const r = rig();
    const gate = deferred<ReturnType<typeof settledOutcome>>();
    r.shell.replySubmit(submitted(pendingView('confirmed'))).replyAwait(gate.promise);
    r.controller.setName('Alpha');
    const run = r.controller.submitCreate();
    await flush();
    expect(r.controller.getState().create.phase).toEqual({ kind: 'pending', state: 'confirmed', commandId: CMD });
    gate.resolve(settledOutcome('applied'));
    await run;
  });

  it('keeps waiting after a timeout until the command is terminal', async () => {
    const r = rig();
    r.shell
      .replySubmit(submitted(pendingView('queued')))
      .replyAwait({ kind: 'still-pending', commandId: CMD, reason: 'timeout', last: null }, { kind: 'still-pending', commandId: CMD, reason: 'timeout', last: null }, settledOutcome('applied'));
    r.controller.setName('Alpha');
    await r.controller.submitCreate();
    expect(r.shell.awaits).toHaveLength(3);
    expect(r.controller.getState().create.phase).toMatchObject({ kind: 'settled', status: 'applied' });
  });

  it('shows a replayed, already finished command without waiting', async () => {
    const r = rig();
    r.shell.replySubmit(submitted(settledView('applied', [], 9), { replayed: true }));
    r.controller.setName('Alpha');
    await r.controller.submitCreate();
    expect(r.shell.awaits).toHaveLength(0);
    expect(r.controller.getState().create.phase).toMatchObject({ kind: 'settled', status: 'applied', dataVersion: 9 });
    expect(r.controller.getState().create.name).toBe('');
  });

  it('shows a command the server no longer knows as lost, and keeps the key', async () => {
    const r = rig();
    r.shell.replySubmit(submitted(pendingView('queued')), submitted(pendingView('queued'))).replyAwait({ kind: 'not-found', commandId: CMD }, settledOutcome('applied'));
    r.controller.setName('Alpha');
    await r.controller.submitCreate();
    expect(r.controller.getState().create.phase).toEqual({ kind: 'lost', commandId: CMD });
    await r.controller.submitCreate();
    expect(r.keys).toEqual(['key-1']);
  });

  it('recovers from a failure while waiting: the same key replays the command', async () => {
    const r = rig();
    r.shell.replySubmit(submitted(pendingView('queued')), submitted(settledView('applied'), { replayed: true })).replyAwait(new TransportError('network'));
    r.controller.setName('Alpha');
    await r.controller.submitCreate();
    expect(r.controller.getState().create.phase.kind).toBe('send-failed');
    await r.controller.submitCreate();
    expect(r.keys).toEqual(['key-1']);
    expect(r.controller.getState().create.phase).toMatchObject({ status: 'applied' });
  });

  it('triggers no list read of its own: a rejected result leaves the list untouched (rule 35)', async () => {
    const r = rig();
    r.shell.data([{ id: 'a', name: 'Alpha' }], 3);
    const list = r.controller.getState().list;
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome('rejected', ['campaign-name-required']));
    r.controller.setName('');
    await r.controller.submitCreate();
    expect(r.controller.getState().list).toBe(list);
    expect(r.shell.watches).toHaveLength(1);
  });

  it('clears the previous errors when the GM edits the text', async () => {
    const r = rig();
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome('rejected', ['campaign-name-required']));
    r.controller.setName('');
    await r.controller.submitCreate();
    r.controller.setName('A');
    const { create } = r.controller.getState();
    expect(create.fieldErrors.name).toEqual([]);
    expect(create.phase).toEqual({ kind: 'idle' });
  });

  it('stops everything on dispose: no state change after it', async () => {
    const r = rig();
    const gate = deferred<ReturnType<typeof settledOutcome>>();
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(gate.promise);
    r.controller.setName('Alpha');
    const run = r.controller.submitCreate();
    await flush();
    r.controller.dispose();
    const frozen = r.controller.getState();
    gate.resolve(settledOutcome('applied'));
    await run;
    expect(r.controller.getState()).toBe(frozen);
    expect(r.shell.awaits[0]?.opts.signal?.aborted).toBe(true);
  });
});

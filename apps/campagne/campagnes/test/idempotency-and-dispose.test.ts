// Regressions the per-rule suites leave open: what an idempotency key does
// across a terminal result, a replayed refusal, a refresh that overtakes a
// command, and a controller disposed while a confirmation is open.

import { describe, expect, it } from 'vitest';
import { CMD, deferred, flush, pendingView, rig, row, settledOutcome, settledView, submitted } from './fakes';

const two = [row('b', 'Beta'), row('a', 'Alpha')];

describe('archive keys (SPEC rules 21, 25)', () => {
  it('mints a new key for an archive asked again after a terminal result', async () => {
    const r = rig();
    r.shell.data(two, 1);
    r.shell
      .replySubmit(submitted(pendingView('queued')), submitted(pendingView('queued')))
      .replyAwait(settledOutcome('rejected', []), settledOutcome('applied'));
    await r.controller.requestArchive('a');
    expect(r.controller.getState().archive['a']).toMatchObject({ kind: 'settled', status: 'rejected' });
    await r.controller.requestArchive('a');
    const keys = r.shell.submits.map((s) => s.idempotencyKey);
    expect(keys).toHaveLength(2);
    expect(keys[1]).not.toBe(keys[0]);
  });

  it('keeps the pending status of a row that a refresh removes before the result arrives (rule 30)', async () => {
    const r = rig();
    r.shell.data(two, 1);
    const gate = deferred<ReturnType<typeof settledOutcome>>();
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(gate.promise);
    const run = r.controller.requestArchive('a');
    await flush();
    expect(r.controller.getState().archive['a']?.kind).toBe('pending');
    // The applied command bumps dataVersion, so the list can drop the row first.
    r.shell.data([row('b', 'Beta')], 7);
    expect(r.controller.getState().archive['a']).toMatchObject({ kind: 'pending', commandId: CMD });
    gate.resolve(settledOutcome('applied'));
    await run;
    expect(r.controller.getState().archive['a']).toMatchObject({ kind: 'settled', status: 'applied' });
  });
});

describe('create refusals (SPEC rules 14, 15, 17)', () => {
  it('shows the field message when a retry replays a command the DataGuard already refused', async () => {
    const r = rig();
    r.shell.replySubmit(new Error('boom'), submitted(settledView('rejected', ['campaign-name-required']), { replayed: true }));
    r.controller.setName('   ');
    await r.controller.submitCreate();
    expect(r.controller.getState().create.phase.kind).toBe('send-failed');
    await r.controller.submitCreate();
    const { create } = r.controller.getState();
    expect(r.shell.awaits).toHaveLength(0);
    expect(create.fieldErrors.name).toEqual(['Le nom de la campagne est obligatoire.']);
    expect(create.name).toBe('   ');
  });

  it('keeps the refusal on screen when the field is set to the text it already holds', async () => {
    const r = rig();
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome('rejected', ['campaign-name-required']));
    r.controller.setName('');
    await r.controller.submitCreate();
    const before = r.controller.getState().create;
    r.controller.setName('');
    expect(r.controller.getState().create).toEqual(before);
    expect(before.fieldErrors.name).toHaveLength(1);
  });
});

describe('dispose in the middle of a command', () => {
  it('sends no create after dispose', async () => {
    const r = rig();
    r.controller.setName('Alpha');
    r.controller.dispose();
    await r.controller.submitCreate();
    expect(r.shell.submits).toHaveLength(0);
  });

  it('opens no confirmation and sends no archive after dispose', async () => {
    const r = rig();
    r.shell.data(two, 1);
    r.controller.dispose();
    await r.controller.requestArchive('a');
    expect(r.confirms).toHaveLength(0);
    expect(r.shell.submits).toHaveLength(0);
  });

  it('sends nothing when the GM confirms after the Micro-UI was unmounted', async () => {
    const r = rig();
    r.shell.data(two, 1);
    const answer = deferred<boolean>();
    r.answers.push(answer.promise);
    const run = r.controller.requestArchive('a');
    await flush();
    expect(r.confirms).toHaveLength(1);
    r.controller.dispose();
    answer.resolve(true);
    await run;
    expect(r.shell.submits).toHaveLength(0);
  });
});

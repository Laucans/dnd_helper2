import { describe, expect, it } from 'vitest';
import { CMD, deferred, flush, pendingView, rig, row, settledOutcome, submitted } from './fakes';

const two = [row('b', 'Beta'), row('a', 'Alpha')];

describe('archive (SPEC rules 20–24, 30)', () => {
  it('sends nothing when the GM cancels the confirmation (rule 23)', async () => {
    const r = rig();
    r.shell.data(two, 1);
    r.answers.push(false);
    await r.controller.requestArchive('a');
    expect(r.confirms).toHaveLength(1);
    expect(r.shell.submits).toHaveLength(0);
    expect(r.controller.getState().archive).toEqual({});
  });

  it('names the row in the confirmation, as returned', async () => {
    const r = rig();
    r.shell.data([row('a', '  Alpha  ')], 1);
    r.answers.push(false);
    await r.controller.requestArchive('a');
    expect(r.confirms[0]).toContain('« ' + '  Alpha  ' + ' »');
  });

  it('sends exactly one archive, with the clicked id and an empty payload, once confirmed', async () => {
    const r = rig();
    r.shell.data(two, 1);
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome('applied'));
    await r.controller.requestArchive('a');
    expect(r.confirms).toHaveLength(1);
    expect(r.shell.submits).toHaveLength(1);
    const sent = r.shell.submits[0]!;
    expect(sent.dataCapability).toBe('campagne.archiverCampagne');
    expect(sent.version).toBe(1);
    expect(sent.mode).toBe('overwrite');
    expect(sent.target).toEqual({ aggregate: 'Campagne', id: 'a' });
    expect(sent.payload).toEqual({});
    expect(typeof sent.idempotencyKey).toBe('string');
  });

  it('shows an already archived campaign (applied, no dataVersion) as a success (rule 21)', async () => {
    const r = rig();
    r.shell.data(two, 1);
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome('applied', [], null));
    await r.controller.requestArchive('a');
    expect(r.controller.getState().archive['a']).toMatchObject({ kind: 'settled', status: 'applied', dataVersion: null });
  });

  it('leaves the row in the list until the next read drops it (rule 30)', async () => {
    const r = rig();
    r.shell.data(two, 1);
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome('applied'));
    await r.controller.requestArchive('a');
    const list = r.controller.getState().list;
    expect(list.kind === 'ready' && list.rows.map((x) => x.id)).toEqual(['b', 'a']);
    r.shell.data([row('b', 'Beta')], 7);
    const after = r.controller.getState();
    expect(after.list.kind === 'ready' && after.list.rows.map((x) => x.id)).toEqual(['b']);
    expect(after.archive).toEqual({});
  });

  it('sends nothing when the row left the list while the dialog was open', async () => {
    const r = rig();
    r.shell.data(two, 1);
    const gate = deferred<boolean>();
    r.answers.push(gate.promise);
    const run = r.controller.requestArchive('a');
    r.shell.data([row('b', 'Beta')], 2);
    gate.resolve(true);
    await run;
    expect(r.shell.submits).toHaveLength(0);
    expect(r.controller.getState().archive).toEqual({});
  });

  it('opens one dialog and sends one command for a double click', async () => {
    const r = rig();
    r.shell.data(two, 1);
    const gate = deferred<boolean>();
    r.answers.push(gate.promise);
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome('applied'));
    const first = r.controller.requestArchive('a');
    await r.controller.requestArchive('a');
    gate.resolve(true);
    await first;
    expect(r.confirms).toHaveLength(1);
    expect(r.shell.submits).toHaveLength(1);
  });

  it('is a no-op while that row is pending: no second confirmation, no second submit', async () => {
    const r = rig();
    r.shell.data(two, 1);
    const gate = deferred<ReturnType<typeof settledOutcome>>();
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(gate.promise);
    const first = r.controller.requestArchive('a');
    await flush();
    await r.controller.requestArchive('a');
    expect(r.confirms).toHaveLength(1);
    expect(r.shell.submits).toHaveLength(1);
    expect(r.controller.getState().archive['a']).toEqual({ kind: 'pending', state: 'queued', commandId: CMD });
    gate.resolve(settledOutcome('applied'));
    await first;
  });

  it('archives two different rows independently', async () => {
    const r = rig();
    r.shell.data(two, 1);
    r.shell
      .replySubmit(submitted(pendingView('queued')), submitted(pendingView('queued')))
      .replyAwait(settledOutcome('applied'), settledOutcome('applied'));
    await r.controller.requestArchive('a');
    await r.controller.requestArchive('b');
    expect(r.shell.submits.map((s) => s.target?.id)).toEqual(['a', 'b']);
    expect(r.keys).toEqual(['key-1', 'key-2']);
  });

  it('shows a rejection on its row, with the violation ids', async () => {
    const r = rig();
    r.shell.data(two, 1);
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome('rejected', ['campaign-active']));
    await r.controller.requestArchive('a');
    const phase = r.controller.getState().archive['a'];
    expect(phase).toMatchObject({ kind: 'settled', status: 'rejected', dataVersion: null });
    expect(phase?.kind === 'settled' && phase.violations.map((v) => v.id)).toEqual(['campaign-active']);
  });

  it('keeps the key after a send failure so a retry replays the same command', async () => {
    const r = rig();
    r.shell.data(two, 1);
    r.shell.replySubmit(new Error('boom'), submitted(pendingView('queued'))).replyAwait(settledOutcome('applied'));
    await r.controller.requestArchive('a');
    expect(r.controller.getState().archive['a']?.kind).toBe('send-failed');
    await r.controller.requestArchive('a');
    expect(r.shell.submits.map((s) => s.idempotencyKey)).toEqual(['key-1', 'key-1']);
  });

  it('never selects a row, nor moves the selection (rule 40)', async () => {
    const r = rig();
    r.shell.data(two, 1);
    r.controller.select('b');
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome('applied'));
    await r.controller.requestArchive('a');
    expect(r.controller.getState().selected).toBe('b');
    expect(r.contexts).toEqual(['b']);
  });

  it('shows the lifecycle states, with no concurrency wording', async () => {
    const r = rig();
    r.shell.data(two, 1);
    const seen: string[] = [];
    r.controller.subscribe((s) => {
      const p = s.archive['a'];
      if (p !== undefined) seen.push(p.kind === 'pending' ? p.state : p.kind === 'settled' ? p.status : p.kind);
    });
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome('applied'));
    await r.controller.requestArchive('a');
    expect(seen).toEqual(['sending', 'queued', 'applied']);
  });
});

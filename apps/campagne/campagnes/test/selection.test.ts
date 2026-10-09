import { describe, expect, it } from 'vitest';
import { pendingView, rig, row, settledOutcome, submitted } from './fakes';

const rows = [row('b', 'Beta'), row('a', 'Alpha')];

describe('selection and the output prop (SPEC rules 36–41)', () => {
  it('emits nothing at creation, nor when the first list arrives', () => {
    const r = rig();
    r.shell.data(rows, 1);
    expect(r.contexts).toEqual([]);
    expect(r.controller.getState().selected).toBeNull();
  });

  it('emits the id of the selected campaign, exactly as returned, once', () => {
    const r = rig();
    r.shell.data(rows, 1);
    r.controller.select('a');
    r.controller.select('a');
    expect(r.contexts).toEqual(['a']);
    expect(r.controller.getState().selected).toBe('a');
  });

  it('replaces the id when another campaign is selected', () => {
    const r = rig();
    r.shell.data(rows, 1);
    r.controller.select('a');
    r.controller.select('b');
    expect(r.contexts).toEqual(['a', 'b']);
    expect(r.controller.getState().selected).toBe('b');
  });

  it('clears the selection and emits null when the campaign leaves the list', () => {
    const r = rig();
    r.shell.data(rows, 1);
    r.controller.select('b');
    r.shell.data([row('a', 'Alpha')], 2);
    expect(r.contexts).toEqual(['b', null]);
    expect(r.controller.getState().selected).toBeNull();
  });

  it('keeps the selection across a refresh that still has the row', () => {
    const r = rig();
    r.shell.data(rows, 1);
    r.controller.select('b');
    r.shell.data([row('c', 'Gamma'), ...rows], 2);
    expect(r.contexts).toEqual(['b']);
    expect(r.controller.getState().selected).toBe('b');
  });

  it('does not clear the selection on a failed refresh', () => {
    const r = rig();
    r.shell.data(rows, 1);
    r.controller.select('b');
    r.shell.emit({ kind: 'stale', asOf: 1, wanted: 3 });
    expect(r.contexts).toEqual(['b']);
  });

  it('never emits an id the list did not return', () => {
    const r = rig();
    r.controller.select('ghost');
    r.shell.data(rows, 1);
    r.controller.select('ghost');
    expect(r.contexts).toEqual([]);
    expect(r.controller.getState().selected).toBeNull();
  });

  it('writes nothing: selecting calls no submit and no await (rule 41)', () => {
    const r = rig();
    r.shell.data(rows, 1);
    r.controller.select('a');
    expect(r.shell.submits).toHaveLength(0);
    expect(r.shell.awaits).toHaveLength(0);
  });

  it('selecting after an archive of another row still works', async () => {
    const r = rig();
    r.shell.data(rows, 1);
    r.shell.replySubmit(submitted(pendingView('queued'))).replyAwait(settledOutcome('applied'));
    await r.controller.requestArchive('a');
    r.controller.select('b');
    expect(r.contexts).toEqual(['b']);
  });

  it('emits nothing after dispose', () => {
    const r = rig();
    r.shell.data(rows, 1);
    r.controller.dispose();
    r.controller.select('a');
    expect(r.contexts).toEqual([]);
  });
});

import { TransportError, type WatchEvent } from '@dnd-helper/micro-ui-shell';
import { describe, expect, it } from 'vitest';
import { createPjsController } from '../src/controller';
import type { PcRow } from '../src/identifiers';
import { viewOf } from '../src/view-model';
import { FakeShell, flush, pendingView, row, settledOutcome, submitted } from './fake-shell';

function loaded(rows = [row('p1', 'Aria', 'Mage', 4), row('p2', 'Borek', 'Clerc', 3)]) {
  const shell = new FakeShell();
  const ctrl = createPjsController('camp-1', { shell, newKey: () => 'k' });
  shell.emit({ kind: 'data', data: rows, asOf: 7 });
  return { shell, ctrl };
}

describe('viewOf', () => {
  it('shows no campaign, no rows and no form before one is chosen', () => {
    const ctrl = createPjsController(null, { shell: new FakeShell() });
    expect(viewOf(ctrl.getState())).toMatchObject({ banner: 'idle', rows: [], add: null, edit: null });
  });

  it('shows loading until the first read answers', () => {
    const ctrl = createPjsController('camp-1', { shell: new FakeShell() });
    expect(viewOf(ctrl.getState())).toMatchObject({ banner: 'loading', rows: [] });
  });

  it('shows no form and no row action when the campaign is not found', () => {
    const { shell, ctrl } = loaded();
    shell.emit({ kind: 'not-found' });
    const v = viewOf(ctrl.getState());
    expect(v).toMatchObject({ banner: 'not-found', bannerText: 'Campagne introuvable.', rows: [], add: null, edit: null });
  });

  it('gives the three read states three different texts', () => {
    const texts = new Set<string | null>();
    const events: WatchEvent<PcRow[]>[] = [
      { kind: 'data', data: [], asOf: 1 },
      { kind: 'not-found' },
      { kind: 'error', error: new TransportError('network') },
    ];
    for (const ev of events) {
      const { shell, ctrl } = loaded();
      shell.emit(ev);
      texts.add(viewOf(ctrl.getState()).bannerText);
    }
    expect(texts.size).toBe(3);
  });

  it('keeps the rows in the order received, one per PC, with name, class and level', () => {
    const { ctrl } = loaded([row('b', 'Zed', 'Mage', 2), row('a', 'Aria', 'Moine', 9)]);
    expect(viewOf(ctrl.getState()).rows.map(({ id, name, class: c, level }) => [id, name, c, level])).toEqual([
      ['b', 'Zed', 'Mage', 2],
      ['a', 'Aria', 'Moine', 9],
    ]);
    expect(viewOf(ctrl.getState()).banner).toBe('none');
  });

  it('shows both values of a parked edit, with only the buttons the queue offers, and never yourValue in the list', async () => {
    const { shell, ctrl } = loaded();
    ctrl.openEdit('p1');
    ctrl.setEditField('niveau', '9');
    const view = pendingView('c1', 'awaiting_confirmation', {
      yourValue: { name: 'Aria', class: 'Mage', level: 9 },
      actions: ['confirm_overwrite', 'cancel'],
      projection: { pendingAhead: [{ value: { name: 'Aria', class: 'Mage', level: 6 } }] },
    });
    shell.submitQueue.push(submitted(view, 'c1'));
    void ctrl.submitEdit();
    await flush();
    const v = viewOf(ctrl.getState());
    expect(v.edit?.awaiting).toMatchObject({
      yourValue: [{ label: 'Nom', text: 'Aria' }, { label: 'Classe', text: 'Mage' }, { label: 'Niveau', text: '9' }],
      ahead: [{ label: 'Nom', text: 'Aria' }, { label: 'Classe', text: 'Mage' }, { label: 'Niveau', text: '6' }],
      canConfirm: true,
      canCancel: true,
    });
    expect(v.edit?.canSubmit).toBe(false);
    expect(v.rows.map((r) => r.level)).toEqual([4, 3]);
    expect(JSON.stringify(v.rows)).not.toContain('9');
  });

  it('shows whatever JSON the queue declared: a scalar as one line, an unknown key as itself, never a property of the label table', async () => {
    const { shell, ctrl } = loaded();
    ctrl.openEdit('p1');
    const view = pendingView('c1', 'awaiting_confirmation', {
      yourValue: 42,
      actions: ['cancel'],
      projection: { pendingAhead: [{ value: { name: 'Aria', constructor: 'x', toString: 3, note: { k: 1 } } }] },
    });
    shell.submitQueue.push(submitted(view, 'c1'));
    void ctrl.submitEdit();
    await flush();
    const awaiting = viewOf(ctrl.getState()).edit?.awaiting;
    expect(awaiting?.yourValue).toEqual([{ label: '', text: '42' }]);
    expect(awaiting?.ahead).toEqual([
      { label: 'Nom', text: 'Aria' },
      { label: 'constructor', text: 'x' },
      { label: 'toString', text: '3' },
      { label: 'note', text: '{"k":1}' },
    ]);
  });

  it('shows an expired edit with no confirm and a notice', async () => {
    const { shell, ctrl } = loaded();
    ctrl.openEdit('p1');
    shell.submitQueue.push(submitted(pendingView('c1', 'awaiting_confirmation', { yourValue: {}, actions: ['confirm_overwrite', 'cancel'] }), 'c1'));
    shell.script('c1', () => Promise.resolve(settledOutcome('c1', 'expired')));
    void ctrl.submitEdit();
    await flush();
    expect(viewOf(ctrl.getState()).edit).toMatchObject({ awaiting: null, notice: 'expired', canSubmit: true, canClose: true });
  });

  it('disables the row actions it must: archiving rows, and edit while a command of the edit form runs', async () => {
    const { shell, ctrl } = loaded();
    ctrl.openEdit('p1');
    shell.submitQueue.push(submitted(pendingView('c1', 'queued'), 'c1'));
    void ctrl.submitEdit();
    await flush();
    expect(viewOf(ctrl.getState()).rows.every((r) => !r.canEdit && r.canArchive)).toBe(true);
  });
});

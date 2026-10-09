import { describe, expect, it } from 'vitest';
import { ProtocolError, TransportError } from '@dnd-helper/micro-ui-shell';
import { row, rig } from './fakes';

describe('the list (SPEC rules 6–11, 31–34)', () => {
  it('opens exactly one watch, on campagne.listerCampagnes with no variables', () => {
    const { shell } = rig();
    expect(shell.watches).toEqual([{ capability: 'campagne.listerCampagnes', variables: {} }]);
  });

  it('starts loading, not empty and not failed', () => {
    expect(rig().controller.getState().list).toEqual({ kind: 'loading' });
  });

  it('shows the rows exactly as returned: same order, same names, duplicates kept', () => {
    const { shell, controller } = rig();
    const rows = [row('b', '  Les Mines  '), row('a', 'Les Mines'), row('c', '<b>Gras</b>'), row('d', 'é')];
    shell.data(rows, 4);
    const list = controller.getState().list;
    expect(list.kind).toBe('ready');
    if (list.kind === 'ready') {
      expect(list.rows).toEqual(rows);
      expect(list.rows.map((r) => r.name)).toEqual(['  Les Mines  ', 'Les Mines', '<b>Gras</b>', 'é']);
      expect(list.asOf).toBe(4);
    }
  });

  it('treats an empty result as a normal state, not a failure (rule 7)', () => {
    const { shell, controller } = rig();
    shell.data([], 1);
    expect(controller.getState().list).toEqual({ kind: 'ready', rows: [], asOf: 1, failure: null, stale: false });
  });

  it('shows an error before any data as a failure, never as an empty list (rule 8)', () => {
    const { shell, controller } = rig();
    shell.emit({ kind: 'error', error: new TransportError('network') });
    expect(controller.getState().list.kind).toBe('failed');
  });

  it('treats not-found like a failure', () => {
    const { shell, controller } = rig();
    shell.emit({ kind: 'not-found' });
    expect(controller.getState().list.kind).toBe('failed');
  });

  it('keeps the last good rows when a refresh fails (rule 34)', () => {
    const { shell, controller } = rig();
    shell.data([row('a', 'Alpha')], 2);
    shell.emit({ kind: 'error', error: new ProtocolError('read', 'bad') });
    const list = controller.getState().list;
    expect(list.kind === 'ready' && list.rows).toEqual([row('a', 'Alpha')]);
    expect(list.kind === 'ready' && list.failure).not.toBeNull();
  });

  it('clears the failure when the next read succeeds', () => {
    const { shell, controller } = rig();
    shell.data([row('a', 'Alpha')], 2);
    shell.emit({ kind: 'error', error: new TransportError('network') });
    shell.data([row('a', 'Alpha')], 2);
    const list = controller.getState().list;
    expect(list.kind === 'ready' && list.failure).toBeNull();
  });

  it('keeps the rows on a stale read and marks it', () => {
    const { shell, controller } = rig();
    shell.data([row('a', 'Alpha')], 2);
    shell.emit({ kind: 'stale', asOf: 2, wanted: 5 });
    const list = controller.getState().list;
    expect(list.kind === 'ready' && list.stale).toBe(true);
    expect(list.kind === 'ready' && list.rows).toEqual([row('a', 'Alpha')]);
  });

  it('ignores an event older than the one shown (rule 32)', () => {
    const { shell, controller } = rig();
    shell.data([row('a', 'Alpha'), row('b', 'Beta')], 6);
    shell.data([row('a', 'Alpha')], 5);
    const list = controller.getState().list;
    expect(list.kind === 'ready' && list.rows).toHaveLength(2);
    expect(list.kind === 'ready' && list.asOf).toBe(6);
  });

  it('accepts a higher version and an event with no version (A10), and the shown version never goes back', () => {
    const { shell, controller } = rig();
    shell.data([row('a', 'Alpha')], 3);
    shell.data([row('a', 'Alpha'), row('b', 'Beta')], 8);
    shell.data([row('b', 'Beta')], null);
    const list = controller.getState().list;
    expect(list.kind === 'ready' && list.rows).toEqual([row('b', 'Beta')]);
    expect(list.kind === 'ready' && list.asOf).toBe(8);
  });

  it('does not lose the typed text when the list refreshes', () => {
    const { shell, controller } = rig();
    controller.setName('  Brouillon ');
    shell.data([row('a', 'Alpha')], 1);
    shell.data([row('a', 'Alpha'), row('b', 'Beta')], 2);
    expect(controller.getState().create.name).toBe('  Brouillon ');
  });

  it('dispose unsubscribes and silences every later event', () => {
    const { shell, controller } = rig();
    shell.data([row('a', 'Alpha')], 1);
    const before = controller.getState();
    controller.dispose();
    expect(shell.unsubscribed).toBe(1);
    shell.data([], 2);
    expect(controller.getState()).toBe(before);
  });
});

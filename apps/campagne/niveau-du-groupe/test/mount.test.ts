import { describe, expect, it } from 'vitest';
import { mount } from '../src/mount';
import { COPY } from '../src/view-model';
import { FakeHost, data } from './fake-host';

// The smallest document the mount touches. There is no DOM test environment in this repository.
class FakeElement {
  readonly children: FakeElement[] = [];
  readonly dataset: Record<string, string> = {};
  readonly attributes: Record<string, string> = {};
  parent: FakeElement | null = null;
  writes = 0;
  private text = '';

  constructor(readonly tag: string) {}

  get textContent(): string {
    return this.text;
  }

  set textContent(value: string) {
    this.writes += 1;
    this.text = value;
  }

  get ownerDocument(): { createElement(tag: string): FakeElement } {
    return { createElement: (tag) => new FakeElement(tag) };
  }

  setAttribute(name: string, value: string): void {
    this.attributes[name] = value;
  }

  append(child: FakeElement): void {
    child.parent = this;
    this.children.push(child);
  }

  remove(): void {
    this.parent?.children.splice(this.parent.children.indexOf(this), 1);
    this.parent = null;
  }
}

// No argument: campaign 'A'. An explicit `undefined` stays undefined.
function setup(...args: [] | [string | undefined]) {
  const root = new FakeElement('div');
  const host = new FakeHost();
  const campagneId = args.length === 0 ? 'A' : args[0];
  const mounted = mount(root as unknown as HTMLElement, { campagneId }, { shell: host });
  const section = root.children[0]!;
  const slot = (name: string): FakeElement => section.children.find((c) => c.dataset['slot'] === name)!;
  return { root, host, mounted, section, slot };
}

describe('mount', () => {
  it('builds one section, shows loading first, and reads the campaign it was given', () => {
    const { root, host, section, slot } = setup();
    expect(root.children).toHaveLength(1);
    expect(section.dataset['microUi']).toBe('NiveauDuGroupe');
    expect(section.dataset['state']).toBe('loading');
    expect(slot('level').textContent).toBe(COPY.loading);
    expect(host.calls[0]).toMatchObject({ capability: 'campagne.niveauDuGroupe', variables: { campagneId: 'A' } });
  });

  it('renders the level and the count of a result', () => {
    const { host, section, slot } = setup();
    host.emit(0, data(4, 2, 5));
    expect(section.dataset['state']).toBe('level');
    expect([slot('level').textContent, slot('count').textContent, slot('note').textContent]).toEqual(['4', '2 PJ', '']);
  });

  it('renders "no party level" with a count of 0 when the level is null', () => {
    const { host, section, slot } = setup();
    host.emit(0, data(null, 0, 5));
    expect(section.dataset['state']).toBe('no-level');
    expect([slot('level').textContent, slot('count').textContent]).toEqual([COPY.noLevel, '0 PJ']);
  });

  it('marks a kept value as possibly stale after a failed read', () => {
    const { host, slot } = setup();
    host.emit(0, data(4, 2, 5));
    host.emit(0, { kind: 'stale', asOf: 5, wanted: 9 });
    expect([slot('level').textContent, slot('note').textContent]).toEqual(['4', COPY.stale]);
  });

  it('shows the unavailable state for not found and for a failure with no value, never a level', () => {
    const { host, section, slot } = setup();
    host.emit(0, { kind: 'not-found' });
    expect(section.dataset['state']).toBe('unavailable');
    expect([slot('level').textContent, slot('count').textContent]).toEqual([COPY.campaignUnavailable, '']);
  });

  it('without a campaign id: no read, a neutral state', () => {
    const { host, section, slot } = setup(undefined);
    expect(host.calls).toHaveLength(0);
    expect(section.dataset['state']).toBe('unavailable');
    expect(slot('level').textContent).toBe(COPY.campaignUnavailable);
  });

  it('writes a slot only when its text changed: an announcement that changes nothing rewrites nothing', () => {
    const { host, slot } = setup();
    host.emit(0, data(4, 2, 5));
    const writes = ['level', 'count', 'note'].map((n) => slot(n).writes);
    host.emit(0, data(4, 2, 6));
    host.emit(0, data(4, 2, 7));
    expect(['level', 'count', 'note'].map((n) => slot(n).writes)).toEqual(writes);
    host.emit(0, data(5, 2, 8));
    expect(slot('level').writes).toBe(writes[0]! + 1);
    expect(slot('count').writes).toBe(writes[1]);
  });

  it('update with another campaign drops the old value and reads the new one; the same one does nothing', () => {
    const { host, mounted, slot } = setup('A');
    host.emit(0, data(4, 2, 5));
    mounted.update({ campagneId: 'A' });
    expect(host.calls).toHaveLength(1);
    mounted.update({ campagneId: 'B' });
    expect(slot('level').textContent).toBe(COPY.loading);
    expect(host.calls[1]).toMatchObject({ variables: { campagneId: 'B' } });
    host.emit(0, data(9, 9, 99));
    expect(slot('level').textContent).toBe(COPY.loading);
  });

  it('unmount releases the watch, removes the section and ignores what comes later', () => {
    const { root, host, mounted } = setup();
    mounted.unmount();
    expect(host.active).toHaveLength(0);
    expect(root.children).toHaveLength(0);
    host.emit(0, data(4, 2, 5));
    mounted.update({ campagneId: 'B' });
    expect(host.calls).toHaveLength(1);
    expect(root.children).toHaveLength(0);
  });
});

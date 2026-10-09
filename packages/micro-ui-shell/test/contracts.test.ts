import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { COMMAND_ACTIONS, G_FIELDS, H_FIELDS, MESSAGE_KINDS, QUEUE_STATES, TERMINAL_STATUSES } from '../src/contracts';

const schema = (name: string): Record<string, any> =>
  JSON.parse(readFileSync(new URL(`../../../contracts/${name}`, import.meta.url), 'utf8')) as Record<string, any>;

const propertyNames = (o: unknown): string[] => Object.keys(o as object);

describe('the TS mirrors equal the contract schemas (rule 11)', () => {
  const h = schema('h-command-result.schema.json');
  const g = schema('g-queue-entry.schema.json');
  const l = schema('l-message.schema.json');

  it('H: terminal statuses and fields', () => {
    expect([...TERMINAL_STATUSES]).toEqual(h.properties.status.enum);
    expect([...H_FIELDS]).toEqual(propertyNames(h.properties));
    expect([...H_FIELDS]).toEqual(h.required);
    expect(h.additionalProperties).toBe(false);
  });

  it('G: states and fields', () => {
    expect([...QUEUE_STATES]).toEqual(g.properties.state.enum);
    expect([...G_FIELDS]).toEqual(propertyNames(g.properties));
    expect(g.additionalProperties).toBe(false);
  });

  it('L: message kinds and actions', () => {
    expect([...MESSAGE_KINDS]).toEqual(l.properties.message.enum);
    expect([...COMMAND_ACTIONS]).toEqual(l.properties.actions.items.enum);
    expect(l.additionalProperties).toBe(true);
  });
});

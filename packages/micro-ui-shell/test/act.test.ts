import { describe, expect, it } from 'vitest';
import { ActionNotOfferedError, TransportError } from '../src/index';
import { CMD, harness, json, notFound, pending, settled } from './fakes';

const ahead = { message: 'ValueDeclaredAhead', to: ['gm'], actions: ['confirm_overwrite', 'cancel', 'edit'] };

describe('act on a command by id (rules 35–36)', () => {
  it('refuses locally when the command was never seen', async () => {
    const h = harness();
    await expect(h.client.act(CMD, 'confirm_overwrite')).rejects.toThrow(ActionNotOfferedError);
    expect(h.fetch.calls).toHaveLength(0);
  });

  it('refuses locally an action the latest message did not offer', async () => {
    const h = harness();
    h.fetch.enqueue(pending('queued', [{ message: 'CommandQueued', to: ['gm'] }]));
    await h.client.lookup(CMD);
    await expect(h.client.act(CMD, 'cancel')).rejects.toThrow(ActionNotOfferedError);
    expect(h.fetch.calls).toHaveLength(1);
  });

  it('sends confirm_overwrite to /confirm only once offered, with no body', async () => {
    const h = harness();
    h.fetch.enqueue(pending('awaiting_confirmation', [ahead]), pending('confirmed', [ahead]));
    await h.client.lookup(CMD);
    const view = await h.client.act(CMD, 'confirm_overwrite');
    const call = h.fetch.calls[1]!;
    expect(call).toMatchObject({ method: 'POST', url: `http://127.0.0.1:7878/commands/${CMD}/confirm`, body: undefined });
    expect(view).toMatchObject({ kind: 'pending', state: 'confirmed' });
  });

  it('cancel goes to /cancel, and the settled answer is final', async () => {
    const h = harness();
    h.fetch.enqueue(pending('awaiting_confirmation', [ahead]), settled('cancelled'));
    await h.client.lookup(CMD);
    expect(await h.client.act(CMD, 'cancel')).toMatchObject({ kind: 'settled', result: { status: 'cancelled' } });
    expect(h.fetch.calls[1]!.url).toBe(`http://127.0.0.1:7878/commands/${CMD}/cancel`);
    expect(h.client.knownDataVersion()).toBeNull();
  });

  it('an applied answer from a confirm raises the known version', async () => {
    const h = harness();
    h.fetch.enqueue(pending('awaiting_confirmation', [ahead]), settled('applied', { dataVersion: 21 }));
    await h.client.lookup(CMD);
    await h.client.act(CMD, 'confirm_overwrite');
    expect(h.client.knownDataVersion()).toBe(21);
  });

  it('a 403 / 404 / 500 is distinct: not-author is a transport error, not-found its own outcome', async () => {
    const h = harness();
    h.fetch.enqueue(pending('awaiting_confirmation', [ahead]), json({ error: 'not-author' }, 403), notFound());
    await h.client.lookup(CMD);
    await expect(h.client.act(CMD, 'cancel')).rejects.toMatchObject({ kind: 'http', status: 403, errorId: 'not-author' });
    expect(await h.client.act(CMD, 'cancel')).toEqual({ kind: 'not-found', commandId: CMD });
    expect(TransportError).toBeDefined();
  });

  it('a route the server has not added yet is a transport error, not a not-found', async () => {
    const h = harness();
    h.fetch.enqueue(pending('awaiting_confirmation', [ahead]), new Response('Not Found', { status: 404 }));
    await h.client.lookup(CMD);
    await expect(h.client.act(CMD, 'confirm_overwrite')).rejects.toMatchObject({ kind: 'http', status: 404 });
  });

  it('exposes no other action (type level)', () => {
    const h = harness();
    // @ts-expect-error `approve` is not an action the shell can send
    void h.client.act(CMD, 'approve').catch(() => undefined);
    // @ts-expect-error nor `edit`
    void h.client.act(CMD, 'edit').catch(() => undefined);
  });
});

import { describe, expect, it } from 'vitest';
import { assertLoopbackBaseUrl } from '../src/base-url';
import { InvalidIdentifierError, NonLoopbackBaseUrlError } from '../src/index';
import { assertIdentifier, wireDataCapability } from '../src/identifier';
import { createShellClient } from '../src/index';
import { BASE, FakeFetch, fakeEventSourceFactory, harness } from './fakes';

describe('identifiers (rule 13)', () => {
  it.each(['', 'campagne.', 'lister-campagnes', 'campagne.lister-pjs', 'Campagne.x', 'campagne.x@1', '.x', 'a.1b'])(
    'refuses %j and names it',
    (id) => {
      expect(() => assertIdentifier(id)).toThrow(InvalidIdentifierError);
      expect(() => assertIdentifier(id)).toThrow(`"${id}"`);
    },
  );

  it.each(['campagne.listerPjs', 'campagne.niveauDuGroupe', 'a-b.C'])('accepts %j', (id) => {
    expect(() => assertIdentifier(id)).not.toThrow();
  });

  it('builds the wire id with the version', () => {
    expect(wireDataCapability('campagne.modifierPJ', 1)).toBe('campagne.modifierPJ@1');
  });

  it.each([0, 1.5, NaN, -1])('refuses version %s', (v) => {
    expect(() => wireDataCapability('campagne.modifierPJ', v)).toThrow(InvalidIdentifierError);
  });

  it('refuses a malformed identifier before any network call', async () => {
    const h = harness();
    await expect(h.client.read('lister-campagnes')).rejects.toThrow(InvalidIdentifierError);
    expect(() => h.client.watch('Campagne.x', {}, () => undefined)).toThrow(InvalidIdentifierError);
    await expect(
      h.client.submit({ dataCapability: 'x', version: 1, mode: 'relative', payload: {} }),
    ).rejects.toThrow(InvalidIdentifierError);
    expect(h.fetch.calls).toHaveLength(0);
    expect(h.sources.instances).toHaveLength(0);
  });
});

describe('base URL (rule 10)', () => {
  it.each(['http://192.168.1.2:7878', 'http://example.com', 'http://127.0.0.2.nip.io', 'ftp://127.0.0.1', 'nonsense', '', 'http://user:pw@127.0.0.1'])(
    'refuses %j',
    (url) => {
      expect(() => assertLoopbackBaseUrl(url)).toThrow(NonLoopbackBaseUrlError);
    },
  );

  it.each([
    ['http://127.0.0.1:7878', 'http://127.0.0.1:7878'],
    ['http://localhost:3000/', 'http://localhost:3000'],
    ['http://[::1]:8080', 'http://[::1]:8080'],
  ])('accepts %j', (url, normalised) => {
    expect(assertLoopbackBaseUrl(url)).toBe(normalised);
  });

  it('is refused at construction, with no call', () => {
    const fetch = new FakeFetch();
    const sources = fakeEventSourceFactory();
    expect(() =>
      createShellClient({ baseUrl: 'http://10.0.0.5', fetch: fetch.fetch, eventSource: sources.create }),
    ).toThrow(NonLoopbackBaseUrlError);
    expect(fetch.calls).toHaveLength(0);
    expect(sources.instances).toHaveLength(0);
    expect(BASE).toContain('127.0.0.1');
  });
});

// The product's shell client over the mock server: made once per page.
import { createShellClient } from '@dnd-helper/micro-ui-shell';
import { mockServer, shellOnce } from './server.js';

export const shell = () => shellOnce((server) => createShellClient({ baseUrl: 'http://127.0.0.1:7878', fetch: server.fetch, eventSource: server.eventSource }));

/** Alone on a preview, with no campaign chosen: the newest active campaign. */
export async function defaultCampagne() {
  const server = mockServer();
  const first = (await server.load('campagnes')).filter((c) => !c.archived).sort((a, b) => b.createdAt - a.createdAt)[0];
  return first ? first.id : null;
}

import { NonLoopbackBaseUrlError } from './errors';

const LOOPBACK_HOSTS = new Set(['127.0.0.1', 'localhost', '[::1]']);

/** Rule 10: no campaign data leaves the machine. Returns the URL without a trailing `/`. */
export function assertLoopbackBaseUrl(raw: string): string {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw new NonLoopbackBaseUrlError(raw);
  }
  if (
    (url.protocol !== 'http:' && url.protocol !== 'https:') ||
    !LOOPBACK_HOSTS.has(url.hostname) ||
    url.username !== '' ||
    url.password !== ''
  ) {
    throw new NonLoopbackBaseUrlError(raw);
  }
  return `${url.origin}${url.pathname}`.replace(/\/+$/, '');
}

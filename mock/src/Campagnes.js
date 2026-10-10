// The real Campagnes Micro-UI on the mock server; it sets the screen's `campagne`.
import { mountCampagnes } from '../../apps/campagne/campagnes/src/index.ts';
import { mountPoint, setContext } from './screen.js';
import { shell } from './shell.js';

const { host } = mountPoint();
mountCampagnes(host, { shell: shell(), setContext, confirm: (message) => window.confirm(message) });

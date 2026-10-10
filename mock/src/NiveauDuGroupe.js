// The real NiveauDuGroupe Micro-UI on the mock server; follows the screen's campaign (`$ctx.campagne`).
import { mount } from '../../apps/campagne/niveau-du-groupe/src/index.ts';
import { compose, mountPoint, resolveProps, setContext } from './screen.js';
import { defaultCampagne, shell } from './shell.js';

const { host, props } = mountPoint();
(async () => {
  // Alone on a preview, with no campaign chosen yet: the newest one.
  if (resolveProps(props).campagneId === undefined) setContext('campagne', await defaultCampagne());
  compose(props, (p) => mount(host, { campagneId: p.campagneId === null ? undefined : p.campagneId }, { shell: shell() }));
})();

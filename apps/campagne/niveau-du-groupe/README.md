# NiveauDuGroupe

The party level of one campaign at a glance: the level and the number of active
PCs, or « Pas de niveau de groupe » when the campaign has none. It is read-only:
it has no `actions`, no list, no form and no PC-level control.

`micro-ui.json` is its contract (D): `system: campagne`, `needs:
["campagne.niveauDuGroupe"]`, `props: { campagneId: "ID" }`.

## Use

```ts
import { createShellClient } from '@dnd-helper/micro-ui-shell';
import { mount } from '@dnd-helper/ui-campagne-niveau-du-groupe';

const shell = createShellClient({ /* the page's single client */ });
const niveau = mount(element, { campagneId }, { shell });
niveau.update({ campagneId: autre }); // drops the old value at once, reads the new campaign
niveau.unmount();                      // releases the subscription; a pending read has no effect
```

`shell` is the page's one `ShellClient`; this Micro-UI never creates one and calls
only `shell.watch`. It reads `campagne.niveauDuGroupe` by identifier and nothing
else, so it imports no Capability, DataCapability or Data-layer code.

## What it does, and what the shell does for it

| | |
|---|---|
| Re-read on `dataVersion` | `ShellClient.watch` refetches on every announcement, coalesces a burst into one read at the highest version and never hands over an answer older than the announced version. This unit adds none of that. |
| Out of order | The result with the highest `asOf` stays; a lower one is ignored. |
| Campaign switch | The old value goes at once; a late answer for the old id is discarded. |
| Validation | A result whose `level` is not `null` or an integer 1–20, whose `pcCount` is not a non-negative integer, whose `level` is `null` while `pcCount > 0` (or the reverse), or whose `model` is not `v1`, counts as a failed read. Nothing is rounded or recomputed. |
| Storage | None. Nothing is cached across campaigns; the value is kept per `campagneId` and dropped on a switch. |

## States

| State | Shown |
|---|---|
| no `campagneId`, or unknown / archived / foreign campaign | « Campagne indisponible » — one answer, whatever the reason; no read is made without an id |
| first read pending | « Chargement… » |
| PCs | the level, « n PJ » |
| no active PC (`level === null`) | « Pas de niveau de groupe », « 0 PJ » |
| failed read, a value is known | that value, with « peut-être pas à jour » |
| failed read, no value | « Niveau du groupe indisponible » (after a not-found answer the not-found state stays) |
| the shell cannot start the read (disposed) | « Niveau du groupe indisponible »; nothing throws into the page |

It never shows `0` where the level should be. It recovers by itself on the next
`dataVersion` announcement.

## Layout

`controller.ts` is the state machine over an injected host (no DOM), `view-model.ts`
the pure state → copy function, `mount.ts` the only file that touches the DOM.
Tests are hermetic: a fake host at the shell interface, and one suite that drives
the real shell on fake `fetch`, `EventSource` and clock.

## Not served yet

The live read needs the server's `POST /capabilities/{identifier}` route
(`packages/micro-ui-shell/PROTOCOL.md`, "server to conform"). Until it exists the
shell gets a transport error and this view shows « Niveau du groupe indisponible ».
Serving it, and composing this Micro-UI into the campaign screen, are not part of
this unit.

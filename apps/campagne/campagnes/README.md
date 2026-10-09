# Campagnes (Micro-UI)

Lets the GM see the active campaigns, create one by name, archive one, and pick
the campaign the rest of the screen works on. System `campagne`, manifest
[`micro-ui.json`](micro-ui.json) (contract D).

It reads and writes **by identifier only**, through the shell
(`@dnd-helper/micro-ui-shell`, `packages/micro-ui-shell`):

| | Identifier |
|---|---|
| needs | `campagne.listerCampagnes` |
| actions | `campagne.creerCampagne`, `campagne.archiverCampagne` |

It imports no Capability, DataCapability or crate, runs no business rule
(no trim, no length, no emptiness check on the name: the DataGuard decides and
the UI shows the violation it returns), and never re-sorts or filters the list.

## Layout

- `src/controller.ts` — all the behaviour, with no DOM. Tested in node.
- `src/view.ts` — `mountCampagnes(root, deps)`, the thin DOM binding. Text only
  through `textContent`.
- `src/identifiers.ts`, `src/violations.ts` — the three identifiers, and the
  French copy for the violation ids (the ids themselves stay untranslated).

## Mount

```ts
import { createShellClient } from '@dnd-helper/micro-ui-shell';
import { mountCampagnes } from '@dnd-helper/ui-campagnes';

const shell = createShellClient({ baseUrl, fetch, eventSource });
const mounted = mountCampagnes(rootElement, {
  shell,
  setContext, // supplied by the host, see below
  // confirm: optional, defaults to window.confirm
});
mounted.unmount();
```

## The output prop

`props: { "campagneId": "ID" }` declares the emitted campaign id. The Micro-UI
writes it by calling the host's `setContext('campagne', id | null)`:

- `id` when the GM selects a campaign, exactly as `campagne.listerCampagnes`
  returned it;
- `null` when the selected campaign leaves the list (archived here or
  elsewhere);
- nothing at mount.

The screen composition (#27) wires it as `{"Campagnes": {"campagneId": "$ctx.campagne"}}`.

## Known gaps, owned elsewhere

- The shell has no `setContext` yet (the owner decision recorded in #27 expects
  one). This unit takes it as an injected dependency with that exact signature,
  so the host can pass the shell's once it exists, or its own.
- The local server does not serve `POST /capabilities/{identifier}` yet
  (`crates/campagne/serveur/src/http.rs`). Until it does, the list read fails
  and the UI says so, instead of showing an empty list.
- There is no bundler or page server here: the unit is exported as TypeScript
  source and composed and served by #27.

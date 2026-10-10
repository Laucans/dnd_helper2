# The mock-up

The product's frontend with no backend: the frozen screens as pages under
`pages/`, each a static HTML page plus the **micro-frontends** it injects
(`<div data-component="<Name>">`, filled by `mock.js`). A micro-frontend here
is the **real Micro-UI** of `apps/` (its `source` in `pages.json`), bundled
by `build.sh` (esbuild through `npx`) from `src/<Name>.js` into
`components/<Name>.js`, mounted on the real shell client (`src/shell.js`)
over the **mock server** the harness installs (`src/server.js`): the shell's
wire protocol answered in memory, reads from `src/reads.js`, commands judged
and applied by `src/commands.js`, `dataVersion` bumped, so the page behaves
as the product would. The screen host (`src/screen.js`, installed too)
carries the screen's context: a page injects `Pjs` with
`data-props='{"campagneId": "$ctx.campagne"}'` and `Campagnes` sets it. The
bundles are committed with what they were built from (`builtFrom`): the
mock-up needs no build to be viewed, and the gates refuse a stale bundle.

`pages.json` (contract N) lists the pages and the micro-frontends; the
`pages-mocked` gate checks it. After a change under `apps/` or `src/`, run
`mock/build.sh` and commit the bundles.

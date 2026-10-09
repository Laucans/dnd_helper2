// The DOM binding of the controller, and nothing else: no rule, no state of its
// own beyond what the DOM holds. Every string reaches the page through
// `textContent`, so a campaign name is always plain text (SPEC rule 10).

import { createCampagnesController, type CampagnesController, type CampagnesDeps, type CampagnesState, type CommandPhase } from './controller';
import { STATE_LABELS, pendingLabel } from './violations';

export interface MountedCampagnes {
  unmount(): void;
}

type MountDeps = Omit<CampagnesDeps, 'confirm'> & { confirm?: CampagnesDeps['confirm'] };

function el<K extends keyof HTMLElementTagNameMap>(doc: Document, tag: K, attrs: Record<string, string> = {}, text?: string): HTMLElementTagNameMap[K] {
  const node = doc.createElement(tag);
  for (const [name, value] of Object.entries(attrs)) node.setAttribute(name, value);
  if (text !== undefined) node.textContent = text;
  return node;
}

/** The line shown for a command: its state, and the violation ids when it was refused (rule 27). */
export function describePhase(phase: CommandPhase): string | null {
  switch (phase.kind) {
    case 'idle':
      return null;
    case 'sending':
      return 'Envoi…';
    case 'pending':
      return pendingLabel(phase.state);
    case 'settled': {
      const label = STATE_LABELS[phase.status];
      const ids = phase.violations.flatMap((v) => (v.id === null ? [] : [v.id]));
      return ids.length === 0 ? label : `${label} : ${ids.join(', ')}`;
    }
    case 'send-failed':
      return phase.message;
    case 'lost':
      return 'Commande introuvable côté serveur.';
  }
}

const busy = (phase: CommandPhase | undefined): boolean => phase?.kind === 'sending' || phase?.kind === 'pending';

export function mountCampagnes(root: HTMLElement, deps: MountDeps): MountedCampagnes {
  const doc = root.ownerDocument;
  const confirm = deps.confirm ?? ((message: string): boolean => doc.defaultView?.confirm(message) ?? false);
  const controller: CampagnesController = createCampagnesController({ ...deps, confirm });

  // The skeleton is built once: a refresh never recreates the input, so typed text survives it.
  const form = el(doc, 'form', { 'data-slot': 'create', novalidate: '' });
  const label = el(doc, 'label', { for: 'campagnes-name' }, 'Nom de la campagne');
  const input = el(doc, 'input', { id: 'campagnes-name', type: 'text', name: 'name', autocomplete: 'off', 'aria-describedby': 'campagnes-name-errors' });
  const fieldErrors = el(doc, 'div', { id: 'campagnes-name-errors', 'data-slot': 'name-errors', role: 'alert' });
  const submit = el(doc, 'button', { type: 'submit' }, 'Créer');
  const formErrors = el(doc, 'div', { 'data-slot': 'form-errors', role: 'alert' });
  const status = el(doc, 'p', { 'data-slot': 'status', role: 'status' });
  form.append(label, input, fieldErrors, submit, formErrors, status);

  const listStatus = el(doc, 'p', { 'data-slot': 'list-status', role: 'status' });
  const list = el(doc, 'ul', { 'data-slot': 'list' });
  root.replaceChildren(form, listStatus, list);

  function paragraphs(messages: readonly string[]): HTMLElement[] {
    return messages.map((message) => el(doc, 'p', {}, message));
  }

  function renderForm(state: CampagnesState): void {
    const { create } = state;
    // Only a cleared name is pushed into the input: the GM's own typing is never overwritten.
    if (create.name === '' && input.value !== '') input.value = '';
    submit.disabled = busy(create.phase);
    input.setAttribute('aria-invalid', create.fieldErrors.name.length > 0 ? 'true' : 'false');
    fieldErrors.replaceChildren(...paragraphs(create.fieldErrors.name));
    formErrors.replaceChildren(...paragraphs(create.formErrors));
    status.textContent = describePhase(create.phase) ?? '';
  }

  function renderList(state: CampagnesState): void {
    const view = state.list;
    if (view.kind === 'loading') {
      listStatus.textContent = 'Chargement des campagnes…';
      list.replaceChildren();
      return;
    }
    if (view.kind === 'failed') {
      listStatus.textContent = view.message;
      list.replaceChildren();
      return;
    }
    const notes = [view.failure, view.stale ? 'La liste peut ne pas être à jour.' : null].filter((n): n is string => n !== null);
    listStatus.textContent = notes.length > 0 ? notes.join(' ') : view.rows.length === 0 ? 'Aucune campagne active.' : '';
    // Rebuilding the rows must not drop the keyboard focus.
    const active = doc.activeElement;
    const focusedId = active !== null && list.contains(active) ? active.closest('li')?.getAttribute('data-id') : undefined;
    const focusedAction = active?.getAttribute('data-action');
    list.replaceChildren(
      ...view.rows.map((row) => {
        const item = el(doc, 'li', { 'data-id': row.id });
        const pick = el(doc, 'button', { type: 'button', 'data-action': 'select', 'aria-pressed': state.selected === row.id ? 'true' : 'false' }, row.name);
        const archive = el(doc, 'button', { type: 'button', 'data-action': 'archive', 'aria-label': `Archiver ${row.name}` }, 'Archiver');
        archive.disabled = busy(state.archive[row.id]);
        const phase = state.archive[row.id];
        const line = el(doc, 'span', { 'data-slot': 'row-status', role: 'status' }, (phase === undefined ? null : describePhase(phase)) ?? '');
        item.append(pick, archive, line);
        return item;
      }),
    );
    if (focusedId !== undefined && focusedId !== null && focusedAction !== null && focusedAction !== undefined) {
      list.querySelector<HTMLElement>(`li[data-id="${CSS.escape(focusedId)}"] button[data-action="${focusedAction}"]`)?.focus();
    }
  }

  let shown: CampagnesState | null = null;
  function render(state: CampagnesState): void {
    renderForm(state);
    // Typing changes only the form: the list is rebuilt when it, the selection or an archive status changed.
    if (shown === null || shown.list !== state.list || shown.selected !== state.selected || shown.archive !== state.archive) renderList(state);
    shown = state;
  }

  const onInput = (): void => {
    controller.setName(input.value);
  };
  const onSubmit = (event: Event): void => {
    event.preventDefault();
    void controller.submitCreate();
  };
  const onClick = (event: Event): void => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const action = target.closest<HTMLElement>('button[data-action]');
    const id = action?.closest<HTMLElement>('li[data-id]')?.dataset['id'];
    if (action === null || action === undefined || id === undefined) return;
    if (action.dataset['action'] === 'select') controller.select(id);
    else if (action.dataset['action'] === 'archive') void controller.requestArchive(id);
  };

  input.addEventListener('input', onInput);
  form.addEventListener('submit', onSubmit);
  list.addEventListener('click', onClick);
  const unsubscribe = controller.subscribe(render);
  render(controller.getState());

  return {
    unmount(): void {
      unsubscribe();
      input.removeEventListener('input', onInput);
      form.removeEventListener('submit', onSubmit);
      list.removeEventListener('click', onClick);
      controller.dispose();
      root.replaceChildren();
    },
  };
}

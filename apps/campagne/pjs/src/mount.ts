// The only file that touches the document. It paints `viewOf(state)` and
// sends gestures back to the controller. Text goes in through `textContent`
// only, so a name typed by the GM is never parsed as HTML. The inputs live as
// long as the page, so typing never loses focus.

import { createPjsController, type PjsDeps, type Violation } from './controller';
import { COPY } from './copy';
import { viewOf, type AwaitingView, type FormView, type PjsView, type ValueLine } from './view-model';
import { PC_FIELDS, type PcField } from './violations';

export interface PjsProps {
  campagneId: string | null;
}

export interface MountedPjs {
  update(props: PjsProps): void;
  unmount(): void;
}

type Attrs = Partial<Record<'class' | 'type' | 'name' | 'for' | 'id' | 'inputmode' | 'autocomplete' | 'role', string>>;

function el<K extends keyof HTMLElementTagNameMap>(
  doc: Document,
  tag: K,
  attrs: Attrs = {},
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = doc.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, v);
  if (text !== undefined) node.textContent = text;
  return node;
}

interface FormDom {
  root: HTMLFormElement;
  inputs: Record<PcField, HTMLInputElement>;
  fieldErrors: Record<PcField, HTMLUListElement>;
  formErrors: HTMLUListElement;
  status: HTMLParagraphElement;
  submit: HTMLButtonElement;
}

function buildForm(doc: Document, scope: 'add' | 'edit', title: string, submitLabel: string): FormDom {
  const root = el(doc, 'form', { class: `pjs-form pjs-form-${scope}` });
  root.noValidate = true;
  root.append(el(doc, 'h3', {}, title));
  const inputs = {} as Record<PcField, HTMLInputElement>;
  const fieldErrors = {} as Record<PcField, HTMLUListElement>;
  for (const field of PC_FIELDS) {
    const id = `pjs-${scope}-${field}`;
    const wrap = el(doc, 'div', { class: 'pjs-field' });
    const input = el(doc, 'input', { type: 'text', name: field, id, autocomplete: 'off' });
    if (field === 'niveau') input.setAttribute('inputmode', 'numeric');
    const errors = el(doc, 'ul', { class: 'pjs-errors', role: 'alert' });
    wrap.append(el(doc, 'label', { for: id }, COPY.labels[field]), input, errors);
    root.append(wrap);
    inputs[field] = input;
    fieldErrors[field] = errors;
  }
  const formErrors = el(doc, 'ul', { class: 'pjs-errors pjs-form-errors', role: 'alert' });
  const status = el(doc, 'p', { class: 'pjs-status' });
  const submit = el(doc, 'button', { type: 'submit' }, submitLabel);
  root.append(formErrors, status, submit);
  return { root, inputs, fieldErrors, formErrors, status, submit };
}

function paintViolations(ul: HTMLUListElement, notes: readonly Violation[]): void {
  const doc = ul.ownerDocument;
  ul.replaceChildren(
    ...notes.map((n) => {
      const li = el(doc, 'li', {}, n.text);
      if (n.id !== null) li.dataset['violation'] = n.id;
      return li;
    }),
  );
}

function paintForm(dom: FormDom, v: FormView | null): void {
  dom.root.hidden = v === null;
  if (v === null) return;
  for (const field of PC_FIELDS) {
    const input = dom.inputs[field];
    // Only when it differs: setting the value of a focused input moves the caret.
    if (input.value !== v.values[field]) input.value = v.values[field];
    input.disabled = v.busy;
    input.setAttribute('aria-invalid', v.fieldErrors[field].length > 0 ? 'true' : 'false');
    paintViolations(dom.fieldErrors[field], v.fieldErrors[field]);
  }
  paintViolations(dom.formErrors, v.formErrors);
  dom.submit.disabled = !v.canSubmit;
  dom.status.textContent = v.busy && v.awaiting === null ? COPY.sending : (noticeText(v) ?? '');
}

function noticeText(v: FormView): string | null {
  if (v.transportError) return COPY.transport;
  if (v.notice === 'expired') return COPY.expired;
  if (v.notice === 'cancelled') return COPY.cancelled;
  return null;
}

function paintValueLines(ul: HTMLUListElement, lines: readonly ValueLine[]): void {
  const doc = ul.ownerDocument;
  ul.replaceChildren(...lines.map((l) => el(doc, 'li', {}, l.label === '' ? l.text : `${l.label} : ${l.text}`)));
}

export function mount(host: HTMLElement, props: PjsProps, deps: PjsDeps): MountedPjs {
  const doc = host.ownerDocument;
  const ctrl = createPjsController(props.campagneId, deps);

  const root = el(doc, 'section', { class: 'pjs' });
  const banner = el(doc, 'p', { class: 'pjs-banner', role: 'status' });
  const list = el(doc, 'ul', { class: 'pjs-rows' });
  const add = buildForm(doc, 'add', COPY.addTitle, COPY.add);
  const edit = buildForm(doc, 'edit', COPY.editTitle, COPY.save);

  // The parked-edit panel: the GM's value beside the one already applied.
  const awaiting = el(doc, 'div', { class: 'pjs-awaiting', role: 'alert' });
  const awaitingTitle = el(doc, 'p', {}, COPY.awaitingTitle);
  const yours = el(doc, 'ul', { class: 'pjs-yours' });
  const ahead = el(doc, 'ul', { class: 'pjs-ahead' });
  const confirm = el(doc, 'button', { type: 'button' }, COPY.confirm);
  const cancel = el(doc, 'button', { type: 'button' }, COPY.cancel);
  awaiting.append(awaitingTitle, el(doc, 'h4', {}, COPY.yourValue), yours, el(doc, 'h4', {}, COPY.aheadValue), ahead, confirm, cancel);
  const close = el(doc, 'button', { type: 'button' }, COPY.close);
  edit.root.insertBefore(awaiting, edit.formErrors);
  edit.root.append(close);

  root.append(el(doc, 'h2', {}, COPY.title), banner, list, add.root, edit.root);
  host.replaceChildren(root);

  for (const field of PC_FIELDS) {
    add.inputs[field].addEventListener('input', () => {
      ctrl.setAddField(field, add.inputs[field].value);
    });
    edit.inputs[field].addEventListener('input', () => {
      ctrl.setEditField(field, edit.inputs[field].value);
    });
  }
  add.root.addEventListener('submit', (e) => {
    e.preventDefault();
    void ctrl.submitAdd();
  });
  edit.root.addEventListener('submit', (e) => {
    e.preventDefault();
    void ctrl.submitEdit();
  });
  confirm.addEventListener('click', () => void ctrl.confirmEdit());
  cancel.addEventListener('click', () => void ctrl.cancelEdit());
  close.addEventListener('click', () => {
    ctrl.closeEdit();
  });

  function paintRows(view: PjsView): void {
    list.replaceChildren(
      ...view.rows.map((r) => {
        const li = el(doc, 'li', { class: 'pjs-row' });
        li.dataset['pc'] = r.id;
        li.append(el(doc, 'span', { class: 'pjs-name' }, r.name), el(doc, 'span', { class: 'pjs-class' }, r.class), el(doc, 'span', { class: 'pjs-level' }, String(r.level)));
        const editBtn = el(doc, 'button', { type: 'button' }, COPY.edit);
        editBtn.disabled = !r.canEdit;
        editBtn.addEventListener('click', () => {
          ctrl.openEdit(r.id);
        });
        const archiveBtn = el(doc, 'button', { type: 'button' }, r.archiving ? COPY.archiving : COPY.archive);
        archiveBtn.disabled = !r.canArchive;
        archiveBtn.addEventListener('click', () => void ctrl.archive(r.id));
        li.append(editBtn, archiveBtn, ...r.notes.map((n) => el(doc, 'span', { class: 'pjs-note' }, n)));
        return li;
      }),
    );
  }

  function paintAwaiting(a: AwaitingView | null): void {
    awaiting.hidden = a === null;
    if (a === null) {
      // The unconfirmed value does not linger in the page once the panel is gone.
      yours.replaceChildren();
      ahead.replaceChildren();
      return;
    }
    paintValueLines(yours, a.yourValue);
    paintValueLines(ahead, a.ahead ?? []);
    confirm.hidden = !a.canConfirm;
    cancel.hidden = !a.canCancel;
  }

  function render(view: PjsView): void {
    banner.hidden = view.bannerText === null;
    banner.textContent = view.bannerText ?? '';
    banner.dataset['banner'] = view.banner;
    paintRows(view);
    paintForm(add, view.add);
    paintForm(edit, view.edit);
    paintAwaiting(view.edit?.awaiting ?? null);
    close.hidden = view.edit === null || !view.edit.canClose;
  }

  const off = ctrl.subscribe((s) => {
    render(viewOf(s));
  });
  render(viewOf(ctrl.getState()));

  return {
    update(next) {
      ctrl.setCampagne(next.campagneId);
    },
    unmount() {
      off();
      ctrl.dispose();
      host.replaceChildren();
    },
  };
}

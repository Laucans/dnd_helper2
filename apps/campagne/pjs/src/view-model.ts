// What the page shows, as plain data: the DOM layer only paints it. The list
// is the controller's rows in their order; nothing here sorts, filters or
// judges a value.

import type { JsonValue } from '@dnd-helper/micro-ui-shell';
import { COPY } from './copy';
import { isBusy, type EditState, type FormState, type PjsState, type RowNotice, type Violation } from './controller';
import { PC_FIELDS, type PcField, type PcFormValues } from './violations';

export type Banner = 'none' | 'idle' | 'loading' | 'empty' | 'not-found' | 'error' | 'stale';

export interface ValueLine {
  label: string;
  text: string;
}

export interface AwaitingView {
  /** The GM's value, beside the value already applied ahead of it. */
  yourValue: ValueLine[];
  ahead: ValueLine[] | null;
  aheadField: PcField | null;
  canConfirm: boolean;
  canCancel: boolean;
}

export interface FormView {
  values: PcFormValues;
  fieldErrors: Record<PcField, Violation[]>;
  formErrors: Violation[];
  busy: boolean;
  canSubmit: boolean;
  transportError: boolean;
  notice: 'expired' | 'cancelled' | null;
  awaiting: AwaitingView | null;
}

export interface EditView extends FormView {
  pcId: string;
  canClose: boolean;
}

export interface RowView {
  id: string;
  name: string;
  class: string;
  level: number;
  canEdit: boolean;
  canArchive: boolean;
  archiving: boolean;
  notes: string[];
}

export interface PjsView {
  banner: Banner;
  bannerText: string | null;
  rows: RowView[];
  /** `null` where the campaign is unknown or unset: no add, edit or archive action then. */
  add: FormView | null;
  edit: EditView | null;
}

const BANNER_TEXT: Record<Banner, string | null> = {
  none: null,
  idle: COPY.noCampaign,
  loading: COPY.loading,
  empty: COPY.empty,
  'not-found': COPY.notFound,
  error: COPY.readError,
  stale: COPY.stale,
};

const KEY_LABELS: Readonly<Record<string, string>> = {
  name: COPY.labels.nom,
  class: COPY.labels.classe,
  level: COPY.labels.niveau,
};

function valueLines(value: JsonValue | null): ValueLine[] {
  if (value === null) return [];
  if (typeof value === 'object' && !Array.isArray(value)) {
    return Object.entries(value).map(([key, v]) => ({
      label: Object.hasOwn(KEY_LABELS, key) ? (KEY_LABELS[key] ?? key) : key,
      text: typeof v === 'string' ? v : JSON.stringify(v),
    }));
  }
  return [{ label: '', text: JSON.stringify(value) }];
}

function bannerOf(s: PjsState): Banner {
  const list = s.list;
  switch (list.kind) {
    case 'idle':
    case 'loading':
    case 'not-found':
    case 'error':
      return list.kind;
    case 'rows':
      if (list.error) return 'error';
      if (list.stale) return 'stale';
      return list.rows.length === 0 ? 'empty' : 'none';
  }
}

function formView(f: FormState): FormView {
  const busy = isBusy(f);
  const fieldErrors: Record<PcField, Violation[]> = { nom: [], classe: [], niveau: [] };
  for (const field of PC_FIELDS) fieldErrors[field] = f.fieldErrors[field] ?? [];
  const c = f.command;
  return {
    values: f.values,
    fieldErrors,
    formErrors: f.formErrors,
    busy,
    canSubmit: !busy,
    transportError: f.transportError,
    notice: c.kind === 'done' && (c.status === 'expired' || c.status === 'cancelled') ? c.status : null,
    awaiting:
      c.kind === 'awaiting'
        ? {
            yourValue: valueLines(c.yourValue),
            ahead: c.ahead === null ? null : valueLines(c.ahead),
            aheadField: c.aheadField,
            canConfirm: c.actions.includes('confirm_overwrite'),
            canCancel: c.actions.includes('cancel'),
          }
        : null,
  };
}

function editView(e: EditState): EditView {
  const base = formView(e);
  // A parked edit can be left: it stays in the queue and lapses by itself.
  return { ...base, pcId: e.pcId, canClose: e.command.kind !== 'sending' };
}

function noteTexts(n: RowNotice | undefined): string[] {
  if (n === undefined) return [];
  switch (n.kind) {
    case 'rejected':
      return n.violations.map((v) => v.text);
    case 'transport':
      return [COPY.transport];
    case 'expired':
      return [COPY.expired];
    case 'cancelled':
      return [COPY.cancelled];
  }
}

export function viewOf(s: PjsState): PjsView {
  const banner = bannerOf(s);
  const actionable = s.list.kind !== 'idle' && s.list.kind !== 'not-found';
  const editBusy = s.edit !== null && isBusy(s.edit);
  const rows: RowView[] =
    s.list.kind === 'rows'
      ? s.list.rows.map((r) => {
          const archiving = s.archiving.has(r.id);
          return {
            id: r.id,
            name: r.name,
            class: r.class,
            level: r.level,
            canEdit: !archiving && !editBusy,
            canArchive: !archiving,
            archiving,
            notes: noteTexts(s.rowNotices[r.id]),
          };
        })
      : [];
  return {
    banner,
    bannerText: BANNER_TEXT[banner],
    rows,
    add: actionable ? formView(s.add) : null,
    edit: actionable && s.edit !== null ? editView(s.edit) : null,
  };
}

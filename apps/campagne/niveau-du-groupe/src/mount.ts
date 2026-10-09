// The only file that touches the DOM. It renders with `textContent`, never `innerHTML`, and writes a
// slot only when its text changed, so an announcement that changes nothing flickers nothing.

import { PartyLevelController, type PartyLevelHost, type PartyLevelState } from './controller';
import { COPY, toView, type PartyLevelView } from './view-model';

export interface NiveauDuGroupeProps {
  campagneId?: string | undefined;
}

export interface MountDeps {
  /** The page's single shell client: this Micro-UI never creates one. */
  shell: PartyLevelHost;
}

export interface MountedNiveauDuGroupe {
  update(props: NiveauDuGroupeProps): void;
  unmount(): void;
}

function slots(view: PartyLevelView): { level: string; count: string; note: string } {
  switch (view.kind) {
    case 'loading':
    case 'unavailable':
      return { level: view.text, count: '', note: '' };
    case 'no-level':
      return { level: view.text, count: view.count, note: view.stale ? COPY.stale : '' };
    case 'level':
      return { level: view.level, count: view.count, note: view.stale ? COPY.stale : '' };
  }
}

export function mount(host: HTMLElement, props: NiveauDuGroupeProps, deps: MountDeps): MountedNiveauDuGroupe {
  const doc = host.ownerDocument;
  const section = doc.createElement('section');
  section.dataset['microUi'] = 'NiveauDuGroupe';
  section.setAttribute('aria-live', 'polite');
  const slot = (name: string): HTMLElement => {
    const element = doc.createElement('span');
    element.dataset['slot'] = name;
    section.append(element);
    return element;
  };
  const levelSlot = slot('level');
  const countSlot = slot('count');
  const noteSlot = slot('note');

  const write = (element: HTMLElement, text: string): void => {
    if (element.textContent !== text) element.textContent = text;
  };
  const render = (state: PartyLevelState): void => {
    const view = toView(state);
    const text = slots(view);
    section.dataset['state'] = view.kind;
    write(levelSlot, text.level);
    write(countSlot, text.count);
    write(noteSlot, text.note);
  };

  const controller = new PartyLevelController(deps.shell, render);
  render(controller.state());
  host.append(section);
  controller.setCampagneId(props.campagneId);

  return {
    update(next) {
      controller.setCampagneId(next.campagneId);
    },
    unmount() {
      controller.dispose();
      section.remove();
    },
  };
}

import { InvalidIdentifierError } from './errors';

/** Contracts D and F: `<system>.<name>`, no version. */
export const IDENTIFIER_PATTERN = /^[a-z][a-z0-9-]*\.[A-Za-z][A-Za-z0-9]*$/;

export function assertIdentifier(id: string): void {
  if (typeof id !== 'string' || !IDENTIFIER_PATTERN.test(id)) {
    throw new InvalidIdentifierError(String(id));
  }
}

/** The wire id of a DataCapability, `<system>.<name>@<version>` (contract G). */
export function wireDataCapability(id: string, version: number): string {
  assertIdentifier(id);
  if (!Number.isInteger(version) || version < 1) {
    throw new InvalidIdentifierError(`${id}@${String(version)}`, 'has an invalid version (an integer >= 1 is required)');
  }
  return `${id}@${String(version)}`;
}

// The typed errors. Local refusals, transport failures and protocol failures
// are distinct classes; a rejected command is a value and has no class here.
// No message carries a payload value or an idempotency key (rule 53).

export class InvalidIdentifierError extends Error {
  readonly identifier: string;
  constructor(identifier: string, reason = 'is not a valid identifier') {
    super(`"${identifier}" ${reason}`);
    this.name = 'InvalidIdentifierError';
    this.identifier = identifier;
  }
}

/** Scheme and host only: a base URL may carry userinfo, which must never reach a log. */
function redactedOrigin(raw: string): string {
  try {
    const url = new URL(raw);
    return `${url.protocol}//${url.hostname}`;
  } catch {
    return '(unparsable)';
  }
}

export class NonLoopbackBaseUrlError extends Error {
  /** Redacted: scheme and host only. */
  readonly baseUrl: string;
  constructor(baseUrl: string) {
    const shown = redactedOrigin(baseUrl);
    super(`base URL "${shown}" is not on the loopback interface`);
    this.name = 'NonLoopbackBaseUrlError';
    this.baseUrl = shown;
  }
}

export class MissingBasedOnError extends Error {
  readonly dataCapability: string;
  constructor(dataCapability: string) {
    super(`"${dataCapability}" is confirm_on_stale: basedOn.version is required`);
    this.name = 'MissingBasedOnError';
    this.dataCapability = dataCapability;
  }
}

export class ActionNotOfferedError extends Error {
  readonly commandId: string;
  readonly action: string;
  constructor(commandId: string, action: string) {
    super(`action "${action}" is not offered for command ${commandId}`);
    this.name = 'ActionNotOfferedError';
    this.commandId = commandId;
    this.action = action;
  }
}

export class ClientDisposedError extends Error {
  constructor() {
    super('the shell client is disposed');
    this.name = 'ClientDisposedError';
  }
}

export class TransportError extends Error {
  readonly kind: 'network' | 'http';
  readonly status: number | undefined;
  readonly errorId: string | undefined;
  constructor(kind: 'network' | 'http', status?: number, errorId?: string) {
    super(
      kind === 'network'
        ? 'network failure'
        : `HTTP ${String(status)}${errorId === undefined ? '' : ` (${errorId})`}`,
    );
    this.name = 'TransportError';
    this.kind = kind;
    this.status = status;
    this.errorId = errorId;
  }
}

export class ProtocolError extends Error {
  readonly where: string;
  constructor(where: string, detail: string) {
    super(`protocol error in ${where}: ${detail}`);
    this.name = 'ProtocolError';
    this.where = where;
  }
}

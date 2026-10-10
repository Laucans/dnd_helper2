"use strict";
(() => {
  // packages/micro-ui-shell/src/contracts.ts
  var TERMINAL_STATUSES = ["applied", "rejected", "expired", "cancelled"];
  var H_FIELDS = ["commandId", "status", "dataVersion", "violations", "reviewId"];
  var QUEUE_STATES = [
    "queued",
    "awaiting_confirmation",
    "confirmed",
    "awaiting_review",
    "parked",
    "applied",
    "rejected",
    "cancelled",
    "expired"
  ];
  var G_FIELDS = [
    "command",
    "dataCapability",
    "by",
    "partition",
    "position",
    "basedOn",
    "projection",
    "yourValue",
    "state",
    "confirmation",
    "parked",
    "requeuedFrom"
  ];
  var MESSAGE_KINDS = [
    "CommandQueued",
    "InvariantAtRisk",
    "ValueDeclaredAhead",
    "BlockedByHold",
    "HoldExpiring",
    "ReviewRequired",
    "Parked",
    "ParkedExpiring",
    "Expired",
    "AheadResolved",
    "CommandApplied",
    "CommandRejected"
  ];
  var COMMAND_ACTIONS = [
    "confirm_overwrite",
    "cancel",
    "edit",
    "requeue",
    "renew",
    "approve",
    "reject"
  ];

  // packages/micro-ui-shell/src/errors.ts
  var InvalidIdentifierError = class extends Error {
    identifier;
    constructor(identifier, reason = "is not a valid identifier") {
      super(`"${identifier}" ${reason}`);
      this.name = "InvalidIdentifierError";
      this.identifier = identifier;
    }
  };
  function redactedOrigin(raw) {
    try {
      const url = new URL(raw);
      return `${url.protocol}//${url.hostname}`;
    } catch {
      return "(unparsable)";
    }
  }
  var NonLoopbackBaseUrlError = class extends Error {
    /** Redacted: scheme and host only. */
    baseUrl;
    constructor(baseUrl) {
      const shown = redactedOrigin(baseUrl);
      super(`base URL "${shown}" is not on the loopback interface`);
      this.name = "NonLoopbackBaseUrlError";
      this.baseUrl = shown;
    }
  };
  var MissingBasedOnError = class extends Error {
    dataCapability;
    constructor(dataCapability) {
      super(`"${dataCapability}" is confirm_on_stale: basedOn.version is required`);
      this.name = "MissingBasedOnError";
      this.dataCapability = dataCapability;
    }
  };
  var ActionNotOfferedError = class extends Error {
    commandId;
    action;
    constructor(commandId, action) {
      super(`action "${action}" is not offered for command ${commandId}`);
      this.name = "ActionNotOfferedError";
      this.commandId = commandId;
      this.action = action;
    }
  };
  var ClientDisposedError = class extends Error {
    constructor() {
      super("the shell client is disposed");
      this.name = "ClientDisposedError";
    }
  };
  var TransportError = class extends Error {
    kind;
    status;
    errorId;
    constructor(kind, status, errorId) {
      super(
        kind === "network" ? "network failure" : `HTTP ${String(status)}${errorId === void 0 ? "" : ` (${errorId})`}`
      );
      this.name = "TransportError";
      this.kind = kind;
      this.status = status;
      this.errorId = errorId;
    }
  };
  var ProtocolError = class extends Error {
    where;
    constructor(where, detail) {
      super(`protocol error in ${where}: ${detail}`);
      this.name = "ProtocolError";
      this.where = where;
    }
  };

  // packages/micro-ui-shell/src/identifier.ts
  var IDENTIFIER_PATTERN = /^[a-z][a-z0-9-]*\.[A-Za-z][A-Za-z0-9]*$/;
  function assertIdentifier(id) {
    if (typeof id !== "string" || !IDENTIFIER_PATTERN.test(id)) {
      throw new InvalidIdentifierError(String(id));
    }
  }
  function wireDataCapability(id, version) {
    assertIdentifier(id);
    if (!Number.isInteger(version) || version < 1) {
      throw new InvalidIdentifierError(`${id}@${String(version)}`, "has an invalid version (an integer >= 1 is required)");
    }
    return `${id}@${String(version)}`;
  }

  // packages/micro-ui-shell/src/protocol.ts
  var ROUTES = {
    /** `POST` — submit a command. Landed (`crates/campagne/serveur/src/http.rs`). */
    commands: "/commands",
    /** `GET` — look a command up. Landed. */
    command: (id) => `/commands/${encodeURIComponent(id)}`,
    /** `POST` — confirm an overwrite. Defined by #23, not served yet. */
    confirm: (id) => `/commands/${encodeURIComponent(id)}/confirm`,
    /** `POST` — cancel a command. Defined by #23, not served yet. */
    cancel: (id) => `/commands/${encodeURIComponent(id)}/cancel`,
    /** `POST` — run a persisted-query Capability by identifier. Defined by #23, not served yet. */
    capability: (identifier) => `/capabilities/${identifier}`,
    /** `GET` — server-sent events of the global `dataVersion`. Landed. */
    dataVersion: "/data-version"
  };
  var SSE_DATA_VERSION_EVENT = "dataVersion";
  var NOT_FOUND_ERROR = "not-found";

  // packages/micro-ui-shell/src/timers.ts
  var defaultTimers = {
    now: () => Date.now(),
    setTimeout: (fn, ms) => globalThis.setTimeout(fn, ms),
    clearTimeout: (handle) => {
      globalThis.clearTimeout(handle);
    }
  };
  function backoff(initialMs, maxMs, factor) {
    return (attempt) => Math.min(maxMs, initialMs * Math.pow(factor, Math.max(0, attempt)));
  }
  function sleep(timers, ms, signal) {
    return new Promise((resolve) => {
      if (signal.aborted) {
        resolve(false);
        return;
      }
      const onAbort = () => {
        timers.clearTimeout(handle);
        resolve(false);
      };
      const handle = timers.setTimeout(() => {
        signal.removeEventListener("abort", onAbort);
        resolve(true);
      }, ms);
      signal.addEventListener("abort", onAbort, { once: true });
    });
  }

  // packages/micro-ui-shell/src/validate.ts
  function isObject(x) {
    return typeof x === "object" && x !== null && !Array.isArray(x);
  }
  function obj(x, where) {
    if (!isObject(x)) throw new ProtocolError(where, "expected an object");
    return x;
  }
  function isVersion(x) {
    return typeof x === "number" && Number.isInteger(x) && x >= 0;
  }
  function isStringArray(x) {
    return Array.isArray(x) && x.every((s) => typeof s === "string");
  }
  function oneOf(list, x) {
    return typeof x === "string" && list.includes(x);
  }
  function isJson(x) {
    if (x === null || typeof x === "string" || typeof x === "boolean") return true;
    if (typeof x === "number") return Number.isFinite(x);
    if (Array.isArray(x)) return x.every(isJson);
    return isObject(x) && Object.values(x).every(isJson);
  }
  function parseCommandResult(x, where = "command result") {
    const o = obj(x, where);
    const keys = Object.keys(o);
    if (keys.length !== H_FIELDS.length || !H_FIELDS.every((k) => k in o)) {
      throw new ProtocolError(where, "fields differ from contract H");
    }
    if (typeof o["commandId"] !== "string") throw new ProtocolError(where, "commandId");
    if (!oneOf(TERMINAL_STATUSES, o["status"])) throw new ProtocolError(where, "unknown status");
    if (o["dataVersion"] !== null && !isVersion(o["dataVersion"])) throw new ProtocolError(where, "dataVersion");
    if (!isStringArray(o["violations"])) throw new ProtocolError(where, "violations");
    if (o["reviewId"] !== null && typeof o["reviewId"] !== "string") throw new ProtocolError(where, "reviewId");
    return {
      commandId: o["commandId"],
      status: o["status"],
      dataVersion: o["dataVersion"],
      violations: o["violations"],
      reviewId: o["reviewId"]
    };
  }
  function parseQueueEntry(x, where = "queue entry") {
    const o = obj(x, where);
    for (const k of Object.keys(o)) {
      if (!G_FIELDS.includes(k)) throw new ProtocolError(where, `unexpected field ${k}`);
    }
    for (const k of ["command", "dataCapability", "by", "partition"]) {
      if (typeof o[k] !== "string") throw new ProtocolError(where, k);
    }
    if (!isVersion(o["position"])) throw new ProtocolError(where, "position");
    const basedOn = obj(o["basedOn"], where);
    if (!isVersion(basedOn["version"])) throw new ProtocolError(where, "basedOn.version");
    if (!oneOf(QUEUE_STATES, o["state"])) throw new ProtocolError(where, "unknown state");
    if (o["yourValue"] !== void 0 && !isJson(o["yourValue"])) throw new ProtocolError(where, "yourValue");
    return o;
  }
  function parseMessage(x) {
    if (!isObject(x)) return null;
    if (!oneOf(MESSAGE_KINDS, x["message"])) return null;
    const to = x["to"];
    if (!isStringArray(to) || to.length === 0) return null;
    const message = { ...x, message: x["message"], to };
    if ("actions" in x) {
      const actions = x["actions"];
      if (Array.isArray(actions)) {
        message.actions = actions.filter((a) => oneOf(COMMAND_ACTIONS, a));
      } else {
        delete message.actions;
      }
    }
    return message;
  }
  function parseMessages(x, where) {
    if (!Array.isArray(x)) throw new ProtocolError(where, "messages is not an array");
    const out = [];
    for (const m of x) {
      const parsed = parseMessage(m);
      if (parsed !== null) out.push(parsed);
    }
    return out;
  }
  function parseLookupAnswer(x, where = "command lookup") {
    const o = obj(x, where);
    const hasResult = "result" in o;
    const hasEntry = "entry" in o;
    if (hasResult === hasEntry) throw new ProtocolError(where, "expected exactly one of result / entry");
    if (hasResult) return { result: parseCommandResult(o["result"], where) };
    return { entry: parseQueueEntry(o["entry"], where), messages: parseMessages(o["messages"], where) };
  }
  function parseSubmitAnswer(x) {
    const where = "submit answer";
    const o = obj(x, where);
    if (typeof o["commandId"] !== "string") throw new ProtocolError(where, "commandId");
    if (typeof o["partition"] !== "string") throw new ProtocolError(where, "partition");
    if (typeof o["replayed"] !== "boolean") throw new ProtocolError(where, "replayed");
    if (!isStringArray(o["warnings"])) throw new ProtocolError(where, "warnings");
    const lookup = parseLookupAnswer(o, where);
    return {
      ...lookup,
      commandId: o["commandId"],
      partition: o["partition"],
      replayed: o["replayed"],
      warnings: o["warnings"]
    };
  }
  function parseReadAnswer(x) {
    const where = "read answer";
    const o = obj(x, where);
    if (!("data" in o) || !isJson(o["data"])) throw new ProtocolError(where, "data");
    const asOf = o["asOf"];
    if (asOf !== void 0 && asOf !== null && !isVersion(asOf)) throw new ProtocolError(where, "asOf");
    return { asOf: asOf ?? null, data: o["data"] };
  }
  function parseDataVersionEvent(data) {
    let parsed;
    try {
      parsed = JSON.parse(data);
    } catch {
      return null;
    }
    if (!isObject(parsed)) return null;
    const v = parsed.dataVersion;
    return isVersion(v) ? v : null;
  }
  function asJsonObject(x) {
    return isObject(x) && isJson(x) ? x : null;
  }

  // packages/micro-ui-shell/src/violations.ts
  var defaultGeneric = (id) => id === null ? "rejected" : `rejected: ${id}`;
  function mapViolations(result, map, generic = defaultGeneric) {
    if (result.status !== "rejected") return [];
    if (result.violations.length === 0) {
      return [{ id: null, field: null, message: generic(null), mapped: false }];
    }
    return result.violations.map((id) => {
      const entry = Object.hasOwn(map, id) ? map[id] : void 0;
      return entry === void 0 ? { id, field: null, message: generic(id), mapped: false } : { id, field: entry.field, message: entry.message, mapped: true };
    });
  }

  // packages/micro-ui-shell/src/commands.ts
  var Commands = class {
    constructor(deps) {
      this.deps = deps;
    }
    /** Per command: the first terminal result is final (rule 33), else the latest pending view. */
    seen = /* @__PURE__ */ new Map();
    /** Per command: how many messages were known when an action was last sent. Those messages' actions are spent. */
    spent = /* @__PURE__ */ new Map();
    async submit(input) {
      this.deps.life.assertLive();
      const dataCapability = wireDataCapability(input.dataCapability, input.version);
      if (input.mode === "confirm_on_stale" && input.basedOn === void 0) {
        throw new MissingBasedOnError(input.dataCapability);
      }
      const idempotencyKey = input.idempotencyKey ?? this.deps.newIdempotencyKey();
      const body = { dataCapability, payload: input.payload, idempotencyKey };
      if (input.target?.id !== void 0) body.target = { id: input.target.id };
      if (input.basedOn !== void 0) body.basedOn = input.basedOn;
      const serialised = JSON.stringify(body);
      const delay2 = backoff(this.deps.config.poll.initialMs, this.deps.config.poll.maxMs, this.deps.config.poll.factor);
      for (let attempt = 0; ; attempt += 1) {
        try {
          const res = await this.deps.http.request("POST", ROUTES.commands, { body: serialised, signal: input.signal });
          if (res.kind === "not-found") throw new TransportError("http", 404, "not-found");
          const answer = parseSubmitAnswer(res.body);
          const first = this.ingest(answer.commandId, answer);
          return {
            commandId: answer.commandId,
            partition: answer.partition,
            replayed: answer.replayed,
            warnings: answer.warnings,
            idempotencyKey,
            first
          };
        } catch (e) {
          if (!(e instanceof TransportError) || e.kind !== "network" || attempt >= this.deps.config.submitRetries) throw e;
          const signal = input.signal === void 0 ? this.deps.life.signal : AbortSignal.any([this.deps.life.signal, input.signal]);
          if (!await sleep(this.deps.timers, delay2(attempt), signal)) {
            if (this.deps.life.signal.aborted) throw new ClientDisposedError();
            throw e;
          }
        }
      }
    }
    async lookup(commandId, opts) {
      this.deps.life.assertLive();
      const res = await this.deps.http.request("GET", ROUTES.command(commandId), { signal: opts?.signal });
      if (res.kind === "not-found") return { kind: "not-found", commandId };
      return this.ingest(commandId, parseLookupAnswer(res.body));
    }
    async act(commandId, action, opts) {
      this.deps.life.assertLive();
      const view = this.seen.get(commandId);
      if (view?.kind !== "pending" || !view.actions.includes(action)) throw new ActionNotOfferedError(commandId, action);
      const path = action === "confirm_overwrite" ? ROUTES.confirm(commandId) : ROUTES.cancel(commandId);
      const res = await this.deps.http.request("POST", path, { signal: opts?.signal });
      if (res.kind === "not-found") return { kind: "not-found", commandId };
      const answer = parseLookupAnswer(res.body);
      if ("messages" in answer) this.spent.set(commandId, answer.messages.length);
      return this.ingest(commandId, answer);
    }
    async awaitResult(commandId, opts = {}) {
      this.deps.life.assertLive();
      const settled = (result) => ({
        kind: "settled",
        result,
        violationMessages: mapViolations(result, opts.violationMessages ?? {})
      });
      const cached = this.seen.get(commandId);
      if (cached?.kind === "settled") return settled(cached.result);
      const stop = new AbortController();
      let timedOut = false;
      const timeoutMs = opts.timeoutMs ?? this.deps.config.awaitTimeoutMs;
      const timer = this.deps.timers.setTimeout(() => {
        timedOut = true;
        stop.abort();
      }, timeoutMs);
      const sources = [this.deps.life.signal, ...opts.signal === void 0 ? [] : [opts.signal]];
      const link = AbortSignal.any(sources);
      const onLink = () => stop.abort();
      if (link.aborted) stop.abort();
      else link.addEventListener("abort", onLink, { once: true });
      const delay2 = backoff(this.deps.config.poll.initialMs, this.deps.config.poll.maxMs, this.deps.config.poll.factor);
      let last = cached ?? null;
      let signature = last === null ? "" : signatureOf(last);
      let networkFailures = 0;
      const pending = () => ({
        kind: "still-pending",
        commandId,
        reason: timedOut ? "timeout" : "aborted",
        last
      });
      try {
        for (let attempt = 0; !stop.signal.aborted; attempt += 1) {
          let view;
          try {
            view = await this.lookup(commandId, { signal: stop.signal });
          } catch (e) {
            if (stop.signal.aborted || e instanceof ClientDisposedError) return pending();
            if (e instanceof TransportError && e.kind === "network" && networkFailures < this.deps.config.submitRetries) {
              networkFailures += 1;
              await sleep(this.deps.timers, delay2(attempt), stop.signal);
              continue;
            }
            throw e;
          }
          networkFailures = 0;
          if (view.kind === "not-found") return view;
          if (view.kind === "settled") return settled(view.result);
          last = view;
          const sig = signatureOf(view);
          if (sig !== signature) {
            signature = sig;
            opts.onState?.(view);
          }
          await sleep(this.deps.timers, delay2(attempt), stop.signal);
        }
        return pending();
      } finally {
        this.deps.timers.clearTimeout(timer);
        link.removeEventListener("abort", onLink);
      }
    }
    /** Merges an answer into what the client knows; the first terminal result wins. */
    ingest(commandId, answer) {
      const known = this.seen.get(commandId);
      if (known?.kind === "settled") return known;
      if ("result" in answer) {
        const view2 = { kind: "settled", result: answer.result };
        this.remember(commandId, view2);
        if (answer.result.status === "applied") {
          const { dataVersion } = answer.result;
          if (dataVersion === null) this.deps.hub.forceRefetch();
          else this.deps.hub.observe(dataVersion, "result");
        }
        return view2;
      }
      const { entry, messages } = answer;
      if (!isPending(entry.state)) throw new ProtocolError("command lookup", "pending answer with a terminal state");
      let offeredAt = messages.length - 1;
      while (offeredAt >= 0 && messages[offeredAt]?.actions === void 0) offeredAt -= 1;
      const offered = offeredAt < (this.spent.get(commandId) ?? 0) ? void 0 : messages[offeredAt];
      const withValue = [...messages].reverse().find((m) => m.yourValue !== void 0);
      const yourValue = entry.yourValue ?? withValue?.yourValue;
      const view = {
        kind: "pending",
        commandId,
        state: entry.state,
        entry,
        messages,
        actions: offered?.actions ?? [],
        ...yourValue === void 0 ? {} : { yourValue }
      };
      this.remember(commandId, view);
      return view;
    }
    /** Bounded: the oldest commands are forgotten first. */
    remember(commandId, view) {
      this.seen.delete(commandId);
      this.seen.set(commandId, view);
      while (this.seen.size > MAX_REMEMBERED) {
        const oldest = this.seen.keys().next().value;
        if (oldest === void 0) break;
        this.seen.delete(oldest);
        this.spent.delete(oldest);
      }
    }
  };
  var MAX_REMEMBERED = 500;
  function isPending(state) {
    return state !== "applied" && state !== "rejected" && state !== "expired" && state !== "cancelled";
  }
  function signatureOf(view) {
    if (view.kind === "pending") return `${view.state}|${view.actions.join(",")}`;
    return view.kind === "settled" ? `settled:${view.result.status}` : "not-found";
  }

  // packages/micro-ui-shell/src/data-version.ts
  var CLOSED = 2;
  var DataVersionHub = class {
    constructor(deps) {
      this.deps = deps;
      this.delay = backoff(deps.reconnect.initialMs, deps.reconnect.maxMs, 2);
    }
    known = null;
    pushed = null;
    listeners = /* @__PURE__ */ new Set();
    retainers = 0;
    source = null;
    reconnectTimer = null;
    attempt = 0;
    sawError = false;
    closed = false;
    refetchers = [];
    waiters = [];
    delay;
    knownVersion() {
      return this.known;
    }
    pushedVersion() {
      return this.pushed;
    }
    /** Called with every newly higher version. Idempotent unsubscribe. */
    addListener(fn) {
      if (this.closed) throw new ClientDisposedError();
      const entry = { fn };
      this.listeners.add(entry);
      this.sync();
      return () => {
        if (this.listeners.delete(entry)) this.sync();
      };
    }
    /** A live read keeps the stream open without being a version listener. */
    retain() {
      if (this.closed) throw new ClientDisposedError();
      this.retainers += 1;
      this.sync();
      let released = false;
      return () => {
        if (released) return;
        released = true;
        this.retainers -= 1;
        this.sync();
      };
    }
    /**
     * Registers the function that re-reads everything. `force` is true when the
     * version alone cannot say the data is current (a reconnect, an `applied`
     * result without a version); false on a bump, which a read already at that
     * version may skip.
     */
    onRefetch(fn) {
      this.refetchers.push(fn);
    }
    /** Re-read without moving the known version (an `applied` result without one). */
    forceRefetch() {
      this.fireRefetch(true);
    }
    /**
     * `push` comes from the stream, `result` from an own `applied` command.
     * A value that is not an integer >= 0, or not above the known one, is ignored.
     */
    observe(version, source) {
      if (this.closed || version === null || !Number.isInteger(version) || version < 0) return;
      if (source === "push" && (this.pushed === null || version > this.pushed)) {
        this.pushed = version;
        this.releaseWaiters();
      }
      if (this.known !== null && version <= this.known) return;
      this.known = version;
      for (const l of [...this.listeners]) {
        try {
          l.fn(version);
        } catch {
          this.deps.diagnostic({ kind: "listener-threw" });
        }
      }
      this.fireRefetch(false);
    }
    /** Resolves `true` when a push reached `version`, `false` after `ms` or on close. */
    whenPushed(version, ms) {
      if (this.pushed !== null && this.pushed >= version) return Promise.resolve(true);
      if (this.closed) return Promise.resolve(false);
      return new Promise((resolve) => {
        const waiter = { version, resolve, handle: null };
        waiter.handle = this.deps.timers.setTimeout(() => {
          this.waiters = this.waiters.filter((w) => w !== waiter);
          resolve(false);
        }, ms);
        this.waiters.push(waiter);
      });
    }
    close() {
      this.closed = true;
      this.listeners.clear();
      this.retainers = 0;
      this.stopStream();
      for (const w of this.waiters) {
        this.deps.timers.clearTimeout(w.handle);
        w.resolve(false);
      }
      this.waiters = [];
    }
    fireRefetch(force) {
      for (const fn of this.refetchers) fn(force);
    }
    releaseWaiters() {
      const pushed = this.pushed;
      if (pushed === null) return;
      const ready = this.waiters.filter((w) => w.version <= pushed);
      this.waiters = this.waiters.filter((w) => w.version > pushed);
      for (const w of ready) {
        this.deps.timers.clearTimeout(w.handle);
        w.resolve(true);
      }
    }
    /** Opens on the first listener, closes when the last one leaves (rule 45). */
    sync() {
      const wanted = !this.closed && this.listeners.size + this.retainers > 0;
      if (wanted) {
        if (this.source === null && this.reconnectTimer === null) this.open();
      } else {
        this.stopStream();
      }
    }
    stopStream() {
      if (this.source !== null) {
        this.source.close();
        this.source = null;
      }
      if (this.reconnectTimer !== null) {
        this.deps.timers.clearTimeout(this.reconnectTimer);
        this.reconnectTimer = null;
      }
      this.attempt = 0;
      this.sawError = false;
    }
    open() {
      const source = this.deps.eventSource(`${this.deps.baseUrl}${ROUTES.dataVersion}`);
      this.source = source;
      source.addEventListener("open", () => {
        if (this.source !== source) return;
        this.attempt = 0;
        if (this.sawError) {
          this.sawError = false;
          this.fireRefetch(true);
        }
      });
      source.addEventListener(SSE_DATA_VERSION_EVENT, (event) => {
        if (this.source !== source) return;
        const version = typeof event.data === "string" ? parseDataVersionEvent(event.data) : null;
        if (version === null) {
          this.deps.diagnostic({ kind: "malformed-data-version-event" });
          return;
        }
        this.observe(version, "push");
      });
      source.addEventListener("error", () => {
        if (this.source !== source) return;
        this.sawError = true;
        if (source.readyState !== CLOSED) return;
        source.close();
        this.source = null;
        this.reconnectTimer = this.deps.timers.setTimeout(() => {
          this.reconnectTimer = null;
          this.sync();
        }, this.delay(this.attempt));
        this.attempt += 1;
      });
    }
  };

  // packages/micro-ui-shell/src/base-url.ts
  var LOOPBACK_HOSTS = /* @__PURE__ */ new Set(["127.0.0.1", "localhost", "[::1]"]);
  function assertLoopbackBaseUrl(raw) {
    let url;
    try {
      url = new URL(raw);
    } catch {
      throw new NonLoopbackBaseUrlError(raw);
    }
    if (url.protocol !== "http:" && url.protocol !== "https:" || !LOOPBACK_HOSTS.has(url.hostname) || url.username !== "" || url.password !== "") {
      throw new NonLoopbackBaseUrlError(raw);
    }
    return `${url.origin}${url.pathname}`.replace(/\/+$/, "");
  }

  // packages/micro-ui-shell/src/http.ts
  function createHttp(baseUrl, fetchFn, life) {
    return {
      async request(method, path, init) {
        life.assertLive();
        const signal = init?.signal === void 0 ? life.signal : AbortSignal.any([life.signal, init.signal]);
        const headers = { accept: "application/json" };
        const req = { method, signal, headers };
        if (init?.body !== void 0) {
          headers["content-type"] = "application/json";
          req.body = init.body;
        }
        let response;
        try {
          response = await fetchFn(`${baseUrl}${path}`, req);
        } catch (e) {
          if (life.signal.aborted) throw new ClientDisposedError();
          if (signal.aborted) throw e;
          throw new TransportError("network");
        }
        let text;
        try {
          text = await response.text();
        } catch {
          if (life.signal.aborted) throw new ClientDisposedError();
          if (signal.aborted) throw signal.reason;
          throw new TransportError("network");
        }
        let parsed;
        let parseable = true;
        try {
          parsed = text === "" ? void 0 : JSON.parse(text);
        } catch {
          parseable = false;
        }
        if (response.status >= 200 && response.status < 300) {
          if (!parseable) throw new ProtocolError(path, "body is not JSON");
          return { kind: "ok", body: parsed };
        }
        const errorId = parseable ? errorIdOf(parsed) : void 0;
        if (response.status === 404 && errorId === NOT_FOUND_ERROR) return { kind: "not-found" };
        throw new TransportError("http", response.status, errorId);
      }
    };
  }
  function errorIdOf(body) {
    const o = asJsonObject(body);
    const id = o?.["error"];
    return typeof id === "string" ? id : void 0;
  }

  // packages/micro-ui-shell/src/reads.ts
  function stableStringify(value) {
    if (Array.isArray(value)) return `[${value.map(stableStringify).join(",")}]`;
    if (value !== null && typeof value === "object") {
      const keys = Object.keys(value).sort();
      return `{${keys.map((k) => `${JSON.stringify(k)}:${stableStringify(value[k])}`).join(",")}}`;
    }
    return JSON.stringify(value);
  }
  var Reads = class {
    constructor(deps) {
      this.deps = deps;
      deps.hub.onRefetch((force) => {
        this.markAllDirty(force);
      });
    }
    entries = /* @__PURE__ */ new Map();
    flushTimer = null;
    async read(capability, variables = {}, opts) {
      this.deps.life.assertLive();
      assertIdentifier(capability);
      const body = { variables };
      const res = await this.deps.http.request("POST", ROUTES.capability(capability), {
        body: JSON.stringify(body),
        signal: opts?.signal
      });
      if (res.kind === "not-found") return { kind: "not-found" };
      const answer = parseReadAnswer(res.body);
      return { kind: "found", data: answer.data, asOf: answer.asOf };
    }
    watch(capability, variables, listener) {
      this.deps.life.assertLive();
      assertIdentifier(capability);
      const key = `${capability}\0${stableStringify(variables)}`;
      let entry = this.entries.get(key);
      const fresh = entry === void 0;
      if (entry === void 0) {
        entry = {
          capability,
          // A copy: the caller may keep mutating its own object, and the key was computed from this one.
          variables: structuredClone(variables),
          listeners: /* @__PURE__ */ new Set(),
          last: void 0,
          freshAsOf: null,
          problem: void 0,
          inflight: false,
          dirty: false,
          wanted: null,
          abort: new AbortController()
        };
        this.entries.set(key, entry);
      }
      const wrapped = listener;
      entry.listeners.add(wrapped);
      const release = this.deps.hub.retain();
      if (fresh) {
        this.markDirty(entry, true);
      } else {
        if (entry.last !== void 0) this.emitTo(wrapped, entry.last);
        if (entry.problem !== void 0) {
          this.emitTo(wrapped, entry.problem);
          this.markDirty(entry, true);
        }
      }
      const owner = entry;
      let done = false;
      return () => {
        if (done) return;
        done = true;
        owner.listeners.delete(wrapped);
        release();
        if (owner.listeners.size === 0) {
          owner.abort.abort();
          if (this.entries.get(key) === owner) this.entries.delete(key);
        }
      };
    }
    dispose() {
      if (this.flushTimer !== null) this.deps.timers.clearTimeout(this.flushTimer);
      this.flushTimer = null;
      for (const entry of this.entries.values()) {
        entry.abort.abort();
        entry.listeners.clear();
      }
      this.entries.clear();
    }
    markAllDirty(force) {
      for (const entry of this.entries.values()) this.markDirty(entry, force);
    }
    /**
     * Every bump in one tick lands in a single flush, at the latest version. A
     * bump the entry's data already covers (its `asOf` is at or above the known
     * version) is skipped; a forced refetch never is.
     */
    markDirty(entry, force) {
      const known = this.deps.hub.knownVersion();
      if (!force && !entry.dirty && entry.problem === void 0 && entry.freshAsOf !== null && known !== null && entry.freshAsOf >= known) {
        return;
      }
      entry.dirty = true;
      entry.wanted = known;
      if (this.flushTimer !== null) return;
      this.flushTimer = this.deps.timers.setTimeout(() => {
        this.flushTimer = null;
        for (const e of [...this.entries.values()]) {
          if (e.dirty && !e.inflight) void this.run(e);
        }
      }, 0);
    }
    async run(entry) {
      entry.dirty = false;
      entry.inflight = true;
      const floor = entry.wanted;
      let satisfied = null;
      try {
        const event = await this.fetchAtLeast(entry, floor);
        if (event !== null) {
          if (event.kind === "data") {
            satisfied = event.asOf;
            entry.last = event;
            entry.freshAsOf = event.asOf;
            entry.problem = void 0;
          } else if (event.kind === "not-found") {
            entry.last = event;
            entry.freshAsOf = null;
            entry.problem = void 0;
          } else {
            entry.problem = event;
          }
          this.emit(entry, event);
        }
      } catch (e) {
        if (e instanceof TransportError || e instanceof ProtocolError) {
          const event = { kind: "error", error: e };
          entry.problem = event;
          this.emit(entry, event);
        } else if (!(e instanceof ClientDisposedError) && !entry.abort.signal.aborted) {
          this.deps.diagnostic({ kind: "read-failed" });
        }
      } finally {
        entry.inflight = false;
      }
      if (entry.dirty && satisfied !== null && entry.wanted !== null && satisfied >= entry.wanted) {
        entry.dirty = false;
      }
      if (entry.dirty && !entry.abort.signal.aborted) this.markDirty(entry, true);
    }
    /** Never presents data older than `floor` as fresh (rule 48). `null`: the entry is gone. */
    async fetchAtLeast(entry, floor) {
      const { retries, delayMs, pushWaitMs } = this.deps.config.staleRead;
      const signal = AbortSignal.any([this.deps.life.signal, entry.abort.signal]);
      let waitedForPush = false;
      for (let attempt = 0; ; attempt += 1) {
        const outcome = await this.read(entry.capability, entry.variables, { signal });
        if (outcome.kind === "not-found") return { kind: "not-found" };
        if (floor === null) return { kind: "data", data: outcome.data, asOf: outcome.asOf };
        if (outcome.asOf === null) {
          if (waitedForPush || (this.deps.hub.pushedVersion() ?? -1) >= floor) {
            return { kind: "data", data: outcome.data, asOf: null };
          }
          waitedForPush = true;
          await this.deps.hub.whenPushed(floor, pushWaitMs);
          if (signal.aborted) return null;
          continue;
        }
        if (outcome.asOf >= floor) return { kind: "data", data: outcome.data, asOf: outcome.asOf };
        if (attempt >= retries) return { kind: "stale", asOf: outcome.asOf, wanted: floor };
        if (!await sleep(this.deps.timers, delayMs, signal)) return null;
      }
    }
    emit(entry, event) {
      for (const l of [...entry.listeners]) this.emitTo(l, event);
    }
    emitTo(listener, event) {
      try {
        listener(event);
      } catch {
        this.deps.diagnostic({ kind: "listener-threw" });
      }
    }
  };

  // packages/micro-ui-shell/src/client.ts
  function createShellClient(options) {
    const baseUrl = assertLoopbackBaseUrl(options.baseUrl);
    const fetchFn = options.fetch;
    const eventSourceFactory = options.eventSource;
    const timers = options.timers ?? defaultTimers;
    const diagnostic = (d) => {
      try {
        options.onDiagnostic?.(d);
      } catch {
      }
    };
    const dispose = new AbortController();
    const life = {
      signal: dispose.signal,
      assertLive: () => {
        if (dispose.signal.aborted) throw new ClientDisposedError();
      }
    };
    const hub = new DataVersionHub({
      baseUrl,
      eventSource: (url) => eventSourceFactory(url),
      timers,
      reconnect: { initialMs: options.reconnect?.initialMs ?? 500, maxMs: options.reconnect?.maxMs ?? 3e4 },
      diagnostic
    });
    const http = createHttp(baseUrl, (input, init) => fetchFn(input, init), life);
    const commands = new Commands({
      http,
      hub,
      timers,
      life,
      newIdempotencyKey: options.newIdempotencyKey ?? (() => globalThis.crypto.randomUUID()),
      config: {
        poll: {
          initialMs: options.poll?.initialMs ?? 250,
          maxMs: options.poll?.maxMs ?? 5e3,
          factor: options.poll?.factor ?? 2
        },
        awaitTimeoutMs: options.awaitTimeoutMs ?? 3e4,
        submitRetries: options.submitRetries ?? 2
      }
    });
    const reads = new Reads({
      http,
      hub,
      timers,
      life,
      diagnostic,
      config: {
        staleRead: {
          retries: options.staleRead?.retries ?? 5,
          delayMs: options.staleRead?.delayMs ?? 200,
          pushWaitMs: options.staleRead?.pushWaitMs ?? 5e3
        }
      }
    });
    return {
      read: (capability, variables, opts) => reads.read(capability, variables, opts),
      watch: (capability, variables, listener) => reads.watch(capability, variables, listener),
      submit: (input) => commands.submit(input),
      lookup: (commandId, opts) => commands.lookup(commandId, opts),
      awaitResult: (commandId, opts) => commands.awaitResult(commandId, opts),
      act: (commandId, action, opts) => commands.act(commandId, action, opts),
      onDataVersion: (listener) => {
        life.assertLive();
        return hub.addListener(listener);
      },
      knownDataVersion: () => hub.knownVersion(),
      dispose: () => {
        if (dispose.signal.aborted) return;
        dispose.abort();
        reads.dispose();
        hub.close();
      }
    };
  }

  // apps/campagne/pjs/src/copy.ts
  var COPY = {
    title: "Personnages joueurs",
    loading: "Chargement des PJ\u2026",
    empty: "Aucun PJ pour le moment. Ajoutez le premier ci-dessous.",
    notFound: "Campagne introuvable.",
    noCampaign: "Aucune campagne s\xE9lectionn\xE9e.",
    readError: "La liste des PJ n\u2019a pas pu \xEAtre lue.",
    stale: "La liste affich\xE9e est peut-\xEAtre en retard sur les derni\xE8res modifications.",
    colName: "Nom",
    colClass: "Classe",
    colLevel: "Niveau",
    labels: { nom: "Nom", classe: "Classe", niveau: "Niveau" },
    addTitle: "Ajouter un PJ",
    editTitle: "Modifier un PJ",
    add: "Ajouter",
    edit: "Modifier",
    save: "Enregistrer",
    archive: "Archiver",
    archiving: "Archivage\u2026",
    close: "Fermer",
    sending: "Envoi en cours\u2026",
    transport: "La connexion au serveur a \xE9t\xE9 interrompue. Si la commande est partie, elle ne sera appliqu\xE9e qu\u2019une fois.",
    yourValue: "Votre modification",
    aheadValue: "Valeur d\xE9j\xE0 appliqu\xE9e",
    awaitingTitle: "Votre modification attend votre confirmation.",
    leaveHint: "Vous pouvez fermer ce formulaire : sans confirmation, la modification expire au bout de 24 h et rien ne change.",
    confirm: "Confirmer ma modification",
    cancel: "Annuler ma modification",
    expired: "La modification a expir\xE9 : rien n\u2019a chang\xE9.",
    cancelled: "La modification a \xE9t\xE9 annul\xE9e : rien n\u2019a chang\xE9.",
    refused: (id) => id === null ? "Refus\xE9." : `Refus\xE9 : ${id}`
  };

  // apps/campagne/pjs/src/identifiers.ts
  var LISTER_PJS = "campagne.listerPjs";
  var AJOUTER_PJ = { dataCapability: "campagne.ajouterPj", version: 1, mode: "relative" };
  var MODIFIER_PJ = {
    dataCapability: "campagne.modifierPJ",
    version: 1,
    mode: "confirm_on_stale",
    aggregate: "PJ"
  };
  var ARCHIVER_PJ = {
    dataCapability: "campagne.archiverPJ",
    version: 1,
    mode: "overwrite",
    aggregate: "PJ"
  };

  // apps/campagne/pjs/src/violations.ts
  var PC_FIELDS = ["nom", "classe", "niveau"];
  var PJ_VIOLATIONS = {
    "pc-name-required": { field: "nom", message: "Nom refus\xE9 : pc-name-required" },
    "pc-name-unique-in-campaign": { field: "nom", message: "Nom refus\xE9 : pc-name-unique-in-campaign" },
    "pc-class-required": { field: "classe", message: "Classe refus\xE9e : pc-class-required" },
    "pc-level-range": { field: "niveau", message: "Niveau refus\xE9 : pc-level-range" }
  };
  function fieldOfMessage(field) {
    switch (field) {
      case "PJ.name":
        return "nom";
      case "PJ.class":
        return "classe";
      case "PJ.level":
        return "niveau";
      default:
        return null;
    }
  }
  function fieldOfViolation(field) {
    return field === "nom" || field === "classe" || field === "niveau" ? field : null;
  }
  var INTEGER = /^-?\d+$/;
  function levelPayload(raw) {
    if (raw === "") return void 0;
    if (INTEGER.test(raw)) {
      const n = Number(raw);
      if (Number.isSafeInteger(n)) return n;
    }
    return raw;
  }
  function pcPayload(form) {
    const niveau = levelPayload(form.niveau);
    return niveau === void 0 ? { nom: form.nom, classe: form.classe } : { nom: form.nom, classe: form.classe, niveau };
  }

  // apps/campagne/pjs/src/controller.ts
  var emptyValues = () => ({ nom: "", classe: "", niveau: "" });
  var emptyForm = () => ({
    values: emptyValues(),
    fieldErrors: {},
    formErrors: [],
    command: { kind: "idle" },
    transportError: false
  });
  var isBusy = (f) => f.command.kind === "sending" || f.command.kind === "awaiting";
  var isOffered = (a) => a === "confirm_overwrite" || a === "cancel";
  function isRecord(v) {
    return typeof v === "object" && v !== null && !Array.isArray(v);
  }
  function declaredAhead(view, commandId) {
    for (let i = view.messages.length - 1; i >= 0; i -= 1) {
      const m = view.messages[i];
      if (m === void 0) continue;
      if (m.command !== void 0 && m.command !== commandId) continue;
      if (m.message === "ValueDeclaredAhead" && m.declaredAhead !== void 0) {
        return { value: m.declaredAhead.value, field: fieldOfMessage(m.field) };
      }
    }
    const ahead = view.entry.projection?.["pendingAhead"];
    if (Array.isArray(ahead)) {
      const first = ahead[0];
      if (isRecord(first) && first["value"] !== void 0) return { value: first["value"], field: null };
    }
    return null;
  }
  var CLOSED_MAX = 200;
  function pause(ms, signal) {
    return new Promise((resolve) => {
      if (signal.aborted) {
        resolve();
        return;
      }
      const done = () => {
        clearTimeout(timer);
        signal.removeEventListener("abort", done);
        resolve();
      };
      const timer = setTimeout(done, ms);
      signal.addEventListener("abort", done, { once: true });
    });
  }
  function createPjsController(campagneId, deps) {
    const { shell: shell2 } = deps;
    const newKey = deps.newKey ?? (() => crypto.randomUUID());
    const retryMs = deps.retryMs ?? 5e3;
    let state = freshState(null);
    const listeners = /* @__PURE__ */ new Set();
    let unwatch = null;
    let disposed = false;
    let epoch = 0;
    const aborts = /* @__PURE__ */ new Set();
    const inflight = /* @__PURE__ */ new Map();
    const closed = /* @__PURE__ */ new Set();
    const keys = { add: null, edit: null };
    const current = { add: null, edit: null };
    const archiveKeys = /* @__PURE__ */ new Map();
    function freshState(id) {
      return { campagneId: id, list: { kind: "idle" }, add: emptyForm(), edit: null, archiving: /* @__PURE__ */ new Set(), rowNotices: {} };
    }
    function set(next) {
      state = next;
      for (const l of [...listeners]) l(state);
    }
    function patchForm(which, patch) {
      if (which === "add") set({ ...state, add: { ...state.add, ...patch } });
      else if (state.edit !== null) set({ ...state, edit: { ...state.edit, ...patch } });
    }
    const formOf = (which) => which === "add" ? state.add : state.edit;
    const actionable = () => state.campagneId !== null && state.list.kind !== "not-found" && state.list.kind !== "idle";
    function applyListEvent(e) {
      const prev = state.list;
      switch (e.kind) {
        case "data":
          if (!Array.isArray(e.data)) {
            set({ ...state, list: { kind: "error" } });
            return;
          }
          set({ ...state, list: { kind: "rows", rows: e.data, asOf: e.asOf, stale: false, error: false } });
          return;
        case "not-found":
          set({ ...state, list: { kind: "not-found" } });
          return;
        case "stale":
          if (prev.kind === "rows") set({ ...state, list: { ...prev, stale: true } });
          return;
        case "error":
          set({ ...state, list: prev.kind === "rows" ? { ...prev, error: true } : { kind: "error" } });
          return;
      }
    }
    function startWatch(id) {
      const mine = epoch;
      set({ ...state, list: { kind: "loading" } });
      unwatch = shell2.watch(LISTER_PJS, { campagneId: id }, (e) => {
        if (mine === epoch) applyListEvent(e);
      });
    }
    function stopAll() {
      epoch += 1;
      unwatch?.();
      unwatch = null;
      for (const ac of aborts) ac.abort();
      aborts.clear();
      inflight.clear();
      closed.clear();
    }
    function setCampagne(id) {
      if (disposed || id === state.campagneId) return;
      stopAll();
      keys.add = null;
      keys.edit = null;
      current.add = null;
      current.edit = null;
      archiveKeys.clear();
      set(freshState(id));
      if (id !== null) startWatch(id);
    }
    function remember(commandId) {
      closed.add(commandId);
      if (closed.size > CLOSED_MAX) {
        const oldest = closed.values().next().value;
        if (oldest !== void 0) closed.delete(oldest);
      }
    }
    function clearMarks() {
      set({
        ...state,
        add: { ...state.add, fieldErrors: {}, formErrors: [] },
        edit: state.edit === null ? null : { ...state.edit, fieldErrors: {}, formErrors: [] },
        rowNotices: {}
      });
    }
    function settle(commandId, result, violations, hooks) {
      if (closed.has(commandId)) return;
      remember(commandId);
      const ac = inflight.get(commandId);
      inflight.delete(commandId);
      if (result.status === "applied") clearMarks();
      hooks.onSettled(commandId, result, violations);
      ac?.abort();
    }
    function handleView(commandId, view, hooks) {
      if (view.kind === "settled") settle(commandId, view.result, mapViolations(view.result, PJ_VIOLATIONS), hooks);
      else if (view.kind === "pending") hooks.onPending(commandId, view);
    }
    async function runCommand(input, hooks) {
      const mine = epoch;
      const ac = new AbortController();
      aborts.add(ac);
      let commandId = null;
      try {
        const sent = await shell2.submit({ ...input, signal: ac.signal });
        if (mine !== epoch) return;
        const id = sent.commandId;
        commandId = id;
        inflight.set(id, ac);
        hooks.onId(id);
        handleView(id, sent.first, hooks);
        let linkLost = false;
        for (; ; ) {
          if (mine !== epoch || closed.has(id) || ac.signal.aborted) return;
          let out;
          try {
            out = await shell2.awaitResult(id, {
              signal: ac.signal,
              violationMessages: PJ_VIOLATIONS,
              onState: (view) => {
                if (mine === epoch) handleView(id, view, hooks);
              }
            });
          } catch (e) {
            if (mine !== epoch || ac.signal.aborted) return;
            if (!(e instanceof TransportError || e instanceof ProtocolError)) throw e;
            if (!linkLost) {
              linkLost = true;
              hooks.onLink(false);
            }
            await pause(retryMs, ac.signal);
            continue;
          }
          if (mine !== epoch) return;
          if (linkLost) {
            linkLost = false;
            hooks.onLink(true);
          }
          if (out.kind === "settled") {
            settle(id, out.result, out.violationMessages, hooks);
            return;
          }
          if (out.kind === "not-found") {
            hooks.onTransport(id);
            return;
          }
          if (out.reason === "aborted") return;
        }
      } catch (e) {
        if (mine !== epoch) return;
        if (e instanceof TransportError || e instanceof ProtocolError) {
          hooks.onTransport(commandId);
          return;
        }
        if (e instanceof ClientDisposedError) return;
        throw e;
      } finally {
        aborts.delete(ac);
        if (commandId !== null && inflight.get(commandId) === ac) inflight.delete(commandId);
      }
    }
    function violationNotes(violations) {
      const fieldErrors = {};
      const formErrors = [];
      const all = [];
      for (const v of violations) {
        const field = fieldOfViolation(v.field);
        const note = field === null ? { id: v.id, text: COPY.refused(v.id) } : { id: v.id, text: v.message };
        all.push(note);
        if (field === null) formErrors.push(note);
        else (fieldErrors[field] ??= []).push(note);
      }
      return { fieldErrors, formErrors, all };
    }
    function formHooks(which) {
      return {
        onId: (commandId) => {
          current[which] = commandId;
        },
        onPending: (commandId, view) => {
          if (current[which] !== commandId) return;
          const f = formOf(which);
          if (f === null || f.command.kind === "done") return;
          if (which === "edit" && view.state === "awaiting_confirmation") {
            const ahead = declaredAhead(view, commandId);
            patchForm(which, {
              command: {
                kind: "awaiting",
                commandId,
                // Never ask the GM to overwrite blind: fall back to what they typed.
                yourValue: view.yourValue ?? view.entry.yourValue ?? { name: f.values.nom, class: f.values.classe, level: f.values.niveau },
                ahead: ahead?.value ?? null,
                aheadField: ahead?.field ?? null,
                actions: view.actions.filter(isOffered)
              }
            });
          } else {
            patchForm(which, { command: { kind: "sending" } });
          }
        },
        onSettled: (commandId, result, violations) => {
          if (current[which] !== commandId) return;
          keys[which] = null;
          current[which] = null;
          if (result.status === "applied") {
            if (which === "add") {
              set({ ...state, add: { ...emptyForm(), command: { kind: "done", status: "applied" } } });
            } else {
              set({ ...state, edit: null });
            }
            return;
          }
          const f = formOf(which);
          const { fieldErrors, formErrors } = result.status === "rejected" ? violationNotes(violations) : { fieldErrors: f?.fieldErrors ?? {}, formErrors: f?.formErrors ?? [] };
          patchForm(which, { fieldErrors, formErrors, command: { kind: "done", status: result.status }, transportError: false });
        },
        onTransport: (commandId) => {
          if (commandId !== null && current[which] !== commandId) return;
          patchForm(which, { command: { kind: "idle" }, transportError: true });
        },
        onLink: (ok) => {
          patchForm(which, { transportError: !ok });
        }
      };
    }
    function setField(which, field, value) {
      const f = formOf(which);
      if (disposed || f === null || isBusy(f)) return;
      const fieldErrors = { ...f.fieldErrors };
      delete fieldErrors[field];
      keys[which] = null;
      patchForm(which, {
        values: { ...f.values, [field]: value },
        fieldErrors,
        command: f.command.kind === "done" ? { kind: "idle" } : f.command
      });
    }
    async function submitAdd() {
      const id = state.campagneId;
      if (disposed || id === null || !actionable() || isBusy(state.add)) return;
      const key = keys.add ??= newKey();
      const values = state.add.values;
      patchForm("add", { command: { kind: "sending" }, transportError: false });
      await runCommand(
        {
          dataCapability: AJOUTER_PJ.dataCapability,
          version: AJOUTER_PJ.version,
          mode: AJOUTER_PJ.mode,
          payload: { campagneId: id, ...pcPayload(values) },
          idempotencyKey: key
        },
        formHooks("add")
      );
    }
    function openEdit(pcId) {
      if (disposed || !actionable() || state.edit !== null && isBusy(state.edit)) return;
      const list = state.list;
      if (list.kind !== "rows") return;
      const row = list.rows.find((r) => r.id === pcId);
      if (row === void 0) return;
      const basedOn = list.asOf ?? shell2.knownDataVersion() ?? 0;
      keys.edit = null;
      current.edit = null;
      set({
        ...state,
        edit: { ...emptyForm(), values: { nom: row.name, classe: row.class, niveau: String(row.level) }, pcId, basedOn }
      });
    }
    function closeEdit() {
      const e = state.edit;
      if (disposed || e === null || e.command.kind === "sending") return;
      const id = current.edit;
      if (id !== null) inflight.get(id)?.abort();
      keys.edit = null;
      current.edit = null;
      set({ ...state, edit: null });
    }
    async function submitEdit() {
      const e = state.edit;
      if (disposed || e === null || !actionable() || isBusy(e)) return;
      const key = keys.edit ??= newKey();
      patchForm("edit", { command: { kind: "sending" }, transportError: false });
      await runCommand(
        {
          dataCapability: MODIFIER_PJ.dataCapability,
          version: MODIFIER_PJ.version,
          mode: MODIFIER_PJ.mode,
          target: { aggregate: MODIFIER_PJ.aggregate, id: e.pcId },
          payload: pcPayload(e.values),
          basedOn: { version: e.basedOn },
          idempotencyKey: key
        },
        formHooks("edit")
      );
    }
    async function actOnEdit(action) {
      const e = state.edit;
      const commandId = current.edit;
      if (disposed || e === null || commandId === null || e.command.kind !== "awaiting") return;
      if (!e.command.actions.includes(action)) return;
      const mine = epoch;
      const hooks = formHooks("edit");
      try {
        handleView(commandId, await shell2.act(commandId, action), hooks);
      } catch (err) {
        if (mine !== epoch) return;
        if (err instanceof ActionNotOfferedError) {
          try {
            handleView(commandId, await shell2.lookup(commandId), hooks);
          } catch (lookupErr) {
            if (mine !== epoch) return;
            if (lookupErr instanceof TransportError || lookupErr instanceof ProtocolError) {
              patchForm("edit", { transportError: true });
              return;
            }
            throw lookupErr;
          }
          return;
        }
        if (err instanceof TransportError || err instanceof ProtocolError) {
          patchForm("edit", { transportError: true });
          return;
        }
        if (err instanceof ClientDisposedError) return;
        throw err;
      }
    }
    async function archive(pcId) {
      if (disposed || !actionable() || state.archiving.has(pcId)) return;
      const key = archiveKeys.get(pcId) ?? newKey();
      archiveKeys.set(pcId, key);
      const notices = { ...state.rowNotices };
      delete notices[pcId];
      set({ ...state, archiving: /* @__PURE__ */ new Set([...state.archiving, pcId]), rowNotices: notices });
      const stop = (notice) => {
        const archiving = new Set(state.archiving);
        archiving.delete(pcId);
        set({ ...state, archiving, rowNotices: notice === null ? state.rowNotices : { ...state.rowNotices, [pcId]: notice } });
      };
      await runCommand(
        {
          dataCapability: ARCHIVER_PJ.dataCapability,
          version: ARCHIVER_PJ.version,
          mode: ARCHIVER_PJ.mode,
          target: { aggregate: ARCHIVER_PJ.aggregate, id: pcId },
          payload: {},
          idempotencyKey: key
        },
        {
          onId: () => void 0,
          onPending: () => void 0,
          onSettled: (_commandId, result, violations) => {
            archiveKeys.delete(pcId);
            if (result.status === "applied") stop(null);
            else if (result.status === "rejected") stop({ kind: "rejected", violations: violationNotes(violations).all });
            else stop({ kind: result.status });
          },
          onTransport: () => {
            stop({ kind: "transport" });
          },
          onLink: () => void 0
        }
      );
    }
    function dispose() {
      if (disposed) return;
      disposed = true;
      stopAll();
      listeners.clear();
    }
    setCampagne(campagneId);
    return {
      getState: () => state,
      subscribe(listener) {
        listeners.add(listener);
        return () => {
          listeners.delete(listener);
        };
      },
      setCampagne,
      setAddField: (field, value) => {
        setField("add", field, value);
      },
      submitAdd,
      openEdit,
      setEditField: (field, value) => {
        setField("edit", field, value);
      },
      submitEdit,
      closeEdit,
      confirmEdit: () => actOnEdit("confirm_overwrite"),
      cancelEdit: () => actOnEdit("cancel"),
      archive,
      dispose
    };
  }

  // apps/campagne/pjs/src/view-model.ts
  var BANNER_TEXT = {
    none: null,
    idle: COPY.noCampaign,
    loading: COPY.loading,
    empty: COPY.empty,
    "not-found": COPY.notFound,
    error: COPY.readError,
    stale: COPY.stale
  };
  var KEY_LABELS = {
    name: COPY.labels.nom,
    class: COPY.labels.classe,
    level: COPY.labels.niveau
  };
  function valueLines(value) {
    if (value === null) return [];
    if (typeof value === "object" && !Array.isArray(value)) {
      return Object.entries(value).map(([key, v]) => ({
        label: Object.hasOwn(KEY_LABELS, key) ? KEY_LABELS[key] ?? key : key,
        text: typeof v === "string" ? v : JSON.stringify(v)
      }));
    }
    return [{ label: "", text: JSON.stringify(value) }];
  }
  function bannerOf(s) {
    const list = s.list;
    switch (list.kind) {
      case "idle":
      case "loading":
      case "not-found":
      case "error":
        return list.kind;
      case "rows":
        if (list.error) return "error";
        if (list.stale) return "stale";
        return list.rows.length === 0 ? "empty" : "none";
    }
  }
  function formView(f) {
    const busy = isBusy(f);
    const fieldErrors = { nom: [], classe: [], niveau: [] };
    for (const field of PC_FIELDS) fieldErrors[field] = f.fieldErrors[field] ?? [];
    const c = f.command;
    return {
      values: f.values,
      fieldErrors,
      formErrors: f.formErrors,
      busy,
      canSubmit: !busy,
      transportError: f.transportError,
      notice: c.kind === "done" && (c.status === "expired" || c.status === "cancelled") ? c.status : null,
      awaiting: c.kind === "awaiting" ? {
        yourValue: valueLines(c.yourValue),
        ahead: c.ahead === null ? null : valueLines(c.ahead),
        aheadField: c.aheadField,
        canConfirm: c.actions.includes("confirm_overwrite"),
        canCancel: c.actions.includes("cancel")
      } : null
    };
  }
  function editView(e) {
    const base = formView(e);
    return { ...base, pcId: e.pcId, canClose: e.command.kind !== "sending" };
  }
  function noteTexts(n) {
    if (n === void 0) return [];
    switch (n.kind) {
      case "rejected":
        return n.violations.map((v) => v.text);
      case "transport":
        return [COPY.transport];
      case "expired":
        return [COPY.expired];
      case "cancelled":
        return [COPY.cancelled];
    }
  }
  function viewOf(s) {
    const banner = bannerOf(s);
    const actionable = s.list.kind !== "idle" && s.list.kind !== "not-found";
    const editBusy = s.edit !== null && isBusy(s.edit);
    const rows = s.list.kind === "rows" ? s.list.rows.map((r) => {
      const archiving = s.archiving.has(r.id);
      return {
        id: r.id,
        name: r.name,
        class: r.class,
        level: r.level,
        canEdit: !archiving && !editBusy,
        canArchive: !archiving,
        archiving,
        notes: noteTexts(s.rowNotices[r.id])
      };
    }) : [];
    return {
      banner,
      bannerText: BANNER_TEXT[banner],
      rows,
      add: actionable ? formView(s.add) : null,
      edit: actionable && s.edit !== null ? editView(s.edit) : null
    };
  }

  // apps/campagne/pjs/src/mount.ts
  function el(doc, tag, attrs = {}, text) {
    const node = doc.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, v);
    if (text !== void 0) node.textContent = text;
    return node;
  }
  function buildForm(doc, scope, title, submitLabel) {
    const root = el(doc, "form", { class: `pjs-form pjs-form-${scope}` });
    root.noValidate = true;
    root.append(el(doc, "h3", {}, title));
    const inputs = {};
    const fieldErrors = {};
    for (const field of PC_FIELDS) {
      const id = `pjs-${scope}-${field}`;
      const wrap = el(doc, "div", { class: "pjs-field" });
      const input = el(doc, "input", { type: "text", name: field, id, autocomplete: "off" });
      if (field === "niveau") input.setAttribute("inputmode", "numeric");
      const errors = el(doc, "ul", { class: "pjs-errors", role: "alert" });
      wrap.append(el(doc, "label", { for: id }, COPY.labels[field]), input, errors);
      root.append(wrap);
      inputs[field] = input;
      fieldErrors[field] = errors;
    }
    const formErrors = el(doc, "ul", { class: "pjs-errors pjs-form-errors", role: "alert" });
    const status = el(doc, "p", { class: "pjs-status" });
    const submit = el(doc, "button", { type: "submit" }, submitLabel);
    root.append(formErrors, status, submit);
    return { root, inputs, fieldErrors, formErrors, status, submit };
  }
  function paintViolations(ul, notes) {
    const doc = ul.ownerDocument;
    ul.replaceChildren(
      ...notes.map((n) => {
        const li = el(doc, "li", {}, n.text);
        if (n.id !== null) li.dataset["violation"] = n.id;
        return li;
      })
    );
  }
  function paintForm(dom, v) {
    dom.root.hidden = v === null;
    if (v === null) return;
    for (const field of PC_FIELDS) {
      const input = dom.inputs[field];
      if (input.value !== v.values[field]) input.value = v.values[field];
      input.disabled = v.busy;
      input.setAttribute("aria-invalid", v.fieldErrors[field].length > 0 ? "true" : "false");
      paintViolations(dom.fieldErrors[field], v.fieldErrors[field]);
    }
    paintViolations(dom.formErrors, v.formErrors);
    dom.submit.disabled = !v.canSubmit;
    dom.status.textContent = v.busy && v.awaiting === null ? COPY.sending : noticeText(v) ?? "";
  }
  function noticeText(v) {
    if (v.transportError) return COPY.transport;
    if (v.notice === "expired") return COPY.expired;
    if (v.notice === "cancelled") return COPY.cancelled;
    return null;
  }
  function paintValueLines(ul, lines) {
    const doc = ul.ownerDocument;
    ul.replaceChildren(...lines.map((l) => el(doc, "li", {}, l.label === "" ? l.text : `${l.label} : ${l.text}`)));
  }
  function mount(host2, props2, deps) {
    const doc = host2.ownerDocument;
    const ctrl = createPjsController(props2.campagneId, deps);
    const root = el(doc, "section", { class: "pjs" });
    const banner = el(doc, "p", { class: "pjs-banner", role: "status" });
    const list = el(doc, "ul", { class: "pjs-rows" });
    const add = buildForm(doc, "add", COPY.addTitle, COPY.add);
    const edit = buildForm(doc, "edit", COPY.editTitle, COPY.save);
    const awaiting = el(doc, "div", { class: "pjs-awaiting", role: "alert" });
    const awaitingTitle = el(doc, "p", {}, COPY.awaitingTitle);
    const yours = el(doc, "ul", { class: "pjs-yours" });
    const ahead = el(doc, "ul", { class: "pjs-ahead" });
    const confirm = el(doc, "button", { type: "button" }, COPY.confirm);
    const cancel = el(doc, "button", { type: "button" }, COPY.cancel);
    awaiting.append(awaitingTitle, el(doc, "p", {}, COPY.leaveHint), el(doc, "h4", {}, COPY.yourValue), yours, el(doc, "h4", {}, COPY.aheadValue), ahead, confirm, cancel);
    const close = el(doc, "button", { type: "button" }, COPY.close);
    edit.root.insertBefore(awaiting, edit.formErrors);
    edit.root.append(close);
    root.append(el(doc, "h2", {}, COPY.title), banner, list, add.root, edit.root);
    host2.replaceChildren(root);
    for (const field of PC_FIELDS) {
      add.inputs[field].addEventListener("input", () => {
        ctrl.setAddField(field, add.inputs[field].value);
      });
      edit.inputs[field].addEventListener("input", () => {
        ctrl.setEditField(field, edit.inputs[field].value);
      });
    }
    add.root.addEventListener("submit", (e) => {
      e.preventDefault();
      void ctrl.submitAdd();
    });
    edit.root.addEventListener("submit", (e) => {
      e.preventDefault();
      void ctrl.submitEdit();
    });
    confirm.addEventListener("click", () => void ctrl.confirmEdit());
    cancel.addEventListener("click", () => void ctrl.cancelEdit());
    close.addEventListener("click", () => {
      ctrl.closeEdit();
    });
    let paintedRows = "";
    function paintRows(view) {
      const signature = JSON.stringify(view.rows);
      if (signature === paintedRows) return;
      paintedRows = signature;
      list.replaceChildren(
        ...view.rows.map((r) => {
          const li = el(doc, "li", { class: "pjs-row" });
          li.dataset["pc"] = r.id;
          li.append(el(doc, "span", { class: "pjs-name" }, r.name), el(doc, "span", { class: "pjs-class" }, r.class), el(doc, "span", { class: "pjs-level" }, String(r.level)));
          const editBtn = el(doc, "button", { type: "button" }, COPY.edit);
          editBtn.disabled = !r.canEdit;
          editBtn.addEventListener("click", () => {
            ctrl.openEdit(r.id);
          });
          const archiveBtn = el(doc, "button", { type: "button" }, r.archiving ? COPY.archiving : COPY.archive);
          archiveBtn.disabled = !r.canArchive;
          archiveBtn.addEventListener("click", () => void ctrl.archive(r.id));
          li.append(editBtn, archiveBtn, ...r.notes.map((n) => el(doc, "span", { class: "pjs-note" }, n)));
          return li;
        })
      );
    }
    function paintAwaiting(a) {
      awaiting.hidden = a === null;
      if (a === null) {
        yours.replaceChildren();
        ahead.replaceChildren();
        return;
      }
      paintValueLines(yours, a.yourValue);
      paintValueLines(ahead, a.ahead ?? []);
      confirm.hidden = !a.canConfirm;
      cancel.hidden = !a.canCancel;
    }
    function render(view) {
      banner.hidden = view.bannerText === null;
      banner.textContent = view.bannerText ?? "";
      banner.dataset["banner"] = view.banner;
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
        host2.replaceChildren();
      }
    };
  }

  // mock/src/screen.js
  var context = () => window.__mockContext = window.__mockContext || {};
  function setContext(name, value) {
    const ctx = context();
    if (ctx[name] === value) return;
    ctx[name] = value;
    document.dispatchEvent(new CustomEvent("mock:context", { detail: { name, value } }));
  }
  function getContext(name) {
    return context()[name];
  }
  function onContext(listener) {
    const handler = (e) => listener(e.detail.name, e.detail.value);
    document.addEventListener("mock:context", handler);
    return () => document.removeEventListener("mock:context", handler);
  }
  function mountPoint() {
    const script = document.currentScript;
    const component = script && script.closest("[data-component]");
    const host2 = component ? component.querySelector(".mf-root") || component : document.body;
    let props2 = {};
    try {
      props2 = JSON.parse(component && component.dataset.props || "{}");
    } catch (e) {
      props2 = {};
    }
    return { component, host: host2, props: props2 };
  }
  function resolveProps(raw) {
    const out = {};
    for (const [key, value] of Object.entries(raw || {})) {
      out[key] = typeof value === "string" && value.startsWith("$ctx.") ? getContext(value.slice(5)) : value;
    }
    return out;
  }
  function compose(raw, mount2) {
    const follows = Object.values(raw || {}).filter((v) => typeof v === "string" && v.startsWith("$ctx.")).map((v) => v.slice(5));
    const mounted = mount2(resolveProps(raw));
    if (follows.length) onContext((name) => {
      if (follows.includes(name) && mounted && mounted.update) mounted.update(resolveProps(raw));
    });
    return mounted;
  }

  // mock/src/commands.js
  var active = async (server) => (await server.load("campagnes")).filter((c) => !c.archived);
  var pcsOf = async (server, campagneId) => (await server.load("pjs")).filter((p) => p.campagneId === campagneId && !p.archived);
  var pcViolations = async (server, payload, campagneId, selfId) => {
    const found = [];
    const name = typeof payload.nom === "string" ? payload.nom : "";
    if (!name.trim()) found.push("pc-name-required");
    else if ((await pcsOf(server, campagneId)).some((p) => p.id !== selfId && p.name.trim().toLowerCase() === name.trim().toLowerCase())) found.push("pc-name-unique-in-campaign");
    if (typeof payload.classe !== "string" || !payload.classe.trim()) found.push("pc-class-required");
    if (!Number.isInteger(payload.niveau) || payload.niveau < 1 || payload.niveau > 20) found.push("pc-level-range");
    return found;
  };
  var COMMANDS = {
    "campagne.creerCampagne": async (server, { payload }) => {
      const name = typeof payload.name === "string" ? payload.name : "";
      if (!name.trim()) return ["campaign-name-required"];
      if (name.length > 80) return ["campaign-name-length"];
      (await server.load("campagnes")).push({ id: server.uuid(), name, createdAt: server.next(), archived: false });
    },
    "campagne.archiverCampagne": async (server, { targetId }) => {
      const campagne = (await active(server)).find((c) => c.id === targetId);
      if (!campagne) return "not-found";
      campagne.archived = true;
    },
    "campagne.ajouterPj": async (server, { payload }) => {
      const campagne = (await active(server)).find((c) => c.id === payload.campagneId);
      if (!campagne) return "not-found";
      const violations = await pcViolations(server, payload, campagne.id, null);
      if (violations.length) return violations;
      (await server.load("pjs")).push({ id: server.uuid(), campagneId: campagne.id, name: payload.nom, class: payload.classe, level: payload.niveau, createdAt: server.next(), archived: false });
    },
    "campagne.modifierPJ": async (server, { payload, targetId }) => {
      const pc = (await server.load("pjs")).find((p) => p.id === targetId && !p.archived);
      if (!pc) return "not-found";
      const violations = await pcViolations(server, payload, pc.campagneId, pc.id);
      if (violations.length) return violations;
      pc.name = payload.nom;
      pc.class = payload.classe;
      pc.level = payload.niveau;
    },
    "campagne.archiverPJ": async (server, { targetId }) => {
      const pc = (await server.load("pjs")).find((p) => p.id === targetId && !p.archived);
      if (!pc) return "not-found";
      pc.archived = true;
    }
  };

  // mock/src/reads.js
  var active2 = async (server) => (await server.load("campagnes")).filter((c) => !c.archived);
  var pcsOf2 = async (server, campagneId) => (await server.load("pjs")).filter((p) => p.campagneId === campagneId && !p.archived).sort((a, b) => a.createdAt - b.createdAt);
  var partyLevel = (levels) => {
    const n = levels.length;
    if (n === 0) return null;
    const sum = levels.reduce((a, b) => a + b, 0);
    return Math.floor((2 * sum + n) / (2 * n));
  };
  var READS = {
    "campagne.listerCampagnes": async (server) => (await active2(server)).sort((a, b) => b.createdAt - a.createdAt).map(({ id, name }) => ({ id, name })),
    "campagne.listerPjs": async (server, { campagneId }) => {
      if (!(await active2(server)).some((c) => c.id === campagneId)) return void 0;
      return (await pcsOf2(server, campagneId)).map(({ id, name, class: cls, level }) => ({ id, name, class: cls, level }));
    },
    "campagne.niveauDuGroupe": async (server, { campagneId }) => {
      if (!(await active2(server)).some((c) => c.id === campagneId)) return void 0;
      const rows = await pcsOf2(server, campagneId);
      return { level: partyLevel(rows.map((r) => r.level)), pcCount: rows.length, model: "v1", asOf: server.dataVersion };
    }
  };

  // mock/src/server.js
  var LATENCY_MS = 120;
  var delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  var uuid = () => crypto.randomUUID ? crypto.randomUUID() : "xxxxxxxx-xxxx-4xxx-8xxx-xxxxxxxxxxxx".replace(/x/g, () => Math.floor(Math.random() * 16).toString(16));
  var json = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  var notFound = () => json({ error: "not-found" }, 404);
  var same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  function mockServer() {
    if (window.__mockServer) return window.__mockServer;
    const s = { dataVersion: 1, fixtures: /* @__PURE__ */ new Map(), listeners: /* @__PURE__ */ new Set(), commands: /* @__PURE__ */ new Map(), tick: 1e3 };
    s.load = async (name) => {
      if (!s.fixtures.has(name)) {
        s.fixtures.set(name, fetch(new URL(`../fixtures/${name}.json`, location.href)).then((r) => r.ok ? r.json() : void 0).catch(() => void 0));
      }
      return s.fixtures.get(name);
    };
    s.bump = () => {
      s.dataVersion += 1;
      for (const listener of s.listeners) listener({ data: JSON.stringify({ dataVersion: s.dataVersion }) });
    };
    s.uuid = uuid;
    s.next = () => s.tick++;
    const read = async (identifier, variables) => {
      const handler = READS[identifier];
      if (handler) return handler(s, variables);
      const fixture = await s.load(identifier);
      if (fixture === void 0) return void 0;
      if (fixture && typeof fixture === "object" && Array.isArray(fixture.$match)) {
        const hit = fixture.$match.find((m) => Object.entries(m.variables || {}).every(([k, v]) => same(variables[k], v)));
        return hit ? hit.data : fixture.data;
      }
      return fixture;
    };
    const command = async (body) => {
      const [dataCapability, version] = String(body.dataCapability || "").split("@");
      const payload = body.payload || {};
      const targetId = body.target && body.target.id;
      const handler = COMMANDS[dataCapability];
      let violations = [];
      if (handler) {
        const verdict = await handler(s, { payload, targetId, version: Number(version) || 1, basedOn: body.basedOn });
        if (verdict === "not-found") return notFound();
        violations = Array.isArray(verdict) ? verdict : verdict && verdict.violations || [];
      }
      const commandId = uuid();
      const status = violations.length ? "rejected" : "applied";
      if (status === "applied") s.bump();
      const result = { commandId, status, dataVersion: status === "applied" ? s.dataVersion : null, violations, reviewId: null };
      s.commands.set(commandId, result);
      return json({ commandId, partition: `${dataCapability}/${targetId || commandId}`, replayed: false, warnings: [], result }, 202);
    };
    s.fetch = async (input, init) => {
      await delay(LATENCY_MS);
      const url = new URL(typeof input === "string" ? input : input instanceof URL ? input.href : input.url);
      const method = init && init.method || "GET";
      const body = init && typeof init.body === "string" ? JSON.parse(init.body) : {};
      if (method === "POST" && url.pathname.startsWith("/capabilities/")) {
        const data = await read(decodeURIComponent(url.pathname.slice("/capabilities/".length)), body.variables || {});
        return data === void 0 ? notFound() : json({ asOf: s.dataVersion, data });
      }
      if (method === "POST" && url.pathname === "/commands") return command(body);
      const lookup = /^\/commands\/([^/]+)$/.exec(url.pathname);
      if (lookup && method === "GET") {
        const result = s.commands.get(decodeURIComponent(lookup[1]));
        return result ? json({ result }) : notFound();
      }
      return notFound();
    };
    s.eventSource = () => {
      const source = { readyState: 1, addEventListener(type, listener) {
        if (type === "dataVersion") s.listeners.add(listener);
      }, close() {
        source.readyState = 2;
      } };
      return source;
    };
    window.__mockServer = s;
    return s;
  }
  function shellOnce(create) {
    if (!window.__mockShell) window.__mockShell = create(mockServer());
    return window.__mockShell;
  }

  // mock/src/shell.js
  var shell = () => shellOnce((server) => createShellClient({ baseUrl: "http://127.0.0.1:7878", fetch: server.fetch, eventSource: server.eventSource }));
  async function defaultCampagne() {
    const server = mockServer();
    const first = (await server.load("campagnes")).filter((c) => !c.archived).sort((a, b) => b.createdAt - a.createdAt)[0];
    return first ? first.id : null;
  }

  // mock/src/Pjs.js
  var { host, props } = mountPoint();
  (async () => {
    if (resolveProps(props).campagneId === void 0) setContext("campagne", await defaultCampagne());
    compose(props, (p) => mount(host, { campagneId: p.campagneId === void 0 ? null : p.campagneId }, { shell: shell() }));
  })();
})();

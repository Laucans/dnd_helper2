"use strict";
(() => {
  // apps/campagne/niveau-du-groupe/src/identifiers.ts
  var NIVEAU_DU_GROUPE = "campagne.niveauDuGroupe";

  // apps/campagne/niveau-du-groupe/src/party-level.ts
  var isCount = (value) => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
  var isLevel = (value) => typeof value === "number" && Number.isInteger(value) && value >= 1 && value <= 20;
  function parsePartyLevel(data) {
    if (typeof data !== "object" || data === null || Array.isArray(data)) return null;
    if (!Object.hasOwn(data, "level")) return null;
    const { level, pcCount, model, asOf } = data;
    if (level !== null && !isLevel(level)) return null;
    if (!isCount(pcCount) || !isCount(asOf)) return null;
    if (model !== "v1") return null;
    if (level === null !== (pcCount === 0)) return null;
    return { level, pcCount, model, asOf };
  }

  // apps/campagne/niveau-du-groupe/src/controller.ts
  function sameVisible(a, b) {
    if (a.kind !== "ready" || b.kind !== "ready") return a.kind === b.kind;
    return a.level === b.level && a.pcCount === b.pcCount && a.possiblyStale === b.possiblyStale;
  }
  var PartyLevelController = class {
    constructor(host2, onChange) {
      this.host = host2;
      this.onChange = onChange;
    }
    current = { kind: "idle" };
    campagneId;
    /** Bumped on every campaign switch and on dispose: an older listener is ignored. */
    generation = 0;
    release = null;
    disposed = false;
    state() {
      return this.current;
    }
    /** `undefined` and `''` make no read. The previous campaign's value is dropped at once. */
    setCampagneId(id) {
      if (this.disposed) return;
      const next = id === "" ? void 0 : id;
      if (next === this.campagneId) return;
      this.campagneId = next;
      this.stop();
      if (next === void 0) {
        this.set({ kind: "idle" });
        return;
      }
      this.set({ kind: "loading" });
      const generation = this.generation;
      let release;
      try {
        release = this.host.watch(NIVEAU_DU_GROUPE, { campagneId: next }, (event) => {
          this.receive(generation, event);
        });
      } catch {
        if (generation === this.generation) this.set({ kind: "unavailable" });
        return;
      }
      if (generation === this.generation) this.release = release;
      else release();
    }
    /** The subscription is released and nothing a pending read brings back has any effect. */
    dispose() {
      if (this.disposed) return;
      this.disposed = true;
      this.stop();
    }
    stop() {
      this.generation += 1;
      const release = this.release;
      this.release = null;
      release?.();
    }
    receive(generation, event) {
      if (generation !== this.generation) return;
      switch (event.kind) {
        case "not-found":
          this.set({ kind: "not-found" });
          return;
        case "error":
        case "stale":
          this.fail();
          return;
        case "data": {
          const result = parsePartyLevel(event.data);
          if (result === null) {
            this.fail();
            return;
          }
          const asOf = event.asOf ?? result.asOf;
          if (this.current.kind === "ready" && asOf < this.current.asOf) return;
          this.set({ kind: "ready", level: result.level, pcCount: result.pcCount, asOf, possiblyStale: false });
          return;
        }
      }
    }
    /**
     * Never `0` and never "no party level": either would claim a fact the view does not know. A kept
     * value is marked; "not found" stays (an archived campaign does not come back); otherwise unavailable.
     */
    fail() {
      const current = this.current;
      if (current.kind === "ready") this.set({ ...current, possiblyStale: true });
      else if (current.kind !== "not-found") this.set({ kind: "unavailable" });
    }
    set(next) {
      const changed = !sameVisible(this.current, next);
      this.current = next;
      if (changed) this.onChange(next);
    }
  };

  // apps/campagne/niveau-du-groupe/src/view-model.ts
  var COPY = {
    loading: "Chargement\u2026",
    noLevel: "Pas de niveau de groupe",
    campaignUnavailable: "Campagne indisponible",
    levelUnavailable: "Niveau du groupe indisponible",
    stale: "peut-\xEAtre pas \xE0 jour"
  };
  var countText = (pcCount) => `${String(pcCount)} PJ`;
  function toView(state) {
    switch (state.kind) {
      case "idle":
      case "not-found":
        return { kind: "unavailable", text: COPY.campaignUnavailable };
      case "unavailable":
        return { kind: "unavailable", text: COPY.levelUnavailable };
      case "loading":
        return { kind: "loading", text: COPY.loading };
      case "ready":
        if (state.level === null) return { kind: "no-level", text: COPY.noLevel, count: countText(state.pcCount), stale: state.possiblyStale };
        return { kind: "level", level: String(state.level), count: countText(state.pcCount), stale: state.possiblyStale };
    }
  }

  // apps/campagne/niveau-du-groupe/src/mount.ts
  function slots(view) {
    switch (view.kind) {
      case "loading":
      case "unavailable":
        return { level: view.text, count: "", note: "" };
      case "no-level":
        return { level: view.text, count: view.count, note: view.stale ? COPY.stale : "" };
      case "level":
        return { level: view.level, count: view.count, note: view.stale ? COPY.stale : "" };
    }
  }
  function mount(host2, props2, deps) {
    const doc = host2.ownerDocument;
    const section = doc.createElement("section");
    section.dataset["microUi"] = "NiveauDuGroupe";
    section.setAttribute("aria-live", "polite");
    const slot = (name) => {
      const element = doc.createElement("span");
      element.dataset["slot"] = name;
      section.append(element);
      return element;
    };
    const levelSlot = slot("level");
    const countSlot = slot("count");
    const noteSlot = slot("note");
    const write = (element, text) => {
      if (element.textContent !== text) element.textContent = text;
    };
    const render = (state) => {
      const view = toView(state);
      const text = slots(view);
      section.dataset["state"] = view.kind;
      write(levelSlot, text.level);
      write(countSlot, text.count);
      write(noteSlot, text.note);
    };
    const controller = new PartyLevelController(deps.shell, render);
    render(controller.state());
    host2.append(section);
    controller.setCampagneId(props2.campagneId);
    return {
      update(next) {
        controller.setCampagneId(next.campagneId);
      },
      unmount() {
        controller.dispose();
        section.remove();
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

  // mock/src/NiveauDuGroupe.js
  var { host, props } = mountPoint();
  (async () => {
    if (resolveProps(props).campagneId === void 0) setContext("campagne", await defaultCampagne());
    compose(props, (p) => mount(host, { campagneId: p.campagneId === null ? void 0 : p.campagneId }, { shell: shell() }));
  })();
})();

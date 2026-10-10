import { createHash } from "node:crypto";

const OPS = globalThis[Symbol.for("bb.host.ops")];
const plugins = new Map();

globalThis.addEventListener("unhandledrejection", (event) => {
  event.preventDefault();
  const reason = event.reason;
  OPS.op_bb_log("error", "host", `unhandled rejection: ${reason?.message ?? reason}`);
});

class SqliteError extends Error {
  constructor(message, code) {
    super(message);
    this.name = "SqliteError";
    this.code = code;
  }
}

function sqliteCall(fn) {
  try {
    return fn();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const match = /^(SQLITE_[A-Z_]+)\|(.*)$/s.exec(message);
    if (match) throw new SqliteError(match[2], match[1]);
    throw error;
  }
}

function encodeParams(args) {
  if (
    args.length === 1 &&
    args[0] !== null &&
    typeof args[0] === "object" &&
    !Array.isArray(args[0]) &&
    !(args[0] instanceof Uint8Array)
  ) {
    return JSON.stringify({ named: normalizeValues(args[0]) });
  }
  return JSON.stringify({ positional: args.flat().map(normalizeValue) });
}

function normalizeValue(value) {
  if (typeof value === "boolean") return value ? 1 : 0;
  if (typeof value === "bigint") return Number(value);
  if (value === undefined) return null;
  return value;
}

function normalizeValues(record) {
  return Object.fromEntries(
    Object.entries(record).map(([key, value]) => [key, normalizeValue(value)]),
  );
}

class Statement {
  #db;
  #pluck = false;
  #raw = false;
  constructor(db, source) {
    this.#db = db;
    this.source = source;
    this.reader = /^\s*(select|pragma|with|values|explain)\b/i.test(source) ||
      /\breturning\b/i.test(source);
  }
  run(...args) {
    return JSON.parse(
      sqliteCall(() =>
        OPS.op_bb_db_exec(this.#db.handle, this.source, encodeParams(args))
      ),
    );
  }
  all(...args) {
    const rows = JSON.parse(
      sqliteCall(() =>
        OPS.op_bb_db_query(this.#db.handle, this.source, encodeParams(args))
      ),
    );
    if (this.#pluck) return rows.map((row) => Object.values(row)[0]);
    if (this.#raw) return rows.map((row) => Object.values(row));
    return rows;
  }
  get(...args) {
    return this.all(...args)[0];
  }
  iterate(...args) {
    return this.all(...args)[Symbol.iterator]();
  }
  pluck(toggle = true) {
    this.#pluck = toggle;
    return this;
  }
  raw(toggle = true) {
    this.#raw = toggle;
    return this;
  }
  bind() {
    throw new Error("Statement.bind is not supported by the spike shim");
  }
}

class Database {
  constructor(handle, name) {
    this.handle = handle;
    this.name = name;
    this.open = true;
    this.inTransaction = false;
    this.memory = false;
    this.readonly = false;
  }
  prepare(source) {
    return new Statement(this, source);
  }
  exec(source) {
    sqliteCall(() => OPS.op_bb_db_exec(this.handle, source, "null"));
    return this;
  }
  pragma(source, options) {
    const rows = this.prepare(`PRAGMA ${source}`).all();
    if (options?.simple) {
      const first = rows[0];
      return first === undefined ? undefined : Object.values(first)[0];
    }
    return rows;
  }
  transaction(fn) {
    const db = this;
    let depth = 0;
    const wrapped = function (...args) {
      const nested = db.inTransaction;
      const savepoint = `bb_sp_${depth++}`;
      db.exec(nested ? `SAVEPOINT ${savepoint}` : "BEGIN");
      db.inTransaction = true;
      try {
        const result = fn.apply(this, args);
        db.exec(nested ? `RELEASE ${savepoint}` : "COMMIT");
        return result;
      } catch (error) {
        db.exec(
          nested
            ? `ROLLBACK TO ${savepoint}; RELEASE ${savepoint}`
            : "ROLLBACK",
        );
        throw error;
      } finally {
        depth--;
        if (!nested) db.inTransaction = false;
      }
    };
    wrapped.deferred = wrapped;
    wrapped.immediate = wrapped;
    wrapped.exclusive = wrapped;
    return wrapped;
  }
  close() {
    this.open = false;
    return this;
  }
  function() {
    throw new Error("Database.function is not supported by the spike shim");
  }
}

function runMigrations(database, statements) {
  database.exec(
    "CREATE TABLE IF NOT EXISTS _bb_migrations (id INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL, statement_hash TEXT)",
  );
  const rows = database
    .prepare("SELECT id, statement_hash FROM _bb_migrations ORDER BY id")
    .all();
  const applied = new Map(rows.map((row) => [row.id, row.statement_hash]));
  const hashes = statements.map((statement) =>
    createHash("sha256").update(statement).digest("hex")
  );
  hashes.forEach((hash, index) => {
    const recorded = applied.get(index);
    if (recorded != null && recorded !== hash) {
      throw new Error(`migration ${index} does not match the recorded statement`);
    }
  });
  const record = database.prepare(
    "INSERT INTO _bb_migrations (id, applied_at, statement_hash) VALUES (?, ?, ?)",
  );
  database.transaction(() => {
    statements.forEach((statement, index) => {
      if (applied.has(index)) return;
      database.exec(statement);
      record.run(index, Date.now(), hashes[index]);
    });
  })();
}

function stripFunctions(value) {
  return JSON.parse(
    JSON.stringify(value, (_key, item) =>
      typeof item === "function" ? `[function ${item.name || "anonymous"}]` : item
    ),
  );
}

function sdkProxy(pluginId, path) {
  return new Proxy(function () {}, {
    get(_target, key) {
      if (key === "then") return undefined;
      if (typeof key === "symbol") return undefined;
      return sdkProxy(pluginId, [...path, key]);
    },
    async apply(_target, _thisArg, args) {
      const reply = await OPS.op_bb_sdk_call(
        pluginId,
        path.join("."),
        JSON.stringify(args ?? []),
      );
      return JSON.parse(reply);
    },
  });
}

function unimplemented(pluginId, namespace) {
  return new Proxy(
    {},
    {
      get(_target, key) {
        if (typeof key === "symbol" || key === "then") return undefined;
        return (...args) => {
          OPS.op_bb_record(
            pluginId,
            "unimplemented",
            JSON.stringify({ api: `${namespace}.${String(key)}`, args: stripFunctions(args) }),
          );
          return { dispose() {} };
        };
      },
    },
  );
}

async function validateWith(schema, value, label) {
  const standard = schema?.["~standard"];
  if (!standard) return value;
  const result = await standard.validate(value);
  if (result.issues) {
    throw new Error(
      `${label} failed validation: ${result.issues
        .map((issue) => `${(issue.path ?? []).map((p) => p?.key ?? p).join(".")}: ${issue.message}`)
        .join("; ")}`,
    );
  }
  return result.value;
}

function createApi(pluginId) {
  const state = {
    rpc: new Map(),
    cli: null,
    services: [],
    schedules: [],
    events: new Map(),
    settingsDescriptors: {},
    disposers: [],
    controllers: [],
  };
  const record = (kind, value) =>
    OPS.op_bb_record(pluginId, kind, JSON.stringify(stripFunctions(value ?? null)));
  const log = Object.fromEntries(
    ["debug", "info", "warn", "error"].map((level) => [
      level,
      (message) => OPS.op_bb_log(level, pluginId, String(message)),
    ]),
  );
  const settingsNamespace = `${pluginId}#settings`;
  const readSettings = async () => {
    const values = {};
    for (const [key, descriptor] of Object.entries(state.settingsDescriptors)) {
      const stored = await OPS.op_bb_kv_get(settingsNamespace, key);
      values[key] = stored === null ? descriptor.default : JSON.parse(stored);
    }
    return values;
  };
  const kv = {
    async get(key) {
      const stored = await OPS.op_bb_kv_get(pluginId, key);
      return stored === null ? undefined : JSON.parse(stored);
    },
    async set(key, value) {
      await OPS.op_bb_kv_set(pluginId, key, JSON.stringify(value));
    },
    async delete(key) {
      await OPS.op_bb_kv_delete(pluginId, key);
    },
    async list(prefix) {
      return JSON.parse(await OPS.op_bb_kv_list(pluginId, prefix ?? ""));
    },
  };
  let database;
  const api = {
    pluginId,
    log,
    settings: {
      define(descriptors) {
        Object.assign(state.settingsDescriptors, descriptors);
        record("settings", descriptors);
        const listeners = [];
        return {
          get: readSettings,
          async experimental_set(values) {
            const previous = await readSettings();
            for (const [key, value] of Object.entries(values)) {
              if (value === null) await OPS.op_bb_kv_delete(settingsNamespace, key);
              else await OPS.op_bb_kv_set(settingsNamespace, key, JSON.stringify(value));
            }
            const next = await readSettings();
            for (const listener of listeners) listener(next, previous);
            return next;
          },
          onChange(listener) {
            listeners.push(listener);
          },
        };
      },
    },
    storage: {
      kv,
      database() {
        if (database?.open) return database;
        const info = JSON.parse(OPS.op_bb_db_open(pluginId));
        database = new Database(info.handle, info.path);
        database.pragma("journal_mode = WAL");
        database.pragma("busy_timeout = 5000");
        return database;
      },
      migrate: runMigrations,
    },
    rpc: {
      register(contract, handlers, options) {
        for (const [name, handler] of Object.entries(handlers)) {
          if (!(name in contract)) throw new Error(`rpc handler ${name} is not in the contract`);
          state.rpc.set(name, { handler, schema: contract[name] });
        }
        record("rpc", { methods: Object.keys(handlers), options: options ?? null });
      },
    },
    realtime: {
      publish(channel, payload) {
        record("realtime", { channel, payload });
      },
    },
    background: {
      service(name, service) {
        state.services.push({ name, service });
        record("service", { name });
      },
      schedule(name, cron, fn) {
        state.schedules.push({ name, cron, fn });
        record("schedule", { name, cron });
      },
    },
    cli: {
      register(registration) {
        if (state.cli) throw new Error("cli.register may only be called once");
        state.cli = registration;
        record("cli", {
          name: registration.name,
          summary: registration.summary,
          commands: registration.commands ? Object.keys(registration.commands) : null,
        });
      },
    },
    providers: {
      register(declaration) {
        record("provider", declaration);
        return { dispose() {} };
      },
      experimental_contributeEnv(providerId) {
        record("provider-env", { providerId });
      },
      experimental_contributeEnvHealth(providerId) {
        record("provider-env-health", { providerId });
      },
    },
    events: {
      on(event, handler) {
        const list = state.events.get(event) ?? [];
        list.push(handler);
        state.events.set(event, list);
        record("event-subscription", { event });
      },
    },
    agents: unimplemented(pluginId, "agents"),
    ui: unimplemented(pluginId, "ui"),
    http: unimplemented(pluginId, "http"),
    experimental_hooks: unimplemented(pluginId, "experimental_hooks"),
    experimental_environments: unimplemented(pluginId, "experimental_environments"),
    experimental_machines: unimplemented(pluginId, "experimental_machines"),
    experimental_serverAccess: unimplemented(pluginId, "experimental_serverAccess"),
    experimental_aiServices: unimplemented(pluginId, "experimental_aiServices"),
    status: unimplemented(pluginId, "status"),
    server: {
      experimental_dataDir: OPS.op_bb_data_dir(),
      ...Object.fromEntries(
        ["info", "experimental_info"].map((key) => [key, unimplemented(pluginId, "server")[key]]),
      ),
    },
    hosts: {
      experimental_client(contract) {
        record("host-client", { methods: Object.keys(contract?.methods ?? contract ?? {}) });
        return new Proxy(
          {
            experimental_onWorkerExit: () => () => {},
            experimental_onSignal: () => () => {},
          },
          {
            get(target, key) {
              if (key in target) return target[key];
              if (typeof key === "symbol" || key === "then") return undefined;
              return async () => {
                throw new Error(`host rpc ${String(key)} is not served by the spike host`);
              };
            },
          },
        );
      },
      list: async () => [],
      get: async () => null,
    },
    sdk: sdkProxy(pluginId, []),
    onDispose(hook) {
      state.disposers.push(hook);
    },
    onInstall(handler) {
      record("on-install", null);
      void handler;
    },
  };
  return { api, state };
}

export async function loadPlugin(pluginId, entryUrl) {
  const started = performance.now();
  const mod = await import(entryUrl);
  const factory = mod.default;
  if (typeof factory !== "function") {
    throw new Error(`server entry must default-export a factory, got ${typeof factory}`);
  }
  const { api, state } = createApi(pluginId);
  await factory(api);
  plugins.set(pluginId, { api, state });
  return {
    loadMs: Math.round((performance.now() - started) * 100) / 100,
    rpc: [...state.rpc.keys()],
    services: state.services.map((service) => service.name),
    schedules: state.schedules.map((schedule) => schedule.name),
    events: [...state.events.keys()],
    cli: state.cli?.name ?? null,
  };
}

function requirePlugin(pluginId) {
  const plugin = plugins.get(pluginId);
  if (!plugin) throw new Error(`plugin ${pluginId} is not loaded`);
  return plugin;
}

export async function invokeRpc(pluginId, method, input) {
  const { state } = requirePlugin(pluginId);
  const entry = state.rpc.get(method);
  if (!entry) throw new Error(`unknown rpc method ${pluginId}.${method}`);
  const parsed = await validateWith(entry.schema.input, input, `${method} input`);
  const output = await entry.handler(parsed, {
    experimental_caller: { kind: "client" },
  });
  return validateWith(entry.schema.output, output, `${method} output`);
}

export async function invokeCli(pluginId, argv, context) {
  const { state } = requirePlugin(pluginId);
  if (!state.cli) throw new Error(`plugin ${pluginId} has no cli`);
  try {
    return await state.cli.run(argv, context);
  } catch (error) {
    return {
      thrown: true,
      name: error?.name,
      code: error?.code,
      message: error?.message,
      hint: error?.hint,
      exitCode: error?.exitCode,
    };
  }
}

export async function emitEvent(pluginId, event, payload) {
  const { state } = requirePlugin(pluginId);
  for (const handler of state.events.get(event) ?? []) await handler(payload);
  return (state.events.get(event) ?? []).length;
}

export function startServices(pluginId) {
  const { state, api } = requirePlugin(pluginId);
  for (const { name, service } of state.services) {
    const controller = new AbortController();
    state.controllers.push(controller);
    const startedAt = performance.now();
    Promise.resolve()
      .then(() => service.start(controller.signal))
      .then(
        () =>
          api.log.info(
            `service ${name} exited after ${Math.round(performance.now() - startedAt)}ms (aborted=${controller.signal.aborted})`,
          ),
        (error) => api.log.error(`service ${name} crashed: ${error?.message ?? error}`),
      );
  }
  return state.services.length;
}

export function stopServices(pluginId) {
  const { state } = requirePlugin(pluginId);
  for (const controller of state.controllers.splice(0)) controller.abort();
  return true;
}

export async function runSchedule(pluginId, name) {
  const { state } = requirePlugin(pluginId);
  const schedule = state.schedules.find((entry) => entry.name === name);
  if (!schedule) throw new Error(`unknown schedule ${name}`);
  await schedule.fn();
  return true;
}

export async function probeNode() {
  const results = {};
  const probe = async (label, fn) => {
    try {
      results[label] = await fn();
    } catch (error) {
      results[label] = `ERROR: ${error?.message ?? error}`;
    }
  };
  await probe("process.version", () => process.version);
  await probe("Buffer", () => Buffer.from("bb").toString("hex"));
  await probe("os.homedir", async () => (await import("node:os")).homedir().length > 0);
  await probe("child_process.execFile", async () => {
    const { execFile } = await import("node:child_process");
    const { promisify } = await import("node:util");
    const { stdout } = await promisify(execFile)("uname", ["-s"]);
    return stdout.trim();
  });
  await probe("child_process.spawn+stdio", async () => {
    const { spawn } = await import("node:child_process");
    return await new Promise((resolve, reject) => {
      const child = spawn("cat", [], { stdio: ["pipe", "pipe", "inherit"] });
      let out = "";
      child.stdout.setEncoding("utf8");
      child.stdout.on("data", (chunk) => (out += chunk));
      child.on("error", reject);
      child.on("close", (code) => resolve({ code, out }));
      child.stdin.end("ping\n");
    });
  });
  await probe("fs.promises", async () => {
    const fs = await import("node:fs/promises");
    const os = await import("node:os");
    const path = await import("node:path");
    const dir = await fs.mkdtemp(path.join(os.tmpdir(), "bb-spike-"));
    await fs.writeFile(path.join(dir, "a.txt"), "hello");
    const text = await fs.readFile(path.join(dir, "a.txt"), "utf8");
    await fs.rm(dir, { recursive: true });
    return text;
  });
  await probe("crypto.randomUUID", async () => (await import("node:crypto")).randomUUID().length);
  await probe("fetch", () => typeof fetch);
  await probe("createRequire(node:path)", async () => {
    const { createRequire } = await import("node:module");
    const require = createRequire(import.meta.url);
    return typeof require("node:path").join;
  });
  return results;
}

export async function sweepPlugin(pluginId, serverUrl, hostUrl) {
  const report = { pluginId };
  const timeout = (label) =>
    new Promise((_, reject) => setTimeout(() => reject(new Error(`${label} timed out after 5000ms`)), 5000));
  if (serverUrl) {
    try {
      report.server = await Promise.race([loadPlugin(pluginId, serverUrl), timeout("server load")]);
    } catch (error) {
      report.serverError = String(error?.stack ?? error).split("\n").slice(0, 4).join(" | ");
    }
  }
  if (hostUrl) {
    try {
      const started = performance.now();
      const mod = await Promise.race([import(hostUrl), timeout("host import")]);
      report.host = {
        importMs: Math.round(performance.now() - started),
        exports: Object.keys(mod),
        bridge: mod.experimental_providerBridge?.experimental_apiVersion ?? null,
      };
    } catch (error) {
      report.hostError = String(error?.stack ?? error).split("\n").slice(0, 4).join(" | ");
    }
  }
  return report;
}

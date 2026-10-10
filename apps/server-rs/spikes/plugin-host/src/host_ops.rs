use std::path::PathBuf;

use deno_error::JsErrorBox;
use deno_runtime::deno_core::{op2, JsRuntime, OpState};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{Map, Value};

pub struct HostState {
    data_dir: PathBuf,
    kv: Connection,
    databases: Vec<(String, Connection)>,
    records: Vec<String>,
}

impl HostState {
    pub fn open(data_dir: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&data_dir)?;
        let kv = Connection::open(data_dir.join("host.db"))?;
        kv.execute_batch(
            "PRAGMA journal_mode = WAL; CREATE TABLE IF NOT EXISTS plugin_kv (plugin_id TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY (plugin_id, key));",
        )?;
        Ok(Self {
            data_dir,
            kv,
            databases: Vec::new(),
            records: Vec::new(),
        })
    }
}

deno_runtime::deno_core::extension!(
    bb_host,
    ops = [
        op_bb_data_dir,
        op_bb_log,
        op_bb_record,
        op_bb_kv_get,
        op_bb_kv_set,
        op_bb_kv_delete,
        op_bb_kv_list,
        op_bb_db_open,
        op_bb_db_exec,
        op_bb_db_query,
        op_bb_sdk_call,
    ],
    esm_entry_point = "ext:bb_host/bootstrap.js",
    esm = [dir "js", "bootstrap.js"],
    options = { state: HostState },
    state = |state, options| {
        state.put(options.state);
    },
);

pub fn take_records(runtime: &mut JsRuntime) -> Vec<String> {
    let op_state = runtime.op_state();
    let mut op_state = op_state.borrow_mut();
    std::mem::take(&mut op_state.borrow_mut::<HostState>().records)
}

fn box_err(error: impl std::fmt::Display) -> JsErrorBox {
    JsErrorBox::generic(error.to_string())
}

fn sqlite_err(error: rusqlite::Error) -> JsErrorBox {
    let code = match &error {
        rusqlite::Error::SqliteFailure(inner, _) => format!("SQLITE_{:?}", inner.code)
            .to_uppercase()
            .replace("CONSTRAINTVIOLATION", "CONSTRAINT"),
        _ => "SQLITE_ERROR".to_string(),
    };
    JsErrorBox::generic(format!("{code}|{error}"))
}

#[op2]
#[string]
fn op_bb_data_dir(state: &mut OpState) -> String {
    state.borrow::<HostState>().data_dir.display().to_string()
}

#[op2(fast)]
fn op_bb_log(#[string] level: &str, #[string] plugin: &str, #[string] message: &str) {
    eprintln!("  [plugin:{plugin}] {level}: {message}");
}

#[op2(fast)]
fn op_bb_record(state: &mut OpState, #[string] plugin: &str, #[string] kind: &str, #[string] json: &str) {
    state
        .borrow_mut::<HostState>()
        .records
        .push(format!("{plugin} {kind} {json}"));
}

#[op2]
#[string]
fn op_bb_kv_get(
    state: &mut OpState,
    #[string] plugin: &str,
    #[string] key: &str,
) -> Result<Option<String>, JsErrorBox> {
    state
        .borrow::<HostState>()
        .kv
        .query_row(
            "SELECT value FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
            params![plugin, key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(box_err)
}

#[op2(fast)]
fn op_bb_kv_set(
    state: &mut OpState,
    #[string] plugin: &str,
    #[string] key: &str,
    #[string] value: &str,
) -> Result<(), JsErrorBox> {
    state
        .borrow::<HostState>()
        .kv
        .execute(
            "INSERT INTO plugin_kv (plugin_id, key, value) VALUES (?1, ?2, ?3) ON CONFLICT (plugin_id, key) DO UPDATE SET value = excluded.value",
            params![plugin, key, value],
        )
        .map(|_| ())
        .map_err(box_err)
}

#[op2(fast)]
fn op_bb_kv_delete(
    state: &mut OpState,
    #[string] plugin: &str,
    #[string] key: &str,
) -> Result<(), JsErrorBox> {
    state
        .borrow::<HostState>()
        .kv
        .execute(
            "DELETE FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
            params![plugin, key],
        )
        .map(|_| ())
        .map_err(box_err)
}

#[op2]
#[string]
fn op_bb_kv_list(
    state: &mut OpState,
    #[string] plugin: &str,
    #[string] prefix: &str,
) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostState>();
    let mut statement = host
        .kv
        .prepare("SELECT key FROM plugin_kv WHERE plugin_id = ?1 AND substr(key, 1, length(?2)) = ?2 ORDER BY key")
        .map_err(box_err)?;
    let keys = statement
        .query_map(params![plugin, prefix], |row| row.get::<_, String>(0))
        .map_err(box_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(box_err)?;
    serde_json::to_string(&keys).map_err(box_err)
}

#[op2]
#[string]
fn op_bb_db_open(state: &mut OpState, #[string] plugin: &str) -> Result<String, JsErrorBox> {
    let host = state.borrow_mut::<HostState>();
    let dir = host.data_dir.join("plugins").join(plugin);
    std::fs::create_dir_all(&dir).map_err(box_err)?;
    let path = dir.join("data.db");
    let connection = Connection::open(&path).map_err(box_err)?;
    host.databases.push((plugin.to_string(), connection));
    Ok(serde_json::json!({
        "handle": host.databases.len() - 1,
        "path": path.display().to_string(),
    })
    .to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Params {
    Named(Map<String, Value>),
    Positional(Vec<Value>),
}

fn to_sql(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Bool(flag) => SqlValue::Integer(i64::from(*flag)),
        Value::Number(number) => match number.as_i64() {
            Some(int) => SqlValue::Integer(int),
            None => SqlValue::Real(number.as_f64().unwrap_or(0.0)),
        },
        Value::String(text) => SqlValue::Text(text.clone()),
        other => SqlValue::Text(other.to_string()),
    }
}

fn bind(statement: &mut rusqlite::Statement<'_>, params: &Params) -> Result<(), JsErrorBox> {
    match params {
        Params::Positional(values) => {
            for (index, value) in values.iter().enumerate() {
                statement
                    .raw_bind_parameter(index + 1, to_sql(value))
                    .map_err(sqlite_err)?;
            }
        }
        Params::Named(map) => {
            for index in 1..=statement.parameter_count() {
                let Some(name) = statement.parameter_name(index) else {
                    continue;
                };
                let key = name.trim_start_matches(['@', ':', '$']).to_string();
                let value = map.get(&key).cloned().unwrap_or(Value::Null);
                statement
                    .raw_bind_parameter(index, to_sql(&value))
                    .map_err(sqlite_err)?;
            }
        }
    }
    Ok(())
}

fn connection<'a>(host: &'a HostState, handle: u32) -> Result<&'a Connection, JsErrorBox> {
    host.databases
        .get(handle as usize)
        .map(|(_, connection)| connection)
        .ok_or_else(|| JsErrorBox::generic("unknown database handle"))
}

#[op2]
#[string]
fn op_bb_db_exec(
    state: &mut OpState,
    #[smi] handle: u32,
    #[string] sql: &str,
    #[string] params_json: &str,
) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostState>();
    let connection = connection(host, handle)?;
    if params_json == "null" {
        connection.execute_batch(sql).map_err(sqlite_err)?;
        return Ok("null".into());
    }
    let params: Params = serde_json::from_str(params_json).map_err(box_err)?;
    let mut statement = connection.prepare_cached(sql).map_err(sqlite_err)?;
    bind(&mut statement, &params)?;
    let changes = statement.raw_execute().map_err(sqlite_err)?;
    Ok(serde_json::json!({
        "changes": changes,
        "lastInsertRowid": connection.last_insert_rowid(),
    })
    .to_string())
}

#[op2]
#[string]
fn op_bb_db_query(
    state: &mut OpState,
    #[smi] handle: u32,
    #[string] sql: &str,
    #[string] params_json: &str,
) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostState>();
    let connection = connection(host, handle)?;
    let params: Params = serde_json::from_str(params_json).map_err(box_err)?;
    let mut statement = connection.prepare_cached(sql).map_err(sqlite_err)?;
    bind(&mut statement, &params)?;
    let names: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_string)
        .collect();
    let mut rows = statement.raw_query();
    let mut output = Vec::new();
    while let Some(row) = rows.next().map_err(sqlite_err)? {
        let mut object = Map::new();
        for (index, name) in names.iter().enumerate() {
            let value: SqlValue = row.get(index).map_err(sqlite_err)?;
            let json = match value {
                SqlValue::Null => Value::Null,
                SqlValue::Integer(int) => Value::from(int),
                SqlValue::Real(real) => Value::from(real),
                SqlValue::Text(text) => Value::String(text),
                SqlValue::Blob(bytes) => Value::from(bytes),
            };
            object.insert(name.clone(), json);
        }
        output.push(Value::Object(object));
    }
    serde_json::to_string(&output).map_err(box_err)
}

#[op2]
#[string]
async fn op_bb_sdk_call(
    #[string] plugin: String,
    #[string] path: String,
    #[string] args: String,
) -> Result<String, JsErrorBox> {
    tokio::task::yield_now().await;
    let reply = match path.as_str() {
        "system.config" => serde_json::json!({ "primaryHostId": "host_spike" }),
        "system.uiPreferences.list" => serde_json::json!({ "preferences": {} }),
        "hosts.list" => serde_json::json!([]),
        "providers.list" => serde_json::json!({ "providers": [] }),
        _ => {
            return Err(JsErrorBox::generic(format!(
                "bb.sdk.{path} is not served by the spike host (plugin {plugin}, args {args})"
            )))
        }
    };
    Ok(reply.to_string())
}

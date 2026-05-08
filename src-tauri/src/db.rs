use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};
use serde_json::Value;
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;
use tokio_postgres::config::{Host, SslMode};
use tokio_postgres::{CancelToken, Client, Config, SimpleQueryMessage, SimpleQueryRow};
use uuid::Uuid;

use crate::error::AppError;
use crate::models::{
    ColumnInfo, ConnectionInfo, IndexInfo, QueryColumn, QueryPage, SavedConnection, SchemaInfo,
    SortDirection, SortSpec, StoredConnection, StoredQuery, TableInfo, TablePage,
};

const DEFAULT_PAGE_SIZE: u32 = 500;
const MAX_PAGE_SIZE: u32 = 2_000;
const STATEMENT_TIMEOUT: &str = "30s";
const KEYCHAIN_SERVICE: &str = "com.postgresviewer.desktop.connections";

#[derive(Default)]
pub struct AppState {
    connections: Mutex<HashMap<String, StoredConnection>>,
    query_sessions: Mutex<HashMap<String, StoredQuery>>,
    active_queries: Mutex<HashMap<String, CancelToken>>,
}

#[tauri::command]
pub async fn connect(
    connection_url: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ConnectionInfo, AppError> {
    let connection_url = normalize_connection_url(&connection_url)?;
    if connection_url.is_empty() {
        return Err(AppError::new(
            "invalid_connection_url",
            "Connection URL cannot be empty.",
        ));
    }

    let client = connect_client(&connection_url).await?;
    let rows = simple_query_rows(
        &client,
        "select current_database() as database, current_user as user",
    )
    .await?;
    let row = rows.first().ok_or_else(|| {
        AppError::new(
            "empty_result",
            "Connection identity query returned no rows.",
        )
    })?;
    let database = optional_text(row, "database")?.unwrap_or_else(|| "unknown".to_string());
    let user = optional_text(row, "user")?.unwrap_or_else(|| "unknown".to_string());

    let id = Uuid::new_v4().to_string();
    let saved_url = connection_url.clone();
    state.connections.lock().await.insert(
        id.clone(),
        StoredConnection {
            url: connection_url,
        },
    );

    if let Err(error) = save_connection_profile(&app, &saved_url) {
        eprintln!("failed to save connection profile: {}", error.message);
    }

    Ok(ConnectionInfo { id, database, user })
}

#[tauri::command]
pub async fn connect_saved(
    saved_connection_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ConnectionInfo, AppError> {
    let connection_url = load_saved_connection_url(&saved_connection_id)?;
    connect(connection_url, app, state).await
}

#[tauri::command]
pub async fn list_saved_connections(app: AppHandle) -> Result<Vec<SavedConnection>, AppError> {
    let mut connections = load_saved_connections(&app)?;
    connections.sort_by(|left, right| right.last_used.cmp(&left.last_used));
    Ok(connections)
}

#[tauri::command]
pub async fn forget_saved_connection(
    saved_connection_id: String,
    app: AppHandle,
) -> Result<Vec<SavedConnection>, AppError> {
    let mut connections = load_saved_connections(&app)?;
    connections.retain(|connection| connection.id != saved_connection_id);
    let _ = delete_generic_password(KEYCHAIN_SERVICE, &saved_connection_id);
    write_saved_connections(&app, &connections)?;
    connections.sort_by(|left, right| right.last_used.cmp(&left.last_used));
    Ok(connections)
}

#[tauri::command]
pub async fn disconnect(connection_id: String, state: State<'_, AppState>) -> Result<(), AppError> {
    state.connections.lock().await.remove(&connection_id);
    state
        .query_sessions
        .lock()
        .await
        .retain(|_, query| query.connection_id != connection_id);
    Ok(())
}

#[tauri::command]
pub async fn list_schemas(
    connection_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<SchemaInfo>, AppError> {
    let client = client_for_connection(&state, &connection_id).await?;
    let rows = simple_query_rows(
        &client,
        "
            select schema_name
            from information_schema.schemata
            where schema_name not in ('information_schema', 'pg_catalog')
              and schema_name not in (
                'extensions',
                'graphql',
                'graphql_public',
                'pgbouncer',
                'realtime',
                'supabase_migrations',
                'vault'
              )
              and schema_name not like 'pg_toast%'
              and schema_name not like 'pg_temp_%'
              and schema_name not like 'pg_toast_temp_%'
            order by schema_name
            ",
    )
    .await?;

    let mut schemas = rows
        .iter()
        .map(|row| {
            Ok(SchemaInfo {
                name: required_text(row, "schema_name")?,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    schemas.retain(|schema| !is_ignored_schema(&schema.name));

    Ok(schemas)
}

#[tauri::command]
pub async fn list_tables(
    connection_id: String,
    schema: String,
    state: State<'_, AppState>,
) -> Result<Vec<TableInfo>, AppError> {
    let client = client_for_connection(&state, &connection_id).await?;
    list_tables_inner(&client, &schema).await
}

#[tauri::command]
pub async fn describe_table(
    connection_id: String,
    schema: String,
    table: String,
    state: State<'_, AppState>,
) -> Result<Vec<ColumnInfo>, AppError> {
    let client = client_for_connection(&state, &connection_id).await?;
    describe_table_inner(&client, &schema, &table).await
}

#[tauri::command]
pub async fn list_indexes(
    connection_id: String,
    schema: String,
    table: String,
    state: State<'_, AppState>,
) -> Result<Vec<IndexInfo>, AppError> {
    let client = client_for_connection(&state, &connection_id).await?;
    let rows = simple_query_rows(
        &client,
        &format!(
            "
            select
              idx.relname as name,
              pg_get_indexdef(i.indexrelid) as definition,
              i.indisunique as unique,
              i.indisprimary as primary
            from pg_index i
            join pg_class tbl on tbl.oid = i.indrelid
            join pg_class idx on idx.oid = i.indexrelid
            join pg_namespace ns on ns.oid = tbl.relnamespace
            where ns.nspname = {}
              and tbl.relname = {}
            order by i.indisprimary desc, i.indisunique desc, idx.relname
            ",
            quote_literal(&schema),
            quote_literal(&table),
        ),
    )
    .await?;

    Ok(rows
        .iter()
        .map(|row| {
            Ok(IndexInfo {
                name: required_text(row, "name")?,
                definition: required_text(row, "definition")?,
                unique: required_bool(row, "unique")?,
                primary: required_bool(row, "primary")?,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?)
}

#[tauri::command]
pub async fn fetch_table_page(
    connection_id: String,
    schema: String,
    table: String,
    page: u32,
    page_size: u32,
    sort: Option<SortSpec>,
    state: State<'_, AppState>,
) -> Result<TablePage, AppError> {
    let client = client_for_connection(&state, &connection_id).await?;
    let columns = describe_table_inner(&client, &schema, &table).await?;
    let page_size = clamp_page_size(page_size);
    let (rows, has_more) =
        fetch_table_rows_inner(&client, &schema, &table, page, page_size, sort).await?;

    Ok(TablePage {
        columns,
        rows,
        page,
        page_size,
        has_more,
    })
}

#[tauri::command]
pub async fn run_query(
    connection_id: String,
    sql: String,
    page_size: u32,
    request_id: String,
    state: State<'_, AppState>,
) -> Result<QueryPage, AppError> {
    let connection_url = connection_url(&state, &connection_id).await?;
    let sql = normalize_sql(&sql)?;
    let page_size = clamp_page_size(page_size);
    let client = connect_client(&connection_url).await?;
    let cancel_token = client.cancel_token();

    if !request_id.trim().is_empty() {
        state
            .active_queries
            .lock()
            .await
            .insert(request_id.clone(), cancel_token);
    }

    let result = run_query_page(&client, &sql, page_size).await;

    if !request_id.trim().is_empty() {
        state.active_queries.lock().await.remove(&request_id);
    }

    let (columns, rows, has_more) = result?;
    let handle_id = Uuid::new_v4().to_string();

    state.query_sessions.lock().await.insert(
        handle_id.clone(),
        StoredQuery {
            connection_id,
            sql,
            page_size,
            columns: columns.clone(),
        },
    );

    Ok(QueryPage {
        handle_id,
        columns,
        rows,
        page: 0,
        page_size,
        has_more,
    })
}

#[tauri::command]
pub async fn fetch_query_page(
    handle_id: String,
    page: u32,
    state: State<'_, AppState>,
) -> Result<QueryPage, AppError> {
    let query = {
        state
            .query_sessions
            .lock()
            .await
            .get(&handle_id)
            .cloned()
            .ok_or_else(|| {
                AppError::new("query_not_found", "Query results are no longer active.")
            })?
    };
    let client = client_for_connection(&state, &query.connection_id).await?;
    let (rows, has_more) =
        fetch_query_rows_transaction(&client, &query.sql, query.page_size, page).await?;

    Ok(QueryPage {
        handle_id,
        columns: query.columns,
        rows,
        page,
        page_size: query.page_size,
        has_more,
    })
}

#[tauri::command]
pub async fn cancel_query(
    request_id: String,
    state: State<'_, AppState>,
) -> Result<bool, AppError> {
    let cancel_token = state.active_queries.lock().await.get(&request_id).cloned();

    let Some(cancel_token) = cancel_token else {
        return Ok(false);
    };

    cancel_token.cancel_query(make_tls_connector(true)?).await?;
    Ok(true)
}

async fn client_for_connection(
    state: &State<'_, AppState>,
    connection_id: &str,
) -> Result<Client, AppError> {
    let url = connection_url(state, connection_id).await?;
    connect_client(&url).await
}

async fn connection_url(
    state: &State<'_, AppState>,
    connection_id: &str,
) -> Result<String, AppError> {
    state
        .connections
        .lock()
        .await
        .get(connection_id)
        .map(|connection| connection.url.clone())
        .ok_or_else(|| AppError::new("connection_not_found", "Connection is no longer active."))
}

fn make_tls_connector(accept_invalid_certs: bool) -> Result<MakeTlsConnector, AppError> {
    let mut builder = TlsConnector::builder();

    if accept_invalid_certs {
        // libpq's sslmode=require encrypts without validating the certificate.
        // Supabase poolers present a Supabase CA chain that native TLS does not
        // trust by default, so this keeps common Supabase URLs working.
        builder.danger_accept_invalid_certs(true);
        builder.danger_accept_invalid_hostnames(true);
    }

    let connector = builder.build()?;
    Ok(MakeTlsConnector::new(connector))
}

async fn connect_client(connection_url: &str) -> Result<Client, AppError> {
    let config = Config::from_str(connection_url).map_err(AppError::from)?;
    let accept_invalid_certs = config.get_ssl_mode() != SslMode::Disable;
    let (client, connection) = config
        .connect(make_tls_connector(accept_invalid_certs)?)
        .await?;

    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("postgres connection error: {error}");
        }
    });

    Ok(client)
}

async fn list_tables_inner(client: &Client, schema: &str) -> Result<Vec<TableInfo>, AppError> {
    if is_ignored_schema(schema) {
        return Ok(Vec::new());
    }

    let rows = simple_query_rows(
        client,
        &format!(
            "
            select
              ns.nspname as schema,
              cls.relname as name,
              case cls.relkind
                when 'r' then 'table'
                when 'p' then 'partitioned table'
                when 'v' then 'view'
                when 'm' then 'materialized view'
                when 'f' then 'foreign table'
                else cls.relkind::text
              end as kind
            from pg_class cls
            join pg_namespace ns on ns.oid = cls.relnamespace
            where ns.nspname = {}
              and ns.nspname not like 'pg_temp_%'
              and ns.nspname not like 'pg_toast_temp_%'
              and ns.nspname not in (
                'extensions',
                'graphql',
                'graphql_public',
                'pgbouncer',
                'realtime',
                'supabase_migrations',
                'vault'
              )
              and cls.relkind in ('r', 'p', 'v', 'm', 'f')
            order by kind, name
            ",
            quote_literal(schema),
        ),
    )
    .await?;

    Ok(rows
        .iter()
        .map(|row| {
            Ok(TableInfo {
                schema: required_text(row, "schema")?,
                name: required_text(row, "name")?,
                kind: required_text(row, "kind")?,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?)
}

async fn describe_table_inner(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<ColumnInfo>, AppError> {
    let rows = simple_query_rows(
        client,
        &format!(
            "
            select
              attr.attname as name,
              pg_catalog.format_type(attr.atttypid, attr.atttypmod) as data_type,
              not attr.attnotnull as nullable,
              attr.attnum::int as ordinal,
              pg_get_expr(def.adbin, def.adrelid) as default_value
            from pg_attribute attr
            join pg_class cls on cls.oid = attr.attrelid
            join pg_namespace ns on ns.oid = cls.relnamespace
            left join pg_attrdef def
              on def.adrelid = attr.attrelid
             and def.adnum = attr.attnum
            where ns.nspname = {}
              and cls.relname = {}
              and attr.attnum > 0
              and not attr.attisdropped
            order by attr.attnum
            ",
            quote_literal(schema),
            quote_literal(table),
        ),
    )
    .await?;

    Ok(rows
        .iter()
        .map(|row| {
            Ok(ColumnInfo {
                name: required_text(row, "name")?,
                data_type: required_text(row, "data_type")?,
                nullable: optional_bool(row, "nullable")?,
                ordinal: optional_i32(row, "ordinal")?,
                default_value: optional_text(row, "default_value")?,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?)
}

async fn fetch_table_rows_inner(
    client: &Client,
    schema: &str,
    table: &str,
    page: u32,
    page_size: u32,
    sort: Option<SortSpec>,
) -> Result<(Vec<Value>, bool), AppError> {
    let limit = i64::from(page_size + 1);
    let offset = i64::from(page.saturating_mul(page_size));
    let mut order_clause = String::new();

    if let Some(sort) = sort {
        if !sort.column.trim().is_empty() {
            let direction = match sort.direction {
                SortDirection::Asc => "asc",
                SortDirection::Desc => "desc",
            };
            order_clause = format!(" order by {} {}", quote_ident(&sort.column), direction);
        }
    }

    let sql = format!(
        "
        select to_jsonb(page_rows)::text as row_json
        from (
          select *
          from {}.{}
          {}
          limit {} offset {}
        ) page_rows
        ",
        quote_ident(schema),
        quote_ident(table),
        order_clause,
        limit,
        offset
    );

    fetch_json_rows(client, &sql, page_size).await
}

async fn run_query_page(
    client: &Client,
    sql: &str,
    page_size: u32,
) -> Result<(Vec<QueryColumn>, Vec<Value>, bool), AppError> {
    begin_read_only(client).await?;

    let result = async {
        let metadata_sql = format!("select * from ({sql}) viewer_query limit 0");
        let columns = simple_query_columns(client, &metadata_sql).await?;

        let (rows, has_more) = fetch_query_rows_inner(client, sql, page_size, 0).await?;
        Ok::<_, AppError>((columns, rows, has_more))
    }
    .await;

    finish_transaction(client, result).await
}

async fn fetch_query_rows_transaction(
    client: &Client,
    sql: &str,
    page_size: u32,
    page: u32,
) -> Result<(Vec<Value>, bool), AppError> {
    begin_read_only(client).await?;
    let result = fetch_query_rows_inner(client, sql, page_size, page).await;
    finish_transaction(client, result).await
}

async fn fetch_query_rows_inner(
    client: &Client,
    sql: &str,
    page_size: u32,
    page: u32,
) -> Result<(Vec<Value>, bool), AppError> {
    let limit = i64::from(page_size + 1);
    let offset = i64::from(page.saturating_mul(page_size));
    let page_sql = format!(
        "
        select to_jsonb(page_rows)::text as row_json
        from (
          select *
          from ({sql}) viewer_query
          limit {limit} offset {offset}
        ) page_rows
        "
    );

    fetch_json_rows(client, &page_sql, page_size).await
}

async fn fetch_json_rows(
    client: &Client,
    sql: &str,
    page_size: u32,
) -> Result<(Vec<Value>, bool), AppError> {
    let rows = simple_query_rows(client, sql).await?;
    let mut values = rows
        .iter()
        .map(|row| {
            let json = required_text(row, "row_json")?;
            serde_json::from_str::<Value>(&json).map_err(AppError::from)
        })
        .collect::<Result<Vec<_>, AppError>>()?;

    let has_more = values.len() > page_size as usize;
    values.truncate(page_size as usize);
    Ok((values, has_more))
}

async fn begin_read_only(client: &Client) -> Result<(), AppError> {
    client
        .batch_execute(&format!(
            "begin read only; set local statement_timeout = '{}';",
            STATEMENT_TIMEOUT
        ))
        .await?;
    Ok(())
}

async fn finish_transaction<T>(
    client: &Client,
    result: Result<T, AppError>,
) -> Result<T, AppError> {
    match result {
        Ok(value) => {
            client.batch_execute("commit").await?;
            Ok(value)
        }
        Err(error) => {
            let _ = client.batch_execute("rollback").await;
            Err(error)
        }
    }
}

fn clamp_page_size(page_size: u32) -> u32 {
    if page_size == 0 {
        DEFAULT_PAGE_SIZE
    } else {
        page_size.min(MAX_PAGE_SIZE)
    }
}

fn quote_ident(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn is_ignored_schema(schema: &str) -> bool {
    schema == "information_schema"
        || schema == "pg_catalog"
        || matches!(
            schema,
            "extensions"
                | "graphql"
                | "graphql_public"
                | "pgbouncer"
                | "realtime"
                | "supabase_migrations"
                | "vault"
        )
        || schema.starts_with("pg_toast")
        || schema.starts_with("pg_temp_")
}

fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

async fn simple_query_rows(client: &Client, sql: &str) -> Result<Vec<SimpleQueryRow>, AppError> {
    let messages = client.simple_query(sql).await?;
    Ok(messages
        .into_iter()
        .filter_map(|message| match message {
            SimpleQueryMessage::Row(row) => Some(row),
            _ => None,
        })
        .collect())
}

async fn simple_query_columns(client: &Client, sql: &str) -> Result<Vec<QueryColumn>, AppError> {
    let messages = client.simple_query(sql).await?;
    let columns = messages.into_iter().find_map(|message| match message {
        SimpleQueryMessage::RowDescription(columns) => Some(
            columns
                .iter()
                .map(|column| QueryColumn {
                    name: column.name().to_string(),
                    data_type: "unknown".to_string(),
                })
                .collect::<Vec<_>>(),
        ),
        _ => None,
    });

    columns.ok_or_else(|| AppError::new("empty_result", "Query returned no column metadata."))
}

fn optional_text(row: &SimpleQueryRow, column: &str) -> Result<Option<String>, AppError> {
    Ok(row.try_get(column)?.map(str::to_string))
}

fn required_text(row: &SimpleQueryRow, column: &str) -> Result<String, AppError> {
    optional_text(row, column)?.ok_or_else(|| {
        AppError::new(
            "unexpected_null",
            format!("Column {column} unexpectedly returned NULL."),
        )
    })
}

fn optional_bool(row: &SimpleQueryRow, column: &str) -> Result<Option<bool>, AppError> {
    optional_text(row, column)?
        .map(|value| parse_bool(&value, column))
        .transpose()
}

fn required_bool(row: &SimpleQueryRow, column: &str) -> Result<bool, AppError> {
    optional_bool(row, column)?.ok_or_else(|| {
        AppError::new(
            "unexpected_null",
            format!("Column {column} unexpectedly returned NULL."),
        )
    })
}

fn optional_i32(row: &SimpleQueryRow, column: &str) -> Result<Option<i32>, AppError> {
    optional_text(row, column)?
        .map(|value| {
            value.parse::<i32>().map_err(|error| {
                AppError::new(
                    "parse_error",
                    format!("Could not parse column {column} as integer: {error}"),
                )
            })
        })
        .transpose()
}

fn parse_bool(value: &str, column: &str) -> Result<bool, AppError> {
    match value {
        "t" | "true" => Ok(true),
        "f" | "false" => Ok(false),
        _ => Err(AppError::new(
            "parse_error",
            format!("Could not parse column {column} as boolean."),
        )),
    }
}

fn normalize_connection_url(connection_url: &str) -> Result<String, AppError> {
    let connection_url = connection_url.trim();
    if connection_url.is_empty() {
        return Ok(String::new());
    }

    let mut normalized = connection_url.to_string();
    if !has_connection_param(connection_url, "sslmode") {
        let separator = if connection_url.contains('?') {
            '&'
        } else {
            '?'
        };
        normalized.push(separator);
        normalized.push_str("sslmode=require");
    }

    Config::from_str(&normalized).map_err(AppError::from)?;
    Ok(normalized)
}

fn save_connection_profile(
    app: &AppHandle,
    connection_url: &str,
) -> Result<SavedConnection, AppError> {
    let profile = saved_connection_from_url(connection_url)?;
    set_generic_password(KEYCHAIN_SERVICE, &profile.id, connection_url.as_bytes())?;

    let mut connections = load_saved_connections(app)?;
    if let Some(existing) = connections
        .iter_mut()
        .find(|connection| connection.id == profile.id)
    {
        *existing = profile.clone();
    } else {
        connections.push(profile.clone());
    }
    connections.sort_by(|left, right| right.last_used.cmp(&left.last_used));
    connections.truncate(12);
    write_saved_connections(app, &connections)?;

    Ok(profile)
}

fn load_saved_connection_url(saved_connection_id: &str) -> Result<String, AppError> {
    let bytes = get_generic_password(KEYCHAIN_SERVICE, saved_connection_id)?;
    String::from_utf8(bytes).map_err(|error| {
        AppError::new(
            "keychain_error",
            format!("Saved connection URL is not valid UTF-8: {error}"),
        )
    })
}

fn saved_connection_from_url(connection_url: &str) -> Result<SavedConnection, AppError> {
    let config = Config::from_str(connection_url).map_err(AppError::from)?;
    let host = config
        .get_hosts()
        .first()
        .map(host_label)
        .unwrap_or_else(|| "localhost".to_string());
    let port = config.get_ports().first().copied().unwrap_or(5432);
    let database = config.get_dbname().unwrap_or("postgres").to_string();
    let user = config.get_user().unwrap_or("postgres").to_string();
    let id = stable_connection_id(&host, port, &database, &user);
    let label = format!("{database} @ {host}:{port}");

    Ok(SavedConnection {
        id,
        label,
        host,
        database,
        user,
        last_used: current_timestamp(),
    })
}

fn host_label(host: &Host) -> String {
    match host {
        Host::Tcp(host) => host.clone(),
        #[cfg(unix)]
        Host::Unix(path) => path.display().to_string(),
    }
}

fn stable_connection_id(host: &str, port: u16, database: &str, user: &str) -> String {
    let input = format!("{host}|{port}|{database}|{user}");
    let mut hash = 0xcbf29ce484222325_u64;

    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }

    format!("{hash:016x}")
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn saved_connections_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| AppError::new("path_error", error.to_string()))?;
    Ok(app_data_dir.join("saved-connections.json"))
}

fn load_saved_connections(app: &AppHandle) -> Result<Vec<SavedConnection>, AppError> {
    let path = saved_connections_path(app)?;
    if !path.exists() {
        return Ok(Vec::new());
    }

    let bytes = fs::read(path)?;
    if bytes.is_empty() {
        return Ok(Vec::new());
    }

    Ok(serde_json::from_slice(&bytes)?)
}

fn write_saved_connections(
    app: &AppHandle,
    connections: &[SavedConnection],
) -> Result<(), AppError> {
    let path = saved_connections_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let bytes = serde_json::to_vec_pretty(connections)?;
    fs::write(&path, bytes)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

fn has_connection_param(connection_url: &str, param: &str) -> bool {
    let param = param.to_ascii_lowercase();

    if let Some((_, query)) = connection_url.split_once('?') {
        return query.split('&').any(|pair| {
            pair.split_once('=')
                .map(|(key, _)| key.eq_ignore_ascii_case(&param))
                .unwrap_or_else(|| pair.eq_ignore_ascii_case(&param))
        });
    }

    connection_url
        .split_whitespace()
        .any(|part| part.to_ascii_lowercase().starts_with(&format!("{param}=")))
}

fn normalize_sql(sql: &str) -> Result<String, AppError> {
    let mut trimmed = sql.trim();
    while trimmed.ends_with(';') {
        trimmed = trimmed[..trimmed.len() - 1].trim_end();
    }

    if trimmed.is_empty() {
        return Err(AppError::new("empty_query", "SQL cannot be empty."));
    }

    if contains_statement_separator(trimmed) {
        return Err(AppError::new(
            "multiple_statements",
            "Only one SQL statement can be run at a time.",
        ));
    }

    Ok(trimmed.to_string())
}

fn contains_statement_separator(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut dollar_tag: Option<Vec<u8>> = None;

    while i < bytes.len() {
        if let Some(tag) = dollar_tag.as_ref() {
            if bytes[i..].starts_with(tag) {
                let tag_len = tag.len();
                dollar_tag = None;
                i += tag_len;
            } else {
                i += 1;
            }
            continue;
        }

        if in_line_comment {
            in_line_comment = bytes[i] != b'\n';
            i += 1;
            continue;
        }

        if in_block_comment {
            if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        if in_single {
            if bytes[i] == b'\'' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    i += 2;
                } else {
                    in_single = false;
                    i += 1;
                }
            } else {
                i += 1;
            }
            continue;
        }

        if in_double {
            if bytes[i] == b'"' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                    i += 2;
                } else {
                    in_double = false;
                    i += 1;
                }
            } else {
                i += 1;
            }
            continue;
        }

        if bytes[i] == b';' {
            return true;
        }
        if bytes[i] == b'\'' {
            in_single = true;
            i += 1;
            continue;
        }
        if bytes[i] == b'"' {
            in_double = true;
            i += 1;
            continue;
        }
        if bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            in_line_comment = true;
            i += 2;
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            in_block_comment = true;
            i += 2;
            continue;
        }
        if bytes[i] == b'$' {
            if let Some(tag) = dollar_quote_tag(bytes, i) {
                i += tag.len();
                dollar_tag = Some(tag);
                continue;
            }
        }

        i += 1;
    }

    false
}

fn dollar_quote_tag(bytes: &[u8], start: usize) -> Option<Vec<u8>> {
    if bytes.get(start) != Some(&b'$') {
        return None;
    }

    let mut end = start + 1;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }

    if bytes.get(end) == Some(&b'$') {
        Some(bytes[start..=end].to_vec())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{has_connection_param, normalize_connection_url, normalize_sql};

    #[test]
    fn normalize_connection_url_adds_sslmode_require() {
        let normalized =
            normalize_connection_url("postgresql://user:pass@example.com:5432/postgres").unwrap();
        assert!(normalized.ends_with("?sslmode=require"));
    }

    #[test]
    fn normalize_connection_url_preserves_existing_query_params() {
        let normalized =
            normalize_connection_url("postgresql://user:pass@example.com/db?connect_timeout=5")
                .unwrap();
        assert!(normalized.ends_with("?connect_timeout=5&sslmode=require"));
    }

    #[test]
    fn has_connection_param_detects_sslmode() {
        assert!(has_connection_param(
            "postgresql://user:pass@example.com/db?sslmode=require",
            "sslmode"
        ));
    }

    #[test]
    fn normalize_sql_trims_trailing_semicolons() {
        assert_eq!(normalize_sql(" select 1 ; ; ").unwrap(), "select 1");
    }

    #[test]
    fn normalize_sql_rejects_multiple_statements() {
        let error = normalize_sql("select 1; select 2").unwrap_err();
        assert_eq!(error.code, "multiple_statements");
    }

    #[test]
    fn normalize_sql_allows_semicolons_inside_strings() {
        assert_eq!(normalize_sql("select ';'").unwrap(), "select ';'");
    }

    #[test]
    fn normalize_sql_allows_semicolons_inside_dollar_quotes() {
        assert_eq!(
            normalize_sql("select $$one;two$$;").unwrap(),
            "select $$one;two$$"
        );
    }
}

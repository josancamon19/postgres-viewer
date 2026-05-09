use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;
use tokio::time::Duration;
use tokio_postgres::config::{Host, SslMode};
use tokio_postgres::{CancelToken, Client, Config, SimpleQueryMessage, SimpleQueryRow};
use uuid::Uuid;

use crate::error::AppError;
use crate::models::{
    AppSettings, ColumnInfo, ConnectionInfo, GeneratedSql, IndexInfo, QueryColumn, QueryPage,
    SavedConnection, SchemaInfo, SortDirection, SortSpec, TableIdentity, TableInfo, TablePage,
    WriteResult,
};

const DEFAULT_PAGE_SIZE: u32 = 500;
const MAX_PAGE_SIZE: u32 = 2_000;
const STATEMENT_TIMEOUT: &str = "30s";
const KEYCHAIN_SERVICE: &str = "com.postgresviewer.desktop.connections";
const OPENAI_KEYCHAIN_SERVICE: &str = "com.postgresviewer.desktop.openai";
const OPENAI_KEYCHAIN_ACCOUNT: &str = "openai-api-key";
const OPENAI_MODEL: &str = "gpt-5.5";

pub struct AppState {
    connections: Mutex<HashMap<String, StoredConnection>>,
    query_sessions: Mutex<HashMap<String, StoredQuery>>,
    active_queries: Mutex<HashMap<String, CancelToken>>,
    http: reqwest::Client,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            query_sessions: Mutex::new(HashMap::new()),
            active_queries: Mutex::new(HashMap::new()),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(45))
                .build()
                .expect("failed to build HTTP client"),
        }
    }
}

struct StoredConnection {
    url: String,
    client: Arc<Client>,
    cache: Arc<Mutex<ConnectionCache>>,
}

#[derive(Debug, Clone)]
struct StoredQuery {
    connection_id: String,
    sql: String,
    page_size: u32,
    columns: Vec<QueryColumn>,
}

#[derive(Debug, Default)]
struct ConnectionCache {
    schemas: Option<Vec<SchemaInfo>>,
    tables: HashMap<String, Vec<TableInfo>>,
    columns: HashMap<String, Vec<ColumnInfo>>,
    indexes: HashMap<String, Vec<IndexInfo>>,
    identities: HashMap<String, TableIdentity>,
    pages: HashMap<String, TablePage>,
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
    let profile = save_connection_profile(&app, &connection_url)?;

    let id = Uuid::new_v4().to_string();
    state.connections.lock().await.insert(
        id.clone(),
        StoredConnection {
            url: connection_url,
            client: Arc::new(client),
            cache: Arc::new(Mutex::new(ConnectionCache::default())),
        },
    );

    Ok(ConnectionInfo {
        id,
        saved_connection_id: profile.id,
        label: profile.label,
        database,
        user,
    })
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
pub async fn update_saved_connection_label(
    saved_connection_id: String,
    label: String,
    app: AppHandle,
) -> Result<Vec<SavedConnection>, AppError> {
    let label = label.trim();
    if label.is_empty() {
        return Err(AppError::new(
            "invalid_label",
            "Connection name cannot be empty.",
        ));
    }

    let mut connections = load_saved_connections(&app)?;
    let connection = connections
        .iter_mut()
        .find(|connection| connection.id == saved_connection_id)
        .ok_or_else(|| AppError::new("connection_not_found", "Saved connection was not found."))?;
    connection.label = label.to_string();
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
pub async fn get_settings() -> Result<AppSettings, AppError> {
    Ok(AppSettings {
        has_openai_api_key: load_openai_api_key().is_ok(),
        openai_model: OPENAI_MODEL.to_string(),
    })
}

#[tauri::command]
pub async fn set_openai_api_key(api_key: String) -> Result<AppSettings, AppError> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(AppError::new(
            "invalid_api_key",
            "OpenAI API key cannot be empty.",
        ));
    }

    set_generic_password(
        OPENAI_KEYCHAIN_SERVICE,
        OPENAI_KEYCHAIN_ACCOUNT,
        api_key.as_bytes(),
    )?;
    get_settings().await
}

#[tauri::command]
pub async fn clear_openai_api_key() -> Result<AppSettings, AppError> {
    let _ = delete_generic_password(OPENAI_KEYCHAIN_SERVICE, OPENAI_KEYCHAIN_ACCOUNT);
    get_settings().await
}

#[tauri::command]
pub async fn list_schemas(
    connection_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<SchemaInfo>, AppError> {
    let (_, client, cache) = connection_parts(&state, &connection_id).await?;
    if let Some(schemas) = cache.lock().await.schemas.clone() {
        return Ok(schemas);
    }

    let rows = simple_query_rows(
        client.as_ref(),
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

    cache.lock().await.schemas = Some(schemas.clone());
    Ok(schemas)
}

#[tauri::command]
pub async fn list_tables(
    connection_id: String,
    schema: String,
    state: State<'_, AppState>,
) -> Result<Vec<TableInfo>, AppError> {
    let (_, client, cache) = connection_parts(&state, &connection_id).await?;
    if let Some(tables) = cache.lock().await.tables.get(&schema).cloned() {
        return Ok(tables);
    }

    let tables = list_tables_inner(client.as_ref(), &schema).await?;
    cache.lock().await.tables.insert(schema, tables.clone());
    Ok(tables)
}

#[tauri::command]
pub async fn describe_table(
    connection_id: String,
    schema: String,
    table: String,
    state: State<'_, AppState>,
) -> Result<Vec<ColumnInfo>, AppError> {
    let (_, client, cache) = connection_parts(&state, &connection_id).await?;
    cached_columns(client.as_ref(), cache, &schema, &table).await
}

#[tauri::command]
pub async fn list_indexes(
    connection_id: String,
    schema: String,
    table: String,
    state: State<'_, AppState>,
) -> Result<Vec<IndexInfo>, AppError> {
    let (_, client, cache) = connection_parts(&state, &connection_id).await?;
    let cache_key = table_cache_key(&schema, &table);
    if let Some(indexes) = cache.lock().await.indexes.get(&cache_key).cloned() {
        return Ok(indexes);
    }

    let rows = simple_query_rows(
        client.as_ref(),
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

    let indexes = rows
        .iter()
        .map(|row| {
            Ok(IndexInfo {
                name: required_text(row, "name")?,
                definition: required_text(row, "definition")?,
                unique: required_bool(row, "unique")?,
                primary: required_bool(row, "primary")?,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    cache
        .lock()
        .await
        .indexes
        .insert(cache_key, indexes.clone());
    Ok(indexes)
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
    let (_, client, cache) = connection_parts(&state, &connection_id).await?;
    let page_size = clamp_page_size(page_size);
    let page_key = page_cache_key(&schema, &table, page, page_size, sort.as_ref());
    if let Some(page) = cache.lock().await.pages.get(&page_key).cloned() {
        return Ok(TablePage {
            from_cache: true,
            ..page
        });
    }

    fetch_table_page_live(
        client.as_ref(),
        cache,
        &schema,
        &table,
        page,
        page_size,
        sort,
    )
    .await
}

#[tauri::command]
pub async fn refresh_table_cache(
    connection_id: String,
    schema: String,
    table: String,
    page: u32,
    page_size: u32,
    sort: Option<SortSpec>,
    state: State<'_, AppState>,
) -> Result<TablePage, AppError> {
    let (_, client, cache) = connection_parts(&state, &connection_id).await?;
    let page_size = clamp_page_size(page_size);
    fetch_table_page_live(
        client.as_ref(),
        cache,
        &schema,
        &table,
        page,
        page_size,
        sort,
    )
    .await
}

#[tauri::command]
pub async fn update_cell(
    connection_id: String,
    schema: String,
    table: String,
    key: HashMap<String, Value>,
    column: String,
    value: Value,
    state: State<'_, AppState>,
) -> Result<WriteResult, AppError> {
    let (url, _, cache) = connection_parts(&state, &connection_id).await?;
    let client = connect_client(&url).await?;
    let identity = cached_identity(&client, Arc::clone(&cache), &schema, &table).await?;
    ensure_editable_table(&identity)?;

    if identity
        .columns
        .iter()
        .any(|identity_column| identity_column == &column)
    {
        return Err(AppError::new(
            "identity_column_edit",
            "Identity columns cannot be edited in grid mode.",
        ));
    }

    let columns = cached_columns(&client, Arc::clone(&cache), &schema, &table).await?;
    if !columns
        .iter()
        .any(|table_column| table_column.name == column)
    {
        return Err(AppError::new(
            "unknown_column",
            format!("Column {column} does not exist on {schema}.{table}."),
        ));
    }

    let where_clause = identity_where_clause(&identity, &key)?;
    let sql = format!(
        "
        update {}.{} as t
        set {} = {}
        where {}
        returning to_jsonb(t)::text as row_json
        ",
        quote_ident(&schema),
        quote_ident(&table),
        quote_ident(&column),
        sql_literal(&value),
        where_clause,
    );

    let result = run_returning_write(&client, &sql, "updated").await?;
    invalidate_table_pages(&cache, &schema, &table).await;
    Ok(result)
}

#[tauri::command]
pub async fn insert_row(
    connection_id: String,
    schema: String,
    table: String,
    values: HashMap<String, Value>,
    state: State<'_, AppState>,
) -> Result<WriteResult, AppError> {
    let (url, _, cache) = connection_parts(&state, &connection_id).await?;
    let client = connect_client(&url).await?;
    let identity = cached_identity(&client, Arc::clone(&cache), &schema, &table).await?;
    ensure_editable_table(&identity)?;

    if values.is_empty() {
        return Err(AppError::new(
            "empty_insert",
            "Provide at least one column value to insert a row.",
        ));
    }

    let columns = cached_columns(&client, Arc::clone(&cache), &schema, &table).await?;
    let column_names = columns
        .iter()
        .map(|column| column.name.as_str())
        .collect::<HashSet<_>>();
    for column in values.keys() {
        if !column_names.contains(column.as_str()) {
            return Err(AppError::new(
                "unknown_column",
                format!("Column {column} does not exist on {schema}.{table}."),
            ));
        }
    }

    let mut ordered = values.into_iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.0.cmp(&right.0));
    let insert_columns = ordered
        .iter()
        .map(|(column, _)| quote_ident(column))
        .collect::<Vec<_>>()
        .join(", ");
    let insert_values = ordered
        .iter()
        .map(|(_, value)| sql_literal(value))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "
        with inserted as (
          insert into {}.{} ({})
          values ({})
          returning *
        )
        select to_jsonb(inserted)::text as row_json
        from inserted
        ",
        quote_ident(&schema),
        quote_ident(&table),
        insert_columns,
        insert_values,
    );

    let result = run_returning_write(&client, &sql, "inserted").await?;
    invalidate_table_pages(&cache, &schema, &table).await;
    Ok(result)
}

#[tauri::command]
pub async fn delete_row(
    connection_id: String,
    schema: String,
    table: String,
    key: HashMap<String, Value>,
    state: State<'_, AppState>,
) -> Result<WriteResult, AppError> {
    let (url, _, cache) = connection_parts(&state, &connection_id).await?;
    let client = connect_client(&url).await?;
    let identity = cached_identity(&client, Arc::clone(&cache), &schema, &table).await?;
    ensure_editable_table(&identity)?;
    let where_clause = identity_where_clause(&identity, &key)?;
    let sql = format!(
        "
        delete from {}.{} as t
        where {}
        returning to_jsonb(t)::text as row_json
        ",
        quote_ident(&schema),
        quote_ident(&table),
        where_clause,
    );

    let result = run_returning_write(&client, &sql, "deleted").await?;
    invalidate_table_pages(&cache, &schema, &table).await;
    Ok(result)
}

#[tauri::command]
pub async fn run_write_sql(
    connection_id: String,
    sql: String,
    state: State<'_, AppState>,
) -> Result<WriteResult, AppError> {
    let sql = normalize_sql(&sql)?;
    ensure_write_sql(&sql)?;
    let client = client_for_connection(&state, &connection_id).await?;
    let result = run_manual_write(&client, &sql).await?;

    if let Ok((_, _, cache)) = connection_parts(&state, &connection_id).await {
        invalidate_all_pages(&cache).await;
    }

    Ok(result)
}

#[tauri::command]
pub async fn generate_sql(
    connection_id: String,
    prompt: String,
    state: State<'_, AppState>,
) -> Result<GeneratedSql, AppError> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err(AppError::new("empty_prompt", "Prompt cannot be empty."));
    }

    let api_key = load_openai_api_key()?;
    let (_, client, cache) = connection_parts(&state, &connection_id).await?;
    let schema_context = build_schema_context(client.as_ref(), Arc::clone(&cache), prompt).await?;
    let generated = request_generated_sql(&state.http, &api_key, prompt, &schema_context).await?;
    let sql = normalize_sql(&generated.sql)?;
    let auto_run = is_read_sql(&sql);

    Ok(GeneratedSql {
        sql,
        explanation: generated.explanation,
        confidence: generated.confidence,
        referenced_tables: generated.referenced_tables,
        auto_run,
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
    ensure_read_sql(&sql)?;
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

async fn connection_parts(
    state: &State<'_, AppState>,
    connection_id: &str,
) -> Result<(String, Arc<Client>, Arc<Mutex<ConnectionCache>>), AppError> {
    state
        .connections
        .lock()
        .await
        .get(connection_id)
        .map(|connection| {
            (
                connection.url.clone(),
                Arc::clone(&connection.client),
                Arc::clone(&connection.cache),
            )
        })
        .ok_or_else(|| AppError::new("connection_not_found", "Connection is no longer active."))
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

async fn fetch_table_page_live(
    client: &Client,
    cache: Arc<Mutex<ConnectionCache>>,
    schema: &str,
    table: &str,
    page: u32,
    page_size: u32,
    sort: Option<SortSpec>,
) -> Result<TablePage, AppError> {
    let columns = cached_columns(client, Arc::clone(&cache), schema, table).await?;
    let identity = cached_identity(client, Arc::clone(&cache), schema, table).await?;
    let (rows, has_more) =
        fetch_table_rows_inner(client, schema, table, page, page_size, sort.clone()).await?;
    let table_page = TablePage {
        columns,
        rows,
        page,
        page_size,
        has_more,
        from_cache: false,
        identity,
    };
    let page_key = page_cache_key(schema, table, page, page_size, sort.as_ref());
    cache
        .lock()
        .await
        .pages
        .insert(page_key, table_page.clone());
    Ok(table_page)
}

async fn cached_columns(
    client: &Client,
    cache: Arc<Mutex<ConnectionCache>>,
    schema: &str,
    table: &str,
) -> Result<Vec<ColumnInfo>, AppError> {
    let cache_key = table_cache_key(schema, table);
    if let Some(columns) = cache.lock().await.columns.get(&cache_key).cloned() {
        return Ok(columns);
    }

    let columns = describe_table_inner(client, schema, table).await?;
    cache
        .lock()
        .await
        .columns
        .insert(cache_key, columns.clone());
    Ok(columns)
}

async fn cached_identity(
    client: &Client,
    cache: Arc<Mutex<ConnectionCache>>,
    schema: &str,
    table: &str,
) -> Result<TableIdentity, AppError> {
    let cache_key = table_cache_key(schema, table);
    if let Some(identity) = cache.lock().await.identities.get(&cache_key).cloned() {
        return Ok(identity);
    }

    let identity = table_identity_inner(client, schema, table).await?;
    cache
        .lock()
        .await
        .identities
        .insert(cache_key, identity.clone());
    Ok(identity)
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

async fn table_identity_inner(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<TableIdentity, AppError> {
    let columns = describe_table_inner(client, schema, table).await?;
    let column_by_ordinal = columns
        .iter()
        .filter_map(|column| column.ordinal.map(|ordinal| (ordinal, column)))
        .collect::<HashMap<_, _>>();
    let rows = simple_query_rows(
        client,
        &format!(
            "
            select
              i.indisprimary as is_primary,
              i.indkey::text as key_columns,
              i.indpred is null as no_predicate,
              i.indexprs is null as no_expression
            from pg_index i
            join pg_class tbl on tbl.oid = i.indrelid
            join pg_class idx on idx.oid = i.indexrelid
            join pg_namespace ns on ns.oid = tbl.relnamespace
            where ns.nspname = {}
              and tbl.relname = {}
              and i.indisunique
            order by i.indisprimary desc, idx.relname
            ",
            quote_literal(schema),
            quote_literal(table),
        ),
    )
    .await?;

    let mut fallback: Option<Vec<String>> = None;
    for row in rows {
        let no_predicate = required_bool(&row, "no_predicate")?;
        let no_expression = required_bool(&row, "no_expression")?;
        if !no_predicate || !no_expression {
            continue;
        }

        let attnums = required_text(&row, "key_columns")?;
        let key_columns = attnums
            .split_whitespace()
            .filter_map(|attnum| attnum.parse::<i32>().ok())
            .filter_map(|attnum| column_by_ordinal.get(&attnum))
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        if key_columns.is_empty() {
            continue;
        }

        let all_not_null = key_columns.iter().all(|name| {
            columns
                .iter()
                .find(|column| column.name == *name)
                .and_then(|column| column.nullable)
                == Some(false)
        });

        if required_bool(&row, "is_primary")? || all_not_null {
            return Ok(TableIdentity {
                editable: true,
                columns: key_columns,
                reason: None,
            });
        }

        if fallback.is_none() {
            fallback = Some(key_columns);
        }
    }

    let reason = if fallback.is_some() {
        "Only nullable unique indexes were found; edits require a primary key or non-null unique key."
    } else {
        "Edits require a primary key or non-null unique key."
    };

    Ok(TableIdentity {
        editable: false,
        columns: Vec::new(),
        reason: Some(reason.to_string()),
    })
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

async fn run_returning_write(
    client: &Client,
    sql: &str,
    verb: &str,
) -> Result<WriteResult, AppError> {
    begin_write(client).await?;
    let result = async {
        let (rows, _) = simple_query_value_rows(client, sql).await?;
        let rows_affected = rows.len() as u64;
        Ok::<_, AppError>(WriteResult {
            rows_affected,
            message: format!("{rows_affected} row(s) {verb}."),
            columns: vec![QueryColumn {
                name: "row_json".to_string(),
                data_type: "jsonb".to_string(),
            }],
            rows,
        })
    }
    .await;
    finish_transaction(client, result).await
}

async fn run_manual_write(client: &Client, sql: &str) -> Result<WriteResult, AppError> {
    begin_write(client).await?;
    let result = async {
        let (rows, command_tags) = simple_query_value_rows(client, sql).await?;
        let rows_affected = command_tags.last().copied().unwrap_or(rows.len() as u64);
        let message = format!("{rows_affected} row(s) affected.");
        let columns = if let Some(Value::Object(first)) = rows.first() {
            first
                .keys()
                .map(|name| QueryColumn {
                    name: name.clone(),
                    data_type: "unknown".to_string(),
                })
                .collect()
        } else {
            Vec::new()
        };

        Ok::<_, AppError>(WriteResult {
            rows_affected,
            message,
            columns,
            rows,
        })
    }
    .await;
    finish_transaction(client, result).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeneratedSqlDraft {
    sql: String,
    explanation: String,
    confidence: String,
    referenced_tables: Vec<String>,
}

async fn request_generated_sql(
    http: &reqwest::Client,
    api_key: &str,
    prompt: &str,
    schema_context: &str,
) -> Result<GeneratedSqlDraft, AppError> {
    let payload = json!({
        "model": OPENAI_MODEL,
        "reasoning": { "effort": "high" },
        "input": [
            {
                "role": "system",
                "content": [{
                    "type": "input_text",
                    "text": "You write PostgreSQL for a desktop database viewer. Return exactly one safe SQL statement. Prefer SELECT queries. Never invent tables or columns. Do not include semicolons. If the user asks for a mutation, return the SQL draft but keep it as a single INSERT, UPDATE, or DELETE statement."
                }]
            },
            {
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": format!("User request:\n{prompt}\n\nDatabase context:\n{schema_context}")
                }]
            }
        ],
        "text": {
            "format": {
                "type": "json_schema",
                "name": "postgres_viewer_sql",
                "strict": true,
                "schema": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["sql", "explanation", "confidence", "referencedTables"],
                    "properties": {
                        "sql": { "type": "string" },
                        "explanation": { "type": "string" },
                        "confidence": { "type": "string", "enum": ["low", "medium", "high"] },
                        "referencedTables": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    }
                }
            }
        }
    });

    let response = http
        .post("https://api.openai.com/v1/responses")
        .header(AUTHORIZATION, format!("Bearer {api_key}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&payload)
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;

    if !status.is_success() {
        let message = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or(body);
        return Err(AppError::new("openai_error", message));
    }

    let value = serde_json::from_str::<Value>(&body)?;
    let text = extract_response_text(&value).ok_or_else(|| {
        AppError::new(
            "openai_response_error",
            "OpenAI response did not include structured SQL output.",
        )
    })?;
    serde_json::from_str::<GeneratedSqlDraft>(&strip_markdown_fence(&text)).map_err(AppError::from)
}

fn extract_response_text(value: &Value) -> Option<String> {
    if let Some(text) = value.get("output_text").and_then(Value::as_str) {
        return Some(text.to_string());
    }

    value
        .get("output")
        .and_then(Value::as_array)?
        .iter()
        .flat_map(|item| {
            item.get("content")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .find_map(|content| {
            content
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn strip_markdown_fence(text: &str) -> String {
    let trimmed = text.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }

    trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
        .to_string()
}

async fn build_schema_context(
    client: &Client,
    cache: Arc<Mutex<ConnectionCache>>,
    prompt: &str,
) -> Result<String, AppError> {
    let schemas = cached_schemas(client, Arc::clone(&cache)).await?;
    let prompt_tokens = prompt
        .to_ascii_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|token| token.len() >= 3)
        .map(str::to_string)
        .collect::<HashSet<_>>();
    let mut tables = Vec::new();

    for schema in schemas.iter().take(24) {
        let schema_tables = cached_tables(client, Arc::clone(&cache), &schema.name).await?;
        tables.extend(schema_tables);
        if tables.len() >= 100 {
            break;
        }
    }

    let mut relevant = tables
        .iter()
        .filter(|table| {
            let name = table.name.to_ascii_lowercase();
            let schema = table.schema.to_ascii_lowercase();
            prompt_tokens
                .iter()
                .any(|token| name.contains(token) || schema.contains(token))
        })
        .cloned()
        .collect::<Vec<_>>();

    if relevant.is_empty() {
        relevant = tables.iter().take(12).cloned().collect();
    } else {
        relevant.truncate(20);
    }

    let mut context = String::new();
    context.push_str("Tables and columns:\n");
    for table in relevant.iter().take(20) {
        let columns =
            cached_columns(client, Arc::clone(&cache), &table.schema, &table.name).await?;
        let identity =
            cached_identity(client, Arc::clone(&cache), &table.schema, &table.name).await?;
        let identity_columns = identity.columns.iter().cloned().collect::<HashSet<_>>();
        let column_list = columns
            .iter()
            .map(|column| {
                let marker = if identity_columns.contains(&column.name) {
                    " pk"
                } else {
                    ""
                };
                format!("{} {}{}", column.name, column.data_type, marker)
            })
            .collect::<Vec<_>>()
            .join(", ");
        context.push_str(&format!(
            "- {}.{} ({}) columns: {}\n",
            table.schema, table.name, table.kind, column_list
        ));
    }

    context.push_str("\nSample rows:\n");
    for table in relevant.iter().take(5) {
        match fetch_table_rows_inner(client, &table.schema, &table.name, 0, 3, None).await {
            Ok((rows, _)) => {
                context.push_str(&format!(
                    "- {}.{}: {}\n",
                    table.schema,
                    table.name,
                    Value::Array(rows)
                ));
            }
            Err(error) => {
                context.push_str(&format!(
                    "- {}.{}: sample unavailable ({})\n",
                    table.schema, table.name, error.message
                ));
            }
        }
    }

    Ok(context)
}

async fn cached_schemas(
    client: &Client,
    cache: Arc<Mutex<ConnectionCache>>,
) -> Result<Vec<SchemaInfo>, AppError> {
    if let Some(schemas) = cache.lock().await.schemas.clone() {
        return Ok(schemas);
    }

    let rows = simple_query_rows(
        client,
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
    cache.lock().await.schemas = Some(schemas.clone());
    Ok(schemas)
}

async fn cached_tables(
    client: &Client,
    cache: Arc<Mutex<ConnectionCache>>,
    schema: &str,
) -> Result<Vec<TableInfo>, AppError> {
    if let Some(tables) = cache.lock().await.tables.get(schema).cloned() {
        return Ok(tables);
    }

    let tables = list_tables_inner(client, schema).await?;
    cache
        .lock()
        .await
        .tables
        .insert(schema.to_string(), tables.clone());
    Ok(tables)
}

async fn simple_query_value_rows(
    client: &Client,
    sql: &str,
) -> Result<(Vec<Value>, Vec<u64>), AppError> {
    let messages = client.simple_query(sql).await?;
    let mut rows = Vec::new();
    let mut command_tags = Vec::new();

    for message in messages {
        match message {
            SimpleQueryMessage::Row(row) => {
                rows.push(simple_row_to_value(&row)?);
            }
            SimpleQueryMessage::CommandComplete(count) => command_tags.push(count),
            _ => {}
        }
    }

    Ok((rows, command_tags))
}

fn simple_row_to_value(row: &SimpleQueryRow) -> Result<Value, AppError> {
    if row.len() == 1 && row.columns()[0].name() == "row_json" {
        if let Some(json_text) = row.try_get(0)? {
            return Ok(serde_json::from_str(json_text)?);
        }
    }

    let mut object = Map::new();
    for index in 0..row.len() {
        let column = row.columns()[index].name().to_string();
        let value = row
            .try_get(index)?
            .map(|value| Value::String(value.to_string()))
            .unwrap_or(Value::Null);
        object.insert(column, value);
    }
    Ok(Value::Object(object))
}

fn ensure_editable_table(identity: &TableIdentity) -> Result<(), AppError> {
    if identity.editable {
        return Ok(());
    }

    Err(AppError::new(
        "table_not_editable",
        identity
            .reason
            .clone()
            .unwrap_or_else(|| "This table cannot be edited safely.".to_string()),
    ))
}

fn identity_where_clause(
    identity: &TableIdentity,
    key: &HashMap<String, Value>,
) -> Result<String, AppError> {
    ensure_editable_table(identity)?;
    let mut parts = Vec::new();

    for column in &identity.columns {
        let value = key.get(column).ok_or_else(|| {
            AppError::new(
                "missing_identity_value",
                format!("Missing identity value for column {column}."),
            )
        })?;
        let condition = if value.is_null() {
            format!("t.{} is null", quote_ident(column))
        } else {
            format!("t.{} = {}", quote_ident(column), sql_literal(value))
        };
        parts.push(condition);
    }

    Ok(parts.join(" and "))
}

fn sql_literal(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => quote_literal(value),
        Value::Array(_) | Value::Object(_) => quote_literal(&value.to_string()),
    }
}

async fn invalidate_table_pages(cache: &Arc<Mutex<ConnectionCache>>, schema: &str, table: &str) {
    let prefix = format!("{}::{}::", schema, table);
    cache
        .lock()
        .await
        .pages
        .retain(|key, _| !key.starts_with(&prefix));
}

async fn invalidate_all_pages(cache: &Arc<Mutex<ConnectionCache>>) {
    cache.lock().await.pages.clear();
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

async fn begin_write(client: &Client) -> Result<(), AppError> {
    client
        .batch_execute(&format!(
            "begin; set local statement_timeout = '{}';",
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

fn table_cache_key(schema: &str, table: &str) -> String {
    format!("{schema}::{table}")
}

fn page_cache_key(
    schema: &str,
    table: &str,
    page: u32,
    page_size: u32,
    sort: Option<&SortSpec>,
) -> String {
    let sort_key = sort
        .map(|sort| {
            let direction = match sort.direction {
                SortDirection::Asc => "asc",
                SortDirection::Desc => "desc",
            };
            format!("{}:{direction}", sort.column)
        })
        .unwrap_or_else(|| "none".to_string());
    format!("{schema}::{table}::{page}:{page_size}:{sort_key}")
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
    let mut profile = saved_connection_from_url(connection_url)?;
    set_generic_password(KEYCHAIN_SERVICE, &profile.id, connection_url.as_bytes())?;

    let mut connections = load_saved_connections(app)?;
    if let Some(existing) = connections
        .iter_mut()
        .find(|connection| connection.id == profile.id)
    {
        profile.label = existing.label.clone();
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

fn load_openai_api_key() -> Result<String, AppError> {
    let bytes = get_generic_password(OPENAI_KEYCHAIN_SERVICE, OPENAI_KEYCHAIN_ACCOUNT)?;
    String::from_utf8(bytes).map_err(|error| {
        AppError::new(
            "keychain_error",
            format!("OpenAI API key is not valid UTF-8: {error}"),
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

fn ensure_read_sql(sql: &str) -> Result<(), AppError> {
    let keyword = leading_sql_keyword(sql);
    if matches!(
        keyword.as_deref(),
        Some("select" | "with" | "show" | "explain" | "values")
    ) {
        return Ok(());
    }

    Err(AppError::new(
        "write_requires_confirmation",
        "Use the write action for INSERT, UPDATE, and DELETE statements.",
    ))
}

fn ensure_write_sql(sql: &str) -> Result<(), AppError> {
    let keyword = leading_sql_keyword(sql);
    if matches!(keyword.as_deref(), Some("insert" | "update" | "delete")) {
        return Ok(());
    }

    Err(AppError::new(
        "unsupported_write",
        "Only INSERT, UPDATE, and DELETE statements are supported in write mode.",
    ))
}

fn is_read_sql(sql: &str) -> bool {
    leading_sql_keyword(sql)
        .map(|keyword| {
            matches!(
                keyword.as_str(),
                "select" | "with" | "show" | "explain" | "values"
            )
        })
        .unwrap_or(false)
}

fn leading_sql_keyword(sql: &str) -> Option<String> {
    let mut rest = sql.trim_start();
    loop {
        if rest.starts_with("--") {
            if let Some((_, after)) = rest.split_once('\n') {
                rest = after.trim_start();
                continue;
            }
            return None;
        }

        if rest.starts_with("/*") {
            let end = rest.find("*/")?;
            rest = rest[end + 2..].trim_start();
            continue;
        }

        break;
    }

    rest.split(|character: char| !character.is_ascii_alphabetic())
        .next()
        .filter(|keyword| !keyword.is_empty())
        .map(|keyword| keyword.to_ascii_lowercase())
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
    use super::{
        ensure_read_sql, ensure_write_sql, has_connection_param, is_read_sql,
        normalize_connection_url, normalize_sql,
    };

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

    #[test]
    fn read_sql_rejects_mutations() {
        let error = ensure_read_sql("update users set name = 'a'").unwrap_err();
        assert_eq!(error.code, "write_requires_confirmation");
    }

    #[test]
    fn write_sql_allows_basic_mutations_only() {
        ensure_write_sql("insert into users(name) values ('a')").unwrap();
        ensure_write_sql("update users set name = 'b'").unwrap();
        ensure_write_sql("delete from users where id = 1").unwrap();
        let error = ensure_write_sql("drop table users").unwrap_err();
        assert_eq!(error.code, "unsupported_write");
    }

    #[test]
    fn is_read_sql_skips_leading_comments() {
        assert!(is_read_sql("-- comment\nselect 1"));
        assert!(is_read_sql("/* comment */ select 1"));
    }
}

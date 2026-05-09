use std::str::FromStr;

use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use tokio_postgres::config::SslMode;
use tokio_postgres::{Config, SimpleQueryMessage};

#[tokio::test]
async fn live_supabase_connection_smoke() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("Skipping live Supabase smoke test because DATABASE_URL is not set.");
        return;
    };
    let normalized = normalize_connection_url(&database_url);
    let config = Config::from_str(&normalized).expect("DATABASE_URL should parse");
    let accept_invalid_certs = config.get_ssl_mode() != SslMode::Disable;
    let mut tls_builder = TlsConnector::builder();

    if accept_invalid_certs {
        tls_builder.danger_accept_invalid_certs(true);
        tls_builder.danger_accept_invalid_hostnames(true);
    }

    let tls = MakeTlsConnector::new(tls_builder.build().expect("TLS connector should build"));
    let (client, connection) = config.connect(tls).await.expect("should connect");
    let connection_task = tokio::spawn(async move {
        if let Err(error) = connection.await {
            panic!("connection task failed: {error}");
        }
    });

    let identity_rows = simple_rows(
        &client,
        "select current_database() as database, current_user as user, inet_server_port() as port",
    )
    .await
    .expect("identity query should work");
    let identity = identity_rows.first().expect("identity row should exist");
    let database = identity.get("database").unwrap_or_default();
    let user = identity.get("user").unwrap_or_default();
    let port = identity
        .get("port")
        .unwrap_or_default()
        .parse::<i32>()
        .expect("port should parse");

    assert_eq!(database, "postgres");
    assert!(user.starts_with("postgres"));
    assert!(port > 0);

    let schemas = simple_rows(
        &client,
        "
            select schema_name
            from information_schema.schemata
            where schema_name not in ('information_schema', 'pg_catalog')
              and schema_name not like 'pg_toast%'
            order by schema_name
            ",
    )
    .await
    .expect("schema introspection should work");
    assert!(
        schemas.iter().any(|row| row.get(0) == Some("public")),
        "public schema should be visible"
    );

    let tables = simple_rows(
        &client,
        "
            select cls.relname
            from pg_class cls
            join pg_namespace ns on ns.oid = cls.relnamespace
            where ns.nspname = 'public'
              and cls.relkind in ('r', 'p', 'v', 'm', 'f')
            order by cls.relname
            limit 5
            ",
    )
    .await
    .expect("table introspection should work");

    if let Some(table) = tables.first().and_then(|row| row.get(0)) {
        let bundle_sql = format!(
            "
            with column_data as (
              select jsonb_agg(attr.attname order by attr.attnum) as columns_json
              from pg_attribute attr
              join pg_class cls on cls.oid = attr.attrelid
              join pg_namespace ns on ns.oid = cls.relnamespace
              where ns.nspname = 'public'
                and cls.relname = {table_literal}
                and attr.attnum > 0
                and not attr.attisdropped
            ),
            page_data as (
              select coalesce(jsonb_agg(to_jsonb(page_rows)), '[]'::jsonb) as rows_json
              from (
                select *
                from public.{table_ident}
                limit 1
              ) page_rows
            )
            select
              column_data.columns_json::text as columns_json,
              page_data.rows_json::text as rows_json
            from column_data, page_data
            ",
            table_literal = quote_literal(table),
            table_ident = quote_ident(table),
        );
        let bundle = simple_rows(&client, &bundle_sql)
            .await
            .expect("bundled table metadata and row fetch should work");
        let bundle = bundle.first().expect("bundle row should exist");
        assert!(bundle.get("columns_json").is_some());
        assert!(bundle.get("rows_json").is_some());
    }

    client
        .batch_execute("begin read only; set local statement_timeout = '30s';")
        .await
        .expect("read-only transaction should start");
    let answer_rows = simple_rows(&client, "select 1::int as ok")
        .await
        .expect("read-only query should run");
    client
        .batch_execute("commit")
        .await
        .expect("read-only transaction should commit");
    assert_eq!(answer_rows.first().and_then(|row| row.get("ok")), Some("1"));

    println!(
        "Connected to database={database}, user={user}, port={port}, public_objects_sampled={}",
        tables.len()
    );

    drop(client);
    connection_task.abort();
}

async fn simple_rows(
    client: &tokio_postgres::Client,
    sql: &str,
) -> Result<Vec<tokio_postgres::SimpleQueryRow>, tokio_postgres::Error> {
    Ok(client
        .simple_query(sql)
        .await?
        .into_iter()
        .filter_map(|message| match message {
            SimpleQueryMessage::Row(row) => Some(row),
            _ => None,
        })
        .collect())
}

fn normalize_connection_url(connection_url: &str) -> String {
    let connection_url = connection_url.trim();
    if connection_url.contains("sslmode=") || connection_url.contains("sslmode%3D") {
        return connection_url.to_string();
    }

    let separator = if connection_url.contains('?') {
        '&'
    } else {
        '?'
    };
    format!("{connection_url}{separator}sslmode=require")
}

fn quote_ident(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

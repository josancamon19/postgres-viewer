mod db;
mod error;
mod models;

use db::{
    cancel_query, connect, connect_saved, describe_table, disconnect, fetch_query_page,
    fetch_table_page, forget_saved_connection, list_indexes, list_saved_connections, list_schemas,
    list_tables, run_query, AppState,
};

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            connect,
            connect_saved,
            disconnect,
            list_saved_connections,
            forget_saved_connection,
            list_schemas,
            list_tables,
            describe_table,
            list_indexes,
            fetch_table_page,
            run_query,
            fetch_query_page,
            cancel_query
        ])
        .run(tauri::generate_context!())
        .expect("error while running Postgres Viewer");
}

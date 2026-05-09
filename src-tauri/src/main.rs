mod db;
mod error;
mod models;

use db::{
    cancel_query, clear_openai_api_key, connect, connect_saved, delete_row, describe_table,
    disconnect, fetch_query_page, fetch_table_page, forget_saved_connection, generate_sql,
    get_settings, insert_row, list_indexes, list_saved_connections, list_schemas, list_tables,
    refresh_table_cache, run_query, run_write_sql, set_openai_api_key, update_cell,
    update_saved_connection_label, AppState,
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
            update_saved_connection_label,
            get_settings,
            set_openai_api_key,
            clear_openai_api_key,
            list_schemas,
            list_tables,
            describe_table,
            list_indexes,
            fetch_table_page,
            refresh_table_cache,
            update_cell,
            insert_row,
            delete_row,
            run_write_sql,
            generate_sql,
            run_query,
            fetch_query_page,
            cancel_query
        ])
        .run(tauri::generate_context!())
        .expect("error while running Postgres Viewer");
}

import { invoke } from "@tauri-apps/api/core";
import type {
  AppSettings,
  ColumnInfo,
  ConnectionInfo,
  GeneratedSql,
  IndexInfo,
  QueryPage,
  SavedConnection,
  SchemaInfo,
  SortSpec,
  TableInfo,
  TablePage,
  WriteResult,
} from "./types";

export const api = {
  connect(connectionUrl: string) {
    return invoke<ConnectionInfo>("connect", { connectionUrl });
  },
  connectSaved(savedConnectionId: string) {
    return invoke<ConnectionInfo>("connect_saved", { savedConnectionId });
  },
  listSavedConnections() {
    return invoke<SavedConnection[]>("list_saved_connections");
  },
  forgetSavedConnection(savedConnectionId: string) {
    return invoke<SavedConnection[]>("forget_saved_connection", { savedConnectionId });
  },
  updateSavedConnectionLabel(savedConnectionId: string, label: string) {
    return invoke<SavedConnection[]>("update_saved_connection_label", { savedConnectionId, label });
  },
  getSettings() {
    return invoke<AppSettings>("get_settings");
  },
  setOpenaiApiKey(apiKey: string) {
    return invoke<AppSettings>("set_openai_api_key", { apiKey });
  },
  clearOpenaiApiKey() {
    return invoke<AppSettings>("clear_openai_api_key");
  },
  disconnect(connectionId: string) {
    return invoke<void>("disconnect", { connectionId });
  },
  listSchemas(connectionId: string) {
    return invoke<SchemaInfo[]>("list_schemas", { connectionId });
  },
  listTables(connectionId: string, schema: string) {
    return invoke<TableInfo[]>("list_tables", { connectionId, schema });
  },
  describeTable(connectionId: string, schema: string, table: string) {
    return invoke<ColumnInfo[]>("describe_table", { connectionId, schema, table });
  },
  listIndexes(connectionId: string, schema: string, table: string) {
    return invoke<IndexInfo[]>("list_indexes", { connectionId, schema, table });
  },
  fetchTablePage(
    connectionId: string,
    schema: string,
    table: string,
    page: number,
    pageSize: number,
    sort: SortSpec | null,
  ) {
    return invoke<TablePage>("fetch_table_page", {
      connectionId,
      schema,
      table,
      page,
      pageSize,
      sort,
    });
  },
  refreshTableCache(
    connectionId: string,
    schema: string,
    table: string,
    page: number,
    pageSize: number,
    sort: SortSpec | null,
  ) {
    return invoke<TablePage>("refresh_table_cache", {
      connectionId,
      schema,
      table,
      page,
      pageSize,
      sort,
    });
  },
  updateCell(
    connectionId: string,
    schema: string,
    table: string,
    key: Record<string, unknown>,
    column: string,
    value: unknown,
  ) {
    return invoke<WriteResult>("update_cell", { connectionId, schema, table, key, column, value });
  },
  insertRow(
    connectionId: string,
    schema: string,
    table: string,
    values: Record<string, unknown>,
  ) {
    return invoke<WriteResult>("insert_row", { connectionId, schema, table, values });
  },
  deleteRow(connectionId: string, schema: string, table: string, key: Record<string, unknown>) {
    return invoke<WriteResult>("delete_row", { connectionId, schema, table, key });
  },
  runWriteSql(connectionId: string, sql: string) {
    return invoke<WriteResult>("run_write_sql", { connectionId, sql });
  },
  generateSql(connectionId: string, prompt: string) {
    return invoke<GeneratedSql>("generate_sql", { connectionId, prompt });
  },
  runQuery(connectionId: string, sql: string, pageSize: number, requestId: string) {
    return invoke<QueryPage>("run_query", { connectionId, sql, pageSize, requestId });
  },
  fetchQueryPage(handleId: string, page: number) {
    return invoke<QueryPage>("fetch_query_page", { handleId, page });
  },
  cancelQuery(requestId: string) {
    return invoke<boolean>("cancel_query", { requestId });
  },
};

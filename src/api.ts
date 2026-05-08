import { invoke } from "@tauri-apps/api/core";
import type {
  ColumnInfo,
  ConnectionInfo,
  IndexInfo,
  QueryPage,
  SavedConnection,
  SchemaInfo,
  SortSpec,
  TableInfo,
  TablePage,
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

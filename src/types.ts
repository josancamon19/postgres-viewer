export type SortDirection = "asc" | "desc";

export interface AppError {
  code: string;
  message: string;
}

export interface ConnectionInfo {
  id: string;
  savedConnectionId: string;
  label: string;
  database: string;
  user: string;
}

export interface SavedConnection {
  id: string;
  label: string;
  host: string;
  database: string;
  user: string;
  lastUsed: number;
}

export interface SchemaInfo {
  name: string;
}

export interface TableInfo {
  schema: string;
  name: string;
  kind: string;
}

export interface ColumnInfo {
  name: string;
  dataType: string;
  nullable: boolean | null;
  ordinal: number | null;
  defaultValue: string | null;
}

export interface QueryColumn {
  name: string;
  dataType: string;
}

export interface IndexInfo {
  name: string;
  definition: string;
  unique: boolean;
  primary: boolean;
}

export interface TableIdentity {
  editable: boolean;
  columns: string[];
  reason: string | null;
}

export interface SortSpec {
  column: string;
  direction: SortDirection;
}

export type ResultRow = Record<string, unknown>;

export interface TablePage {
  columns: ColumnInfo[];
  indexes: IndexInfo[];
  rows: ResultRow[];
  page: number;
  pageSize: number;
  hasMore: boolean;
  fromCache: boolean;
  identity: TableIdentity;
}

export interface QueryPage {
  handleId: string;
  columns: QueryColumn[];
  rows: ResultRow[];
  page: number;
  pageSize: number;
  hasMore: boolean;
}

export interface AppSettings {
  hasOpenaiApiKey: boolean;
  openaiModel: string;
}

export interface GeneratedSql {
  sql: string;
  explanation: string;
  confidence: "low" | "medium" | "high" | string;
  referencedTables: string[];
  autoRun: boolean;
}

export interface WriteResult {
  rowsAffected: number;
  message: string;
  columns: QueryColumn[];
  rows: ResultRow[];
}

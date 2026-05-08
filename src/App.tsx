import { useEffect, useMemo, useState } from "react";
import {
  Database,
  FileSearch,
  Loader2,
  Play,
  RefreshCw,
  Search,
  Square,
  Table2,
  Trash2,
  Unplug,
} from "lucide-react";
import { api } from "./api";
import { ResultGrid } from "./ResultGrid";
import type {
  AppError,
  ConnectionInfo,
  IndexInfo,
  QueryPage,
  SavedConnection,
  SchemaInfo,
  SortSpec,
  TableInfo,
  TablePage,
} from "./types";

const PAGE_SIZE = 500;

type WorkspaceTab = "table" | "query";

function App() {
  const [connectionUrl, setConnectionUrl] = useState("");
  const [savedConnections, setSavedConnections] = useState<SavedConnection[]>([]);
  const [connection, setConnection] = useState<ConnectionInfo | null>(null);
  const [schemas, setSchemas] = useState<SchemaInfo[]>([]);
  const [selectedSchema, setSelectedSchema] = useState<string>("");
  const [tables, setTables] = useState<TableInfo[]>([]);
  const [selectedTable, setSelectedTable] = useState<TableInfo | null>(null);
  const [indexes, setIndexes] = useState<IndexInfo[]>([]);
  const [tablePage, setTablePage] = useState<TablePage | null>(null);
  const [tableSort, setTableSort] = useState<SortSpec | null>(null);
  const [sql, setSql] = useState("select now();");
  const [queryPage, setQueryPage] = useState<QueryPage | null>(null);
  const [activeTab, setActiveTab] = useState<WorkspaceTab>("table");
  const [loading, setLoading] = useState(false);
  const [queryRunning, setQueryRunning] = useState(false);
  const [requestId, setRequestId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const resultColumns = useMemo(
    () =>
      tablePage?.columns.map((column) => ({
        name: column.name,
        dataType: column.dataType,
      })) ?? [],
    [tablePage],
  );

  useEffect(() => {
    void refreshSavedConnections();
  }, []);

  useEffect(() => {
    if (!connection) return;

    api
      .listSchemas(connection.id)
      .then((items) => {
        setSchemas(items);
        setSelectedSchema((current) => current || items[0]?.name || "");
      })
      .catch((caught) => setError(errorMessage(caught)));
  }, [connection]);

  useEffect(() => {
    if (!connection || !selectedSchema) return;

    void loadTables(selectedSchema);
  }, [connection, selectedSchema]);

  async function loadTables(schema: string) {
    if (!connection) return;

    setLoading(true);
    api
      .listTables(connection.id, schema)
      .then((items) => {
        setTables(items);
        setSelectedTable(null);
        setIndexes([]);
        setTablePage(null);
      })
      .catch((caught) => setError(errorMessage(caught)))
      .finally(() => setLoading(false));
  }

  async function connect() {
    setError(null);
    setLoading(true);
    try {
      const nextConnection = await api.connect(connectionUrl);
      setConnection(nextConnection);
      setConnectionUrl("");
      await refreshSavedConnections();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setLoading(false);
    }
  }

  async function connectSaved(savedConnectionId: string) {
    setError(null);
    setLoading(true);
    try {
      const nextConnection = await api.connectSaved(savedConnectionId);
      setConnection(nextConnection);
      await refreshSavedConnections();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setLoading(false);
    }
  }

  async function refreshSavedConnections() {
    try {
      setSavedConnections(await api.listSavedConnections());
    } catch (caught) {
      setError(errorMessage(caught));
    }
  }

  async function forgetSavedConnection(savedConnectionId: string) {
    try {
      setSavedConnections(await api.forgetSavedConnection(savedConnectionId));
    } catch (caught) {
      setError(errorMessage(caught));
    }
  }

  async function disconnect() {
    if (connection) {
      await api.disconnect(connection.id).catch(() => undefined);
    }
    setConnection(null);
    setSchemas([]);
    setSelectedSchema("");
    setTables([]);
    setSelectedTable(null);
    setIndexes([]);
    setTablePage(null);
    setQueryPage(null);
  }

  async function openTable(table: TableInfo, page = 0, sort = tableSort) {
    if (!connection) return;

    setError(null);
    setLoading(true);
    setSelectedTable(table);
    setActiveTab("table");

    try {
      const [nextIndexes, nextPage] = await Promise.all([
        api.listIndexes(connection.id, table.schema, table.name),
        api.fetchTablePage(connection.id, table.schema, table.name, page, PAGE_SIZE, sort),
      ]);
      setIndexes(nextIndexes);
      setTablePage(nextPage);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setLoading(false);
    }
  }

  async function selectTable(table: TableInfo) {
    setTableSort(null);
    await openTable(table, 0, null);
  }

  async function refreshTable(page = tablePage?.page ?? 0, sort = tableSort) {
    if (!selectedTable) return;
    await openTable(selectedTable, page, sort);
  }

  async function changeTableSort(sort: SortSpec | null) {
    setTableSort(sort);
    if (!selectedTable) return;
    await openTable(selectedTable, 0, sort);
  }

  async function runSql() {
    if (!connection || queryRunning) return;

    const nextRequestId = crypto.randomUUID();
    setRequestId(nextRequestId);
    setError(null);
    setQueryRunning(true);
    setActiveTab("query");

    try {
      const page = await api.runQuery(connection.id, sql, PAGE_SIZE, nextRequestId);
      setQueryPage(page);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setQueryRunning(false);
      setRequestId(null);
    }
  }

  async function cancelRunningQuery() {
    if (!requestId) return;
    await api.cancelQuery(requestId).catch((caught) => setError(errorMessage(caught)));
  }

  async function loadQueryPage(page: number) {
    if (!queryPage) return;

    setError(null);
    setQueryRunning(true);
    try {
      setQueryPage(await api.fetchQueryPage(queryPage.handleId, page));
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setQueryRunning(false);
    }
  }

  if (!connection) {
    return (
      <main className="connectView">
        <form
          className="connectPanel"
          onSubmit={(event) => {
            event.preventDefault();
            void connect();
          }}
        >
          <div className="connectTitle">
            <Database size={22} />
            <h1>Postgres Viewer</h1>
          </div>
          <label htmlFor="connectionUrl">Connection URL</label>
          <div className="connectionInputRow">
            <input
              id="connectionUrl"
              value={connectionUrl}
              spellCheck={false}
              type="password"
              placeholder="postgresql://postgres:password@host:5432/postgres?sslmode=require"
              onChange={(event) => setConnectionUrl(event.target.value)}
            />
            <button type="submit" disabled={loading || !connectionUrl.trim()} title="Connect">
              {loading ? <Loader2 className="spin" size={18} /> : <Database size={18} />}
              Connect
            </button>
          </div>
          {savedConnections.length > 0 && (
            <section className="savedConnections">
              <div className="savedConnectionsHeader">
                <span>Recent connections</span>
              </div>
              <div className="savedConnectionList">
                {savedConnections.map((savedConnection) => (
                  <div className="savedConnectionItem" key={savedConnection.id}>
                    <button
                      type="button"
                      className="savedConnectionMain"
                      disabled={loading}
                      onClick={() => connectSaved(savedConnection.id)}
                    >
                      <strong>{savedConnection.label}</strong>
                      <span>
                        {savedConnection.user} · {formatSavedConnectionDate(savedConnection.lastUsed)}
                      </span>
                    </button>
                    <button
                      type="button"
                      className="iconButton"
                      title="Forget connection"
                      onClick={() => forgetSavedConnection(savedConnection.id)}
                    >
                      <Trash2 size={15} />
                    </button>
                  </div>
                ))}
              </div>
            </section>
          )}
          {error && <div className="errorLine">{error}</div>}
        </form>
      </main>
    );
  }

  return (
    <main className="appShell">
      <aside className="sidebar">
        <div className="sidebarHeader">
          <div>
            <strong>{connection.database}</strong>
            <span>{connection.user}</span>
          </div>
          <button type="button" className="iconButton" title="Disconnect" onClick={disconnect}>
            <Unplug size={16} />
          </button>
        </div>

        <div className="schemaSelectRow">
          <select
            value={selectedSchema}
            onChange={(event) => setSelectedSchema(event.target.value)}
            title="Schema"
          >
            {schemas.map((schema) => (
              <option key={schema.name} value={schema.name}>
                {schema.name}
              </option>
            ))}
          </select>
          <button
            type="button"
            className="iconButton"
            title="Refresh schema"
            onClick={() => selectedSchema && loadTables(selectedSchema)}
          >
            <RefreshCw size={16} />
          </button>
        </div>

        <div className="tableList">
          {tables.map((table) => (
            <button
              key={`${table.schema}.${table.name}`}
              type="button"
              className={selectedTable?.name === table.name ? "tableItem active" : "tableItem"}
              onClick={() => selectTable(table)}
            >
              <Table2 size={15} />
              <span>{table.name}</span>
              <small>{table.kind}</small>
            </button>
          ))}
        </div>
      </aside>

      <section className="workspace">
        <div className="topBar">
          <div className="tabs">
            <button
              type="button"
              className={activeTab === "table" ? "tab active" : "tab"}
              onClick={() => setActiveTab("table")}
            >
              <Table2 size={16} />
              Table
            </button>
            <button
              type="button"
              className={activeTab === "query" ? "tab active" : "tab"}
              onClick={() => setActiveTab("query")}
            >
              <FileSearch size={16} />
              Query
            </button>
          </div>
          {error && <div className="topError">{error}</div>}
        </div>

        {activeTab === "table" ? (
          <div className="tableWorkspace">
            <section className="objectHeader">
              <div>
                <h2>{selectedTable ? selectedTable.name : "Select a table"}</h2>
                <span>{selectedTable ? `${selectedTable.schema} · ${selectedTable.kind}` : selectedSchema}</span>
              </div>
              <button
                type="button"
                className="secondaryButton"
                disabled={!selectedTable || loading}
                onClick={() => refreshTable()}
                title="Refresh rows"
              >
                {loading ? <Loader2 className="spin" size={16} /> : <RefreshCw size={16} />}
                Refresh
              </button>
            </section>

            {selectedTable && (
              <details className="indexesDisclosure">
                <summary>
                  <span>
                    <Search size={15} />
                    Indexes
                  </span>
                  <small>{indexes.length}</small>
                </summary>
                <div className="indexList">
                  {indexes.length === 0 ? (
                    <span className="indexEmpty">No indexes</span>
                  ) : (
                    indexes.map((index) => (
                      <span key={index.name} title={index.definition}>
                        {index.name}
                        <small>{index.primary ? "primary" : index.unique ? "unique" : "index"}</small>
                      </span>
                    ))
                  )}
                </div>
              </details>
            )}

            <section className="resultArea">
              <ResultGrid
                columns={resultColumns}
                rows={tablePage?.rows ?? []}
                loading={loading}
                sort={tableSort}
                onSortChange={changeTableSort}
              />
            </section>

            <PageBar
              page={tablePage?.page ?? 0}
              hasMore={tablePage?.hasMore ?? false}
              disabled={!tablePage || loading}
              onPage={(page) => refreshTable(page)}
            />
          </div>
        ) : (
          <div className="queryWorkspace">
            <section className="queryEditor">
              <textarea
                value={sql}
                spellCheck={false}
                onChange={(event) => setSql(event.target.value)}
              />
              <div className="queryActions">
                <button
                  type="button"
                  className="primaryButton"
                  disabled={queryRunning || !sql.trim()}
                  onClick={runSql}
                  title="Run query"
                >
                  {queryRunning ? <Loader2 className="spin" size={16} /> : <Play size={16} />}
                  Run
                </button>
                <button
                  type="button"
                  className="secondaryButton"
                  disabled={!queryRunning || !requestId}
                  onClick={cancelRunningQuery}
                  title="Cancel query"
                >
                  <Square size={14} />
                  Cancel
                </button>
              </div>
            </section>

            <section className="resultArea">
              <ResultGrid
                columns={queryPage?.columns ?? []}
                rows={queryPage?.rows ?? []}
                loading={queryRunning}
              />
            </section>

            <PageBar
              page={queryPage?.page ?? 0}
              hasMore={queryPage?.hasMore ?? false}
              disabled={!queryPage || queryRunning}
              onPage={loadQueryPage}
            />
          </div>
        )}
      </section>
    </main>
  );
}

function PageBar({
  page,
  hasMore,
  disabled,
  onPage,
}: {
  page: number;
  hasMore: boolean;
  disabled: boolean;
  onPage: (page: number) => void;
}) {
  return (
    <footer className="pageBar">
      <button
        type="button"
        className="secondaryButton"
        disabled={disabled || page === 0}
        onClick={() => onPage(page - 1)}
      >
        Previous
      </button>
      <span>Page {page + 1}</span>
      <button
        type="button"
        className="secondaryButton"
        disabled={disabled || !hasMore}
        onClick={() => onPage(page + 1)}
      >
        Next
      </button>
    </footer>
  );
}

function errorMessage(caught: unknown) {
  const error = caught as Partial<AppError>;
  if (error?.message) return error.message;
  if (caught instanceof Error) return caught.message;
  return String(caught);
}

function formatSavedConnectionDate(timestamp: number) {
  if (!timestamp) return "never";
  return new Date(timestamp * 1000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

export default App;

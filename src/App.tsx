import { useEffect, useMemo, useState } from "react";
import {
  Database,
  Edit3,
  FileSearch,
  Loader2,
  Play,
  RefreshCw,
  Search,
  Settings,
  Sparkles,
  Square,
  Table2,
  Trash2,
  Unplug,
} from "lucide-react";
import { api } from "./api";
import { ResultGrid } from "./ResultGrid";
import type {
  AppError,
  AppSettings,
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

const PAGE_SIZE = 500;

type WorkspaceTab = "table" | "query" | "settings";

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
  const [tablePages, setTablePages] = useState<Record<string, TablePage>>({});
  const [tableSort, setTableSort] = useState<SortSpec | null>(null);
  const [editMode, setEditMode] = useState(false);
  const [sql, setSql] = useState("select now();");
  const [aiPrompt, setAiPrompt] = useState("");
  const [generatedSql, setGeneratedSql] = useState<GeneratedSql | null>(null);
  const [queryPage, setQueryPage] = useState<QueryPage | null>(null);
  const [writeResult, setWriteResult] = useState<WriteResult | null>(null);
  const [pendingWriteSql, setPendingWriteSql] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<WorkspaceTab>("table");
  const [loading, setLoading] = useState(false);
  const [queryRunning, setQueryRunning] = useState(false);
  const [aiRunning, setAiRunning] = useState(false);
  const [requestId, setRequestId] = useState<string | null>(null);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [openaiKeyInput, setOpenaiKeyInput] = useState("");
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
    void refreshSettings();
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
        setEditMode(false);
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

  async function refreshSettings() {
    try {
      setSettings(await api.getSettings());
    } catch (caught) {
      setError(errorMessage(caught));
    }
  }

  async function saveOpenaiKey() {
    setError(null);
    try {
      setSettings(await api.setOpenaiApiKey(openaiKeyInput));
      setOpenaiKeyInput("");
    } catch (caught) {
      setError(errorMessage(caught));
    }
  }

  async function clearOpenaiKey() {
    setError(null);
    try {
      setSettings(await api.clearOpenaiApiKey());
      setOpenaiKeyInput("");
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

  async function renameSavedConnection(savedConnectionId: string, currentLabel: string) {
    const label = window.prompt("Connection name", currentLabel);
    if (label === null) return;

    try {
      const nextConnections = await api.updateSavedConnectionLabel(savedConnectionId, label);
      setSavedConnections(nextConnections);
      setConnection((current) =>
        current?.savedConnectionId === savedConnectionId ? { ...current, label: label.trim() } : current,
      );
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
    setTablePages({});
    setEditMode(false);
    setQueryPage(null);
    setWriteResult(null);
  }

  async function openTable(table: TableInfo, page = 0, sort = tableSort) {
    if (!connection) return;

    setError(null);
    setSelectedTable(table);
    setEditMode(false);
    setActiveTab("table");
    const cacheKey = tablePageCacheKey(table, page, sort);
    const cachedPage = tablePages[cacheKey];
    setTablePage(cachedPage ?? null);
    setLoading(true);

    try {
      const [nextIndexes, nextPage] = await Promise.all([
        api.listIndexes(connection.id, table.schema, table.name),
        api.fetchTablePage(connection.id, table.schema, table.name, page, PAGE_SIZE, sort),
      ]);
      setIndexes(nextIndexes);
      setTablePage(nextPage);
      setTablePages((current) => ({ ...current, [cacheKey]: nextPage }));

      if (nextPage.fromCache) {
        api
          .refreshTableCache(connection.id, table.schema, table.name, page, PAGE_SIZE, sort)
          .then((freshPage) => {
            setTablePages((current) => ({ ...current, [cacheKey]: freshPage }));
            setTablePage((current) => (current === nextPage ? freshPage : current));
          })
          .catch((caught) => setError(errorMessage(caught)));
      }
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
    if (!selectedTable || !connection) return;
    setLoading(true);
    setError(null);

    try {
      const nextPage = await api.refreshTableCache(
        connection.id,
        selectedTable.schema,
        selectedTable.name,
        page,
        PAGE_SIZE,
        sort,
      );
      setTablePage(nextPage);
      setTablePages((current) => ({
        ...current,
        [tablePageCacheKey(selectedTable, page, sort)]: nextPage,
      }));
      setIndexes(await api.listIndexes(connection.id, selectedTable.schema, selectedTable.name));
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setLoading(false);
    }
  }

  async function changeTableSort(sort: SortSpec | null) {
    setTableSort(sort);
    if (!selectedTable) return;
    await openTable(selectedTable, 0, sort);
  }

  async function updateGridCell(row: Record<string, unknown>, column: string, value: unknown) {
    if (!connection || !selectedTable || !tablePage) return;
    setError(null);
    setLoading(true);
    try {
      await api.updateCell(
        connection.id,
        selectedTable.schema,
        selectedTable.name,
        rowIdentity(row, tablePage.identity.columns),
        column,
        value,
      );
      await refreshTable(tablePage.page, tableSort);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setLoading(false);
    }
  }

  async function insertGridRow(values: Record<string, unknown>) {
    if (!connection || !selectedTable || !tablePage) return;
    setError(null);
    setLoading(true);
    try {
      await api.insertRow(connection.id, selectedTable.schema, selectedTable.name, values);
      await refreshTable(tablePage.page, tableSort);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setLoading(false);
    }
  }

  async function deleteGridRow(row: Record<string, unknown>) {
    if (!connection || !selectedTable || !tablePage) return;
    setError(null);
    setLoading(true);
    try {
      await api.deleteRow(
        connection.id,
        selectedTable.schema,
        selectedTable.name,
        rowIdentity(row, tablePage.identity.columns),
      );
      await refreshTable(tablePage.page, tableSort);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setLoading(false);
    }
  }

  async function runSql() {
    if (!connection || queryRunning) return;
    if (isWriteSql(sql)) {
      setPendingWriteSql(sql);
      return;
    }

    await runReadSql(sql);
  }

  async function runReadSql(nextSql: string) {
    if (!connection || queryRunning) return;

    const nextRequestId = crypto.randomUUID();
    setRequestId(nextRequestId);
    setError(null);
    setQueryRunning(true);
    setActiveTab("query");
    setWriteResult(null);

    try {
      const page = await api.runQuery(connection.id, nextSql, PAGE_SIZE, nextRequestId);
      setQueryPage(page);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setQueryRunning(false);
      setRequestId(null);
    }
  }

  async function confirmWriteSql() {
    if (!connection || !pendingWriteSql) return;

    setError(null);
    setQueryRunning(true);
    setActiveTab("query");
    try {
      const result = await api.runWriteSql(connection.id, pendingWriteSql);
      setWriteResult(result);
      setQueryPage(null);
      setPendingWriteSql(null);
      if (selectedTable) {
        await refreshTable(tablePage?.page ?? 0, tableSort);
      }
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setQueryRunning(false);
    }
  }

  async function generateQueryFromPrompt() {
    if (!connection || aiRunning || !aiPrompt.trim()) return;

    setError(null);
    setAiRunning(true);
    try {
      const generated = await api.generateSql(connection.id, aiPrompt);
      setGeneratedSql(generated);
      setSql(generated.sql);
      if (generated.autoRun) {
        await runReadSql(generated.sql);
      } else {
        setError("Generated a write draft. Review it and press Run to confirm.");
      }
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setAiRunning(false);
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
                      title="Rename connection"
                      onClick={() =>
                        renameSavedConnection(savedConnection.id, savedConnection.label)
                      }
                    >
                      <Edit3 size={15} />
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
            <strong>{connection.label || connection.database}</strong>
            <span>
              {connection.database} · {connection.user}
            </span>
          </div>
          <button
            type="button"
            className="iconButton"
            title="Rename connection"
            onClick={() => renameSavedConnection(connection.savedConnectionId, connection.label)}
          >
            <Edit3 size={15} />
          </button>
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
            <button
              type="button"
              className={activeTab === "settings" ? "tab active" : "tab"}
              onClick={() => setActiveTab("settings")}
            >
              <Settings size={16} />
              Settings
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
                editable={tablePage?.identity.editable ?? false}
                editMode={editMode}
                identityColumns={tablePage?.identity.columns ?? []}
                editDisabledReason={tablePage?.identity.reason}
                onEditModeChange={setEditMode}
                onUpdateCell={updateGridCell}
                onInsertRow={insertGridRow}
                onDeleteRow={deleteGridRow}
              />
            </section>

            <PageBar
              page={tablePage?.page ?? 0}
              hasMore={tablePage?.hasMore ?? false}
              disabled={!tablePage || loading}
              onPage={(page) => refreshTable(page)}
            />
          </div>
        ) : activeTab === "query" ? (
          <div className="queryWorkspace">
            <section className="queryEditor">
              <div className="aiPromptRow">
                <input
                  value={aiPrompt}
                  placeholder="What do you want to view?"
                  onChange={(event) => setAiPrompt(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      void generateQueryFromPrompt();
                    }
                  }}
                />
                <button
                  type="button"
                  className="sparkButton"
                  disabled={aiRunning || !aiPrompt.trim() || !settings?.hasOpenaiApiKey}
                  title={
                    settings?.hasOpenaiApiKey
                      ? `Generate SQL with ${settings.openaiModel}`
                      : "Add an OpenAI API key in Settings"
                  }
                  onClick={generateQueryFromPrompt}
                >
                  {aiRunning ? <Loader2 className="spin" size={16} /> : <Sparkles size={16} />}
                </button>
              </div>
              <textarea
                value={sql}
                spellCheck={false}
                onChange={(event) => setSql(event.target.value)}
              />
              {generatedSql && (
                <div className="generatedSqlNote">
                  <span>{generatedSql.explanation}</span>
                  <small>
                    {generatedSql.confidence} confidence
                    {generatedSql.referencedTables.length > 0
                      ? ` · ${generatedSql.referencedTables.join(", ")}`
                      : ""}
                  </small>
                </div>
              )}
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
                columns={queryPage?.columns ?? writeResult?.columns ?? []}
                rows={queryPage?.rows ?? writeResult?.rows ?? []}
                loading={queryRunning}
              />
            </section>

            {writeResult && <div className="writeResultLine">{writeResult.message}</div>}

            <PageBar
              page={queryPage?.page ?? 0}
              hasMore={queryPage?.hasMore ?? false}
              disabled={!queryPage || queryRunning}
              onPage={loadQueryPage}
            />
          </div>
        ) : (
          <div className="settingsWorkspace">
            <section className="settingsPanel">
              <div className="settingsHeader">
                <h2>Settings</h2>
                <span>{settings?.openaiModel ?? "gpt-5.5"} for SQL generation</span>
              </div>

              <label htmlFor="openaiKey">OpenAI API key</label>
              <div className="settingsInputRow">
                <input
                  id="openaiKey"
                  type="password"
                  value={openaiKeyInput}
                  placeholder={settings?.hasOpenaiApiKey ? "Key saved in Keychain" : "sk-..."}
                  onChange={(event) => setOpenaiKeyInput(event.target.value)}
                />
                <button
                  type="button"
                  className="primaryButton"
                  disabled={!openaiKeyInput.trim()}
                  onClick={saveOpenaiKey}
                >
                  Save
                </button>
                <button
                  type="button"
                  className="secondaryButton"
                  disabled={!settings?.hasOpenaiApiKey}
                  onClick={clearOpenaiKey}
                >
                  Clear
                </button>
              </div>
              <p className="settingsHint">
                {settings?.hasOpenaiApiKey
                  ? "API key is saved locally in macOS Keychain."
                  : "Add a key to enable natural-language SQL generation."}
              </p>
            </section>
          </div>
        )}
      </section>
      {pendingWriteSql && (
        <div className="modalOverlay">
          <div className="confirmModal">
            <h3>Run write statement?</h3>
            <pre>{pendingWriteSql}</pre>
            <div className="modalActions">
              <button type="button" className="secondaryButton" onClick={() => setPendingWriteSql(null)}>
                Cancel
              </button>
              <button type="button" className="primaryButton" onClick={confirmWriteSql}>
                Run write
              </button>
            </div>
          </div>
        </div>
      )}
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

function tablePageCacheKey(table: TableInfo, page: number, sort: SortSpec | null) {
  const sortKey = sort ? `${sort.column}:${sort.direction}` : "none";
  return `${table.schema}.${table.name}:${page}:${sortKey}`;
}

function rowIdentity(row: Record<string, unknown>, identityColumns: string[]) {
  return identityColumns.reduce<Record<string, unknown>>((key, column) => {
    key[column] = row[column];
    return key;
  }, {});
}

function isWriteSql(sql: string) {
  const normalized = sql
    .trim()
    .replace(/^(?:--.*\n|\s|\/\*[\s\S]*?\*\/)*/g, "")
    .toLowerCase();
  return /^(insert|update|delete)\b/.test(normalized);
}

export default App;

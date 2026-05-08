import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowDown, ArrowLeft, ArrowUp, ArrowUpDown, Table2 } from "lucide-react";
import type { QueryColumn, ResultRow, SortSpec } from "./types";

type GridColumn = QueryColumn;
type GridView = "grid" | "detail";

interface ResultGridProps {
  columns: GridColumn[];
  rows: ResultRow[];
  loading?: boolean;
  sort?: SortSpec | null;
  onSortChange?: (sort: SortSpec | null) => void;
}

const ROW_HEIGHT = 32;
const MIN_WRAP_ROW_HEIGHT = 44;
const MAX_WRAP_ROW_HEIGHT = 220;
const HEADER_HEIGHT = 36;
const OVERSCAN_PX = 320;

export function ResultGrid({ columns, rows, loading = false, sort, onSortChange }: ResultGridProps) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(520);
  const [widths, setWidths] = useState<Record<string, number>>({});
  const [localSort, setLocalSort] = useState<SortSpec | null>(null);
  const [wrapCells, setWrapCells] = useState(false);
  const [activeView, setActiveView] = useState<GridView>("grid");
  const [detailRowIndex, setDetailRowIndex] = useState<number | null>(null);
  const activeSort = sort === undefined ? localSort : sort;

  useEffect(() => {
    setWidths((current) => {
      const next = { ...current };
      for (const column of columns) {
        if (!next[column.name]) {
          next[column.name] = Math.min(Math.max(column.name.length * 10 + 60, 120), 280);
        }
      }
      return next;
    });
  }, [columns]);

  useEffect(() => {
    if (!scrollRef.current) return;

    const observer = new ResizeObserver(([entry]) => {
      setViewportHeight(entry.contentRect.height);
    });
    observer.observe(scrollRef.current);
    return () => observer.disconnect();
  }, []);

  const displayedRows = useMemo(() => {
    if (!activeSort || onSortChange) return rows;

    return [...rows].sort((left, right) => {
      const leftValue = left[activeSort.column];
      const rightValue = right[activeSort.column];
      const comparison = compareValues(leftValue, rightValue);
      return activeSort.direction === "asc" ? comparison : -comparison;
    });
  }, [activeSort, onSortChange, rows]);

  useEffect(() => {
    setActiveView("grid");
    setDetailRowIndex(null);
  }, [columns, rows]);

  const detailRow =
    detailRowIndex === null || detailRowIndex >= displayedRows.length
      ? null
      : displayedRows[detailRowIndex];
  const rowScrollTop = Math.max(0, scrollTop - HEADER_HEIGHT);
  const totalWidth = columns.reduce((sum, column) => sum + (widths[column.name] ?? 160), 0);
  const templateColumns = columns.map((column) => `${widths[column.name] ?? 160}px`).join(" ");
  const { metrics, totalRowsHeight } = useMemo(() => {
    let top = 0;
    const nextMetrics = displayedRows.map((row, index) => {
      const height = wrapCells ? estimateWrappedRowHeight(row, columns, widths) : ROW_HEIGHT;
      const metric = { row, index, top, height };
      top += height;
      return metric;
    });

    return { metrics: nextMetrics, totalRowsHeight: top };
  }, [columns, displayedRows, widths, wrapCells]);
  const visibleMetrics = metrics.filter(
    (metric) =>
      metric.top + metric.height >= rowScrollTop - OVERSCAN_PX &&
      metric.top <= rowScrollTop + viewportHeight + OVERSCAN_PX,
  );

  function updateSort(column: string) {
    const next =
      activeSort?.column !== column
        ? { column, direction: "asc" as const }
        : activeSort.direction === "asc"
          ? { column, direction: "desc" as const }
          : null;

    if (onSortChange) {
      onSortChange(next);
    } else {
      setLocalSort(next);
    }
  }

  function openRowDetail(index: number) {
    setDetailRowIndex(index);
    setActiveView("detail");
  }

  function startResize(column: string, event: React.PointerEvent) {
    event.preventDefault();
    event.stopPropagation();
    const startX = event.clientX;
    const startWidth = widths[column] ?? 160;

    const onMove = (moveEvent: PointerEvent) => {
      const nextWidth = Math.max(84, startWidth + moveEvent.clientX - startX);
      setWidths((current) => ({ ...current, [column]: nextWidth }));
    };

    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  }

  if (columns.length === 0) {
    return (
      <div className="gridEmpty">
        <span>{loading ? "Loading..." : "No columns"}</span>
      </div>
    );
  }

  return (
    <div className="gridFrame">
      <div className="gridToolbar">
        <div className="gridViewTabs">
          <button
            type="button"
            className={activeView === "grid" ? "gridViewTab active" : "gridViewTab"}
            onClick={() => setActiveView("grid")}
          >
            <Table2 size={14} />
            Grid
          </button>
          {detailRow && (
            <button
              type="button"
              className={activeView === "detail" ? "gridViewTab active" : "gridViewTab"}
              onClick={() => setActiveView("detail")}
            >
              Row detail
              <small>{detailRowIndex! + 1}</small>
            </button>
          )}
        </div>

        {activeView === "grid" ? (
          <div className="gridToolbarRight">
            <label className="gridToggle">
              <input
                type="checkbox"
                checked={wrapCells}
                onChange={(event) => setWrapCells(event.target.checked)}
              />
              <span>Wrap cells</span>
            </label>
            <span className="gridCount">{displayedRows.length} rows</span>
          </div>
        ) : (
          <button type="button" className="gridBackButton" onClick={() => setActiveView("grid")}>
            <ArrowLeft size={14} />
            Back to grid
          </button>
        )}
      </div>

      {activeView === "detail" && detailRow ? (
        <RowDetailView columns={columns} row={detailRow} rowIndex={detailRowIndex!} />
      ) : (
        <div
          ref={scrollRef}
          className="gridScroll"
          onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
        >
          <div className="gridInner" style={{ width: totalWidth }}>
            <div className="gridHeader" style={{ gridTemplateColumns: templateColumns }}>
              {columns.map((column) => (
                <button
                  key={column.name}
                  type="button"
                  className="gridHeaderCell"
                  title={`${column.name} (${column.dataType})`}
                  onClick={() => updateSort(column.name)}
                >
                  <span className="gridHeaderName">{column.name}</span>
                  <span className="gridHeaderType">{column.dataType}</span>
                  <SortIcon activeSort={activeSort} column={column.name} />
                  <span
                    className="resizeHandle"
                    onPointerDown={(event) => startResize(column.name, event)}
                  />
                </button>
              ))}
            </div>

            <div className="gridRows" style={{ height: totalRowsHeight }}>
              {visibleMetrics.map((metric) => (
                <div
                  className="gridRowSlot"
                  key={metric.index}
                  style={{
                    height: metric.height,
                    transform: `translateY(${metric.top}px)`,
                  }}
                >
                  <div
                    className={[
                      "gridRow",
                      wrapCells ? "wrapCells" : "",
                      detailRowIndex === metric.index ? "selected" : "",
                    ]
                      .filter(Boolean)
                      .join(" ")}
                    style={{ gridTemplateColumns: templateColumns, height: metric.height }}
                    onDoubleClick={() => openRowDetail(metric.index)}
                  >
                    {columns.map((column) => {
                      const value = metric.row[column.name];
                      return (
                        <div
                          key={column.name}
                          className={`gridCell ${
                            value === null || value === undefined ? "isNull" : ""
                          }`}
                          title={formatValue(value)}
                        >
                          {formatValue(value)}
                        </div>
                      );
                    })}
                  </div>
                </div>
              ))}
            </div>

            {displayedRows.length === 0 && (
              <div className="gridEmptyOverlay">{loading ? "Loading..." : "No rows"}</div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function SortIcon({ activeSort, column }: { activeSort: SortSpec | null; column: string }) {
  if (activeSort?.column !== column) {
    return <ArrowUpDown size={14} aria-hidden />;
  }
  return activeSort.direction === "asc" ? (
    <ArrowUp size={14} aria-hidden />
  ) : (
    <ArrowDown size={14} aria-hidden />
  );
}

function formatValue(value: unknown): string {
  if (value === null || value === undefined) return "NULL";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

function RowDetailView({
  columns,
  row,
  rowIndex,
}: {
  columns: GridColumn[];
  row: ResultRow;
  rowIndex: number;
}) {
  return (
    <div className="rowDetailView">
      <div className="rowDetailHeader">
        <h3>Row {rowIndex + 1}</h3>
        <span>{columns.length} fields</span>
      </div>
      <div className="rowDetailTable">
        {columns.map((column) => (
          <div className="rowDetailLine" key={column.name}>
            <div className="rowDetailKey">
              <span>{column.name}</span>
              <small>{column.dataType}</small>
            </div>
            <div className="rowDetailValue">
              <DetailValue value={row[column.name]} />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function DetailValue({ value }: { value: unknown }) {
  if (value === null || value === undefined) {
    return <span className="detailNull">NULL</span>;
  }

  const json = parseJsonCandidate(value);
  if (json) {
    return (
      <details className="jsonDetail">
        <summary>{json.label}</summary>
        <pre>{json.formatted}</pre>
      </details>
    );
  }

  const date = parseDateCandidate(value);
  if (date) {
    return (
      <span className="detailDate">
        <span>{date.formatted}</span>
        <code>{date.raw}</code>
      </span>
    );
  }

  return <span>{formatValue(value)}</span>;
}

function parseJsonCandidate(value: unknown): { label: string; formatted: string } | null {
  let parsed = value;

  if (typeof value === "string") {
    const trimmed = value.trim();
    if (!trimmed.startsWith("{") && !trimmed.startsWith("[")) return null;

    try {
      parsed = JSON.parse(trimmed);
    } catch {
      return null;
    }
  }

  if (parsed === null || typeof parsed !== "object") return null;

  const label = Array.isArray(parsed)
    ? `JSON array (${parsed.length})`
    : `JSON object (${Object.keys(parsed).length})`;

  return {
    label,
    formatted: JSON.stringify(parsed, null, 2),
  };
}

function parseDateCandidate(value: unknown): { raw: string; formatted: string } | null {
  if (typeof value !== "string") return null;

  const trimmed = value.trim();
  if (
    !/^\d{4}-\d{2}-\d{2}(?:[T\s]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?)?$/.test(
      trimmed,
    )
  ) {
    return null;
  }

  if (/^\d{4}-\d{2}-\d{2}$/.test(trimmed)) {
    const [year, month, day] = trimmed.split("-").map(Number);
    const date = new Date(year, month - 1, day);
    return {
      raw: trimmed,
      formatted: date.toLocaleDateString(undefined, {
        year: "numeric",
        month: "short",
        day: "numeric",
      }),
    };
  }

  const date = new Date(trimmed);
  if (Number.isNaN(date.getTime())) return null;

  return {
    raw: trimmed,
    formatted: date.toLocaleString(),
  };
}

function estimateWrappedRowHeight(
  row: ResultRow,
  columns: GridColumn[],
  widths: Record<string, number>,
) {
  const maxLines = columns.reduce((currentMax, column) => {
    const text = formatValue(row[column.name]);
    const width = widths[column.name] ?? 160;
    const charsPerLine = Math.max(8, Math.floor((width - 16) / 7));
    const lines = text
      .split("\n")
      .reduce((sum, line) => sum + Math.max(1, Math.ceil(line.length / charsPerLine)), 0);
    return Math.max(currentMax, lines);
  }, 1);

  return Math.min(MAX_WRAP_ROW_HEIGHT, Math.max(MIN_WRAP_ROW_HEIGHT, maxLines * 17 + 16));
}

function compareValues(left: unknown, right: unknown) {
  if (left === right) return 0;
  if (left === null || left === undefined) return -1;
  if (right === null || right === undefined) return 1;

  if (typeof left === "number" && typeof right === "number") {
    return left - right;
  }

  return String(left).localeCompare(String(right), undefined, {
    numeric: true,
    sensitivity: "base",
  });
}

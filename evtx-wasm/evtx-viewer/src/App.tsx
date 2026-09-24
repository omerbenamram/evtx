import { useState, useCallback, useEffect } from "react";
import { styled } from "styled-components";
import { useThemeMode } from "./styles/ThemeModeProvider";
import { GlobalStyles } from "./styles/GlobalStyles";
import {
  Button,
  MenuBar,
  ProgressBar,
  Toolbar,
  ToolbarButton,
  ToolbarSeparator,
} from "./components/Windows";
import { ResizeHandle } from "./components/Windows/ResizeHandle";
import { FileTree } from "./components/FileTree";
import { DragDropOverlay } from "./components/DragDropOverlay";
import { StatusBar } from "./components/StatusBar";
import {
  Open20Regular,
  Save20Regular,
  Filter20Regular,
  ArrowClockwise20Regular,
  ArrowExportLtr20Regular,
  Table20Regular,
} from "@fluentui/react-icons";
import { useFilters } from "./hooks/useFilters";
import { FilterSidebar } from "./components/FilterSidebar/FilterSidebar";
import { LogTableVirtual } from "./components/LogTableVirtual";
import { useEvtxLog } from "./hooks/useEvtxLog";
import { ColumnManager } from "./components/ColumnManager";
import { SearchWorkspace } from "./components/SearchWorkspace";
import { EventTimeline } from "./components/EventTimeline";
import EvtxStorage from "./lib/storage";
import { fetchRecords, getSessionId } from "./lib/duckdb";
import { errorMessage } from "./lib/types";

const PANEL_MIN_WIDTH = 220;
const PANEL_MAX_WIDTH = 400;

const Shell = styled.div`
  display: flex;
  flex-direction: column;
  height: 100dvh;
  background: ${({ theme }) => theme.colors.background.primary};
`;
const Main = styled.div`
  @media (max-width: 700px) {
    > hr {
      display: none;
    }
  }
  display: flex;
  flex: 1;
  min-height: 0;
  overflow: hidden;
`;
const Sidebar = styled.aside<{ $width: number }>`
  width: ${({ $width }) => $width}px;
  flex-shrink: 0;
  border-right: 1px solid ${({ theme }) => theme.colors.border.light};
  @media (max-width: 700px) {
    display: none;
  }
`;
const Records = styled.main`
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  min-height: 0;
`;
const SidePanel = styled.aside<{ $width: number }>`
  width: ${({ $width }) => $width}px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  overflow: auto;
  border-left: 1px solid ${({ theme }) => theme.colors.border.light};
  @media (max-width: 700px) {
    position: absolute;
    right: 0;
    top: 72px;
    bottom: 24px;
    z-index: 10;
    background: ${({ theme }) => theme.colors.background.secondary};
  }
`;
const Notice = styled.div`
  padding: 8px 12px;
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
  background: ${({ theme }) => theme.colors.background.secondary};
  font-size: 13px;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px;
  progress {
    width: 160px;
  }
`;
const Empty = styled.div`
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 14px;
  padding: 32px;
  text-align: center;
  background: ${({ theme }) => theme.colors.background.secondary};
  h1 {
    font-size: 20px;
    font-weight: 600;
  }
  p {
    color: ${({ theme }) => theme.colors.text.secondary};
    max-width: 440px;
    line-height: 1.5;
  }
`;

function download(blob: Blob, name: string) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  // Keep the URL alive until the browser starts consuming the download.
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export default function App() {
  const [selectedNodeId, setSelectedNodeId] = useState("");
  const [showFilters, setShowFilters] = useState(false);
  const [filesWidth, setFilesWidth] = useState(220);
  const [filtersWidth, setFiltersWidth] = useState(280);
  const [columnsWidth, setColumnsWidth] = useState(260);
  const [showColumns, setShowColumns] = useState(false);
  const [treeVersion, setTreeVersion] = useState(0);
  const [actionError, setActionError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const { filters, clearFilters } = useFilters();
  const { mode, toggle } = useThemeMode();
  const {
    isLoading,
    loadingMessage,
    matchedCount,
    totalRecords,
    fileInfo,
    dataSource,
    currentFileId,
    ingestProgress,
    loadError,
    warnings,
    cancelled,
    loadFile,
    cancel,
    reload,
    currentFile,
  } = useEvtxLog();

  const open = useCallback(() => document.getElementById("file-input")?.click(), []);
  const saveOriginal = useCallback(() => {
    const file = currentFile.current;
    if (file) download(file, file.name);
  }, [currentFile]);

  const handleFileSelect = useCallback(
    async (file: File) => {
      clearFilters();
      setActionError(null);
      await loadFile(file);
      setTreeVersion((version) => version + 1);
    },
    [clearFilters, loadFile],
  );

  const openSample = useCallback(async () => {
    try {
      const response = await fetch(`${import.meta.env.BASE_URL}samples/security.evtx`);
      if (!response.ok)
        throw new Error(`Could not open the example log (HTTP ${response.status}).`);
      await handleFileSelect(new File([await response.blob()], "security.evtx"));
    } catch (error) {
      setActionError(errorMessage(error));
    }
  }, [handleFileSelect]);

  const handleNodeSelect = useCallback(
    async (node: { id: string; fileId?: string; logPath?: string }) => {
      setSelectedNodeId(node.id);
      if (node.logPath) {
        await openSample();
        return;
      }
      if (!node.fileId) return;
      try {
        const storage = await EvtxStorage.getInstance();
        const { blob, fileName } = await storage.getFile(node.fileId);
        await handleFileSelect(new File([blob], fileName));
      } catch (error) {
        setActionError(errorMessage(error));
      }
    },
    [handleFileSelect, openSample],
  );

  const exportJson = useCallback(async () => {
    setExporting(true);
    setActionError(null);
    const session = getSessionId();
    try {
      const parts: BlobPart[] = ["[\n"];
      let afterKey = -1;
      let rows;
      do {
        rows = await fetchRecords(filters, 500, afterKey);
        if (session !== getSessionId())
          throw new Error("The open log changed. Export again from the current log.");
        if (rows.length) {
          if (afterKey >= 0) parts.push(",\n");
          parts.push(rows.map(({ record }) => JSON.stringify(record)).join(",\n"));
          afterKey = rows[rows.length - 1].key;
        }
      } while (rows.length === 500);
      parts.push("\n]");
      download(
        new Blob(parts, { type: "application/json" }),
        `${fileInfo?.fileName ?? "events"}.json`,
      );
    } catch (error) {
      setActionError(errorMessage(error));
    } finally {
      setExporting(false);
    }
  }, [filters, fileInfo]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "o") {
        event.preventDefault();
        open();
      }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        saveOriginal();
      }
      if (event.key === "F5" && fileInfo) {
        event.preventDefault();
        void reload();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, saveOriginal, reload, fileInfo]);

  const exportDisabled = !matchedCount || isLoading || exporting;
  const menus = [
    {
      id: "file",
      label: "File",
      submenu: [
        {
          id: "open",
          label: "Open…",
          icon: <Open20Regular />,
          shortcut: "Ctrl/⌘+O",
          onClick: open,
        },
        {
          id: "save",
          label: "Save Original Log As…",
          icon: <Save20Regular />,
          shortcut: "Ctrl/⌘+S",
          disabled: !fileInfo,
          onClick: saveOriginal,
        },
        {
          id: "export",
          label: "Export Matching Events as JSON…",
          disabled: exportDisabled,
          onClick: () => void exportJson(),
        },
      ],
    },
    {
      id: "view",
      label: "View",
      submenu: [
        {
          id: "filters",
          label: showFilters ? "Hide Filters" : "Show Filters",
          onClick: () => setShowFilters((value) => !value),
        },
        {
          id: "columns",
          label: showColumns ? "Hide Columns" : "Manage Columns",
          onClick: () => setShowColumns((value) => !value),
        },
        {
          id: "refresh",
          label: "Reload Log",
          shortcut: "F5",
          disabled: !fileInfo,
          onClick: () => void reload(),
        },
        {
          id: "theme",
          label: mode === "dark" ? "Light Mode" : "Dark Mode",
          onClick: toggle,
        },
      ],
    },
  ];

  return (
    <>
      <GlobalStyles />
      <Shell>
        <MenuBar items={menus} />
        <Toolbar>
          <ToolbarButton icon={<Open20Regular />} title="Open log (Ctrl/⌘+O)" onClick={open}>
            Open
          </ToolbarButton>
          <ToolbarSeparator />
          <ToolbarButton
            icon={<Filter20Regular />}
            title="Show filters"
            isActive={showFilters}
            onClick={() => setShowFilters((value) => !value)}
          >
            Filters
          </ToolbarButton>
          <ToolbarButton
            icon={<Table20Regular />}
            title="Manage columns"
            isActive={showColumns}
            onClick={() => setShowColumns((value) => !value)}
          >
            Columns
          </ToolbarButton>
          <ToolbarSeparator />
          <ToolbarButton
            icon={<ArrowClockwise20Regular />}
            title="Reload log (F5)"
            disabled={!fileInfo}
            onClick={() => void reload()}
          />
          <ToolbarButton
            icon={<ArrowExportLtr20Regular />}
            title="Export matching events as JSON"
            disabled={exportDisabled}
            onClick={() => void exportJson()}
          >
            {exporting ? "Exporting…" : "Export"}
          </ToolbarButton>
        </Toolbar>
        {isLoading && (
          <Notice as="output">
            <span>
              {loadingMessage} {totalRecords.toLocaleString()} events available
            </span>
            <ProgressBar value={ingestProgress} />
            <Button size="small" onClick={cancel} disabled={ingestProgress >= 1}>
              Cancel import
            </Button>
          </Notice>
        )}
        {(loadError || actionError) && (
          <Notice role="alert">
            <span>{actionError ?? loadError}</span>
            <Button
              size="small"
              onClick={() => {
                setActionError(null);
                void reload();
              }}
            >
              Retry
            </Button>
            <Button size="small" onClick={open}>
              Open another log
            </Button>
          </Notice>
        )}
        {cancelled && (
          <Notice as="output">
            Import cancelled. Showing the events loaded so far.
            <Button size="small" onClick={() => void reload()}>
              Restart import
            </Button>
          </Notice>
        )}
        {warnings.length > 0 && (
          <Notice>
            <details>
              <summary>
                {warnings.length > 100 ? "More than 100" : warnings.length} import warning
                {warnings.length === 1 ? "" : "s"} — some events may be missing
              </summary>
              <ul>
                {warnings.map((warning) => (
                  <li key={warning}>{warning}</li>
                ))}
              </ul>
            </details>
          </Notice>
        )}
        <Main>
          <Sidebar $width={filesWidth}>
            <FileTree
              onNodeSelect={handleNodeSelect}
              selectedNodeId={selectedNodeId}
              activeFileId={currentFileId}
              ingestProgress={isLoading ? ingestProgress : 1}
              refreshVersion={treeVersion}
            />
          </Sidebar>
          <ResizeHandle
            label="Resize log sidebar"
            value={filesWidth}
            min={160}
            max={400}
            orientation="vertical"
            onResize={setFilesWidth}
          />
          <Records>
            <SearchWorkspace disabled={!dataSource} />
            {dataSource ? (
              <>
                <EventTimeline disabled={isLoading} fileId={currentFileId} />
                <LogTableVirtual
                  key={currentFileId ?? "no-file"}
                  dataSource={dataSource}
                  onManageColumns={() => setShowColumns(true)}
                />
              </>
            ) : (
              <Empty>
                <h1>Open a Windows event log</h1>
                <p>
                  Drop an .evtx file here to search and inspect its events. Your log is processed
                  locally in this browser.
                </p>
                <div style={{ display: "flex", gap: 8 }}>
                  <Button size="small" onClick={open}>
                    Open log…
                  </Button>
                  <Button size="small" onClick={() => void openSample()}>
                    Try example log
                  </Button>
                </div>
              </Empty>
            )}
          </Records>
          {showFilters && (
            <>
              <ResizeHandle
                label="Resize filters panel"
                value={filtersWidth}
                min={PANEL_MIN_WIDTH}
                max={PANEL_MAX_WIDTH}
                orientation="vertical"
                reverse
                onResize={setFiltersWidth}
              />
              <SidePanel aria-label="Event filters" $width={filtersWidth}>
                <FilterSidebar />
              </SidePanel>
            </>
          )}
          {showColumns && (
            <>
              <ResizeHandle
                label="Resize columns panel"
                value={columnsWidth}
                min={PANEL_MIN_WIDTH}
                max={PANEL_MAX_WIDTH}
                orientation="vertical"
                reverse
                onResize={setColumnsWidth}
              />
              <SidePanel aria-label="Table columns" $width={columnsWidth}>
                <ColumnManager onClose={() => setShowColumns(false)} />
              </SidePanel>
            </>
          )}
        </Main>
        <StatusBar />
        <DragDropOverlay onFileSelect={handleFileSelect} />
      </Shell>
    </>
  );
}

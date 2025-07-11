import React, { useState, useCallback, useEffect } from "react";
/* eslint-disable @typescript-eslint/no-explicit-any */
import styled, { useTheme } from "styled-components";
import { useThemeMode } from "./styles/ThemeModeProvider";
import { GlobalStyles } from "./styles/GlobalStyles";
import {
  MenuBar,
  ProgressBar,
  Panel,
  Toolbar,
  ToolbarButton,
  ToolbarSeparator,
  Dropdown,
} from "./components/Windows";
import { FileTree } from "./components/FileTree";
import { DragDropOverlay } from "./components/DragDropOverlay";
import { StatusBar as StatusBarView } from "./components/StatusBar";
import {
  Open20Regular,
  Save20Regular,
  Print20Regular,
  Filter20Regular,
  ArrowClockwise20Regular,
  Info20Regular,
  ArrowExportLtr20Regular,
} from "@fluentui/react-icons";
// Note: parsing/export handled via useEvtxLog hook; no direct EvtxParser needed here.
import { useFilters } from "./hooks/useFilters";
import { useColumns } from "./hooks/useColumns";
import { logger, LogLevel } from "./lib/logger";
import init from "./wasm/evtx_wasm.js";
import { FilterSidebar } from "./components/FilterSidebar";
import { LogTableVirtual } from "./components/LogTableVirtual";
import { useEvtxLog } from "./hooks/useEvtxLog";
import { initDuckDB } from "./lib/duckdb";
import { ColumnManager } from "./components/ColumnManager";
import { Table20Regular as TableIcon } from "@fluentui/react-icons";

const AppContainer = styled.div`
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: ${({ theme }) => theme.colors.background.primary};
`;

const MainContent = styled.div`
  display: flex;
  flex: 1;
  overflow: hidden;
`;

const Sidebar = styled.aside`
  width: 280px;
  min-width: 200px;
  max-width: 400px;
  border-right: 1px solid ${({ theme }) => theme.colors.border.light};
  display: flex;
  flex-direction: column;
`;

const ContentArea = styled.main`
  flex: 1;
  display: flex;
  flex-direction: row;
  overflow: hidden;
`;

const RecordsArea = styled.div`
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
`;

const FilterPanel = styled.aside<{ $width: number }>`
  width: ${({ $width }) => $width}px;
  min-width: 220px;
  max-width: 400px;
  display: flex;
  flex-direction: column;
  position: relative;
`;

const ColumnPanel = styled.aside<{ $width: number }>`
  width: ${({ $width }) => $width}px;
  min-width: 220px;
  max-width: 360px;
  display: flex;
  flex-direction: column;
  position: relative;
`;

const ColumnDivider = styled.div`
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
  cursor: col-resize;
  background: ${({ theme }) => theme.colors.border.light};
  transition: background 0.2s ease;

  &:hover {
    background: ${({ theme }) => theme.colors.accent.primary};
  }
`;

const FilterDivider = styled.div`
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
  cursor: col-resize;
  background: ${({ theme }) => theme.colors.border.light};
  transition: background 0.2s ease;

  &:hover {
    background: ${({ theme }) => theme.colors.accent.primary};
  }
`;

// Local StatusBar styled components have been moved to components/StatusBar.tsx

const LoadingOverlay = styled.div`
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(255, 255, 255, 0.8);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1001;
`;

const LoadingContent = styled.div`
  background: ${({ theme }) => theme.colors.background.secondary};
  padding: ${({ theme }) => theme.spacing.xl};
  border-radius: ${({ theme }) => theme.borderRadius.md};
  box-shadow: ${({ theme }) => theme.shadows.elevation};
  text-align: center;
`;

function App() {
  const [selectedNodeId, setSelectedNodeId] = useState<string>("");
  const [isWasmReady, setIsWasmReady] = useState(false);
  const [isDuckDbReady, setIsDuckDbReady] = useState(false);
  const { filters, clearFilters } = useFilters();
  const [showFilters, setShowFilters] = useState(false);
  // Table column state – start with defaults
  const { columns, setColumns } = useColumns();
  const [showColumnMgr, setShowColumnMgr] = useState(false);
  const [filterPanelWidth, setFilterPanelWidth] = useState(300);
  const [fileTreeVersion, setFileTreeVersion] = useState<number>(0);
  const [assetProgress, setAssetProgress] = useState(0);

  const {
    isLoading,
    loadingMessage,
    records,
    matchedCount,
    fileInfo,
    parser,
    dataSource,
    currentFileId,
    ingestProgress,
    loadFile,
  } = useEvtxLog();

  // --- Logging level state ---
  const [logLevel, setLogLevel] = useState<LogLevel>(logger.getLogLevel());

  const handleLogLevelChange = useCallback((level: LogLevel) => {
    logger.setLogLevel(level);
    setLogLevel(level);
  }, []);

  const logLevelOptions = [
    { label: "DEBUG", value: LogLevel.DEBUG },
    { label: "INFO", value: LogLevel.INFO },
    { label: "WARN", value: LogLevel.WARN },
    { label: "ERROR", value: LogLevel.ERROR },
  ];

  // Initialize WASM module
  useEffect(() => {
    const initEngines = async () => {
      try {
        logger.info("Initializing EVTX parser WASM module...");
        setAssetProgress(0.1);
        await init();
        setAssetProgress(0.4);
        setIsWasmReady(true);
        logger.info("EVTX parser WASM module initialized");

        // Now initialise DuckDB WASM.  This step can take several seconds on
        // first load because the browser has to download & compile the DB
        // assets.  We await it so that downstream code relying on the DB can
        // safely proceed and so we can surface meaningful UI feedback.
        logger.info("Initializing DuckDB WASM engine...");
        setAssetProgress(0.5);
        await initDuckDB();
        setAssetProgress(1);
        setIsDuckDbReady(true);
        logger.info("DuckDB WASM engine ready");
      } catch (error) {
        logger.error("Failed to initialise WASM engines", error);
      }
    };

    void initEngines();
  }, []);

  // Handle dragging of the filter panel divider
  const handleFilterDividerMouseDown = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      e.preventDefault();

      const startX = e.clientX;
      const startWidth = filterPanelWidth;

      const onMouseMove = (moveEvent: MouseEvent) => {
        const deltaX = startX - moveEvent.clientX;
        const newWidth = Math.max(220, Math.min(400, startWidth + deltaX));
        setFilterPanelWidth(newWidth);
      };

      const onMouseUp = () => {
        document.removeEventListener("mousemove", onMouseMove);
        document.removeEventListener("mouseup", onMouseUp);
      };

      document.addEventListener("mousemove", onMouseMove);
      document.addEventListener("mouseup", onMouseUp);
    },
    [filterPanelWidth]
  );

  // Wrapper to gate WASM readiness and reset some App-level state before delegating
  const handleFileSelect = useCallback(
    async (file: File) => {
      if (!isWasmReady) {
        alert("WASM module is still loading. Please try again.");
        return;
      }

      // Reset filters in App scope on new file
      clearFilters();

      // Ingest the file (this will persist it to IndexedDB via parser)
      await loadFile(file);

      // Refresh FileTree *after* the file is saved so it appears immediately
      setFileTreeVersion((v) => v + 1);
    },
    [isWasmReady, loadFile, clearFilters]
  );

  type TreeNodeData = { id: string; fileId?: string; logPath?: string };

  const handleNodeSelect = useCallback(
    async (node: TreeNodeData) => {
      setSelectedNodeId(node.id);
      logger.debug("Tree node selected", node);

      if (node.fileId) {
        try {
          const storage = await (
            await import("./lib/storage")
          ).default.getInstance();
          const { meta, blob } = await storage.getFile(node.fileId);
          // Convert Blob to File so existing parser flow works
          const file = new File([blob], meta.fileName, {
            type: "application/octet-stream",
          });
          await handleFileSelect(file);
        } catch (err) {
          logger.error("Failed to load cached file", err);
          alert("Could not load cached log – see console for details");
        }
        return;
      }

      // Handle built-in sample logs (lazy-loaded from /samples)
      if (node.logPath) {
        try {
          const base = (import.meta.env.BASE_URL || "/").replace(/\/$/, "");
          const sampleUrl = `${base}/${node.logPath}`;
          logger.info(`Fetching built-in sample log: ${sampleUrl}`);
          const res = await fetch(sampleUrl);
          if (!res.ok) throw new Error(`HTTP ${res.status}`);

          const blob = await res.blob();
          const fileName = node.logPath.split("/").pop() ?? "sample.evtx";
          const file = new File([blob], fileName, {
            type: "application/octet-stream",
          });

          await handleFileSelect(file);
        } catch (err) {
          logger.error("Failed to load bundled sample", err);
          alert("Could not load bundled sample log. See console for details.");
        }
      }
    },
    [handleFileSelect]
  );

  const handleRefresh = useCallback(async () => {
    if (!parser || !fileInfo) return;

    // Refresh parsing via parser (Hook state will capture changes if needed)
    try {
      const result = await parser.parseAllRecords();
      // Currently the hook owns records; we can't set them directly here.
      // For now we just log and trust DuckDB source; we may expand hook later.
      logger.info("Records refreshed", { count: result.records.length });
    } catch (error) {
      logger.error("Failed to refresh records", error);
    }
  }, [parser, fileInfo]);

  // (Effects computing matched count, bucket counts, and dataSource moved into useEvtxLog)

  const handleExport = useCallback(
    async (format: "json" | "xml") => {
      if (matchedCount === 0) return;

      try {
        const { fetchRecords } = await import("./lib/duckdb");
        const dataArr = await fetchRecords(filters, matchedCount, 0);
        const data =
          parser?.exportRecords(dataArr, format) ||
          JSON.stringify(dataArr, null, 2);
        const blob = new Blob([data], {
          type: format === "json" ? "application/json" : "application/xml",
        });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = `evtx_export_${new Date().toISOString()}.${format}`;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);

        logger.info(
          `Exported ${matchedCount} records as ${format.toUpperCase()}`
        );
      } catch (error) {
        logger.error(`Failed to export as ${format}`, error);
        alert(
          `Failed to export as ${format}. ${
            error instanceof Error ? error.message : ""
          }`
        );
      }
    },
    [parser, matchedCount, filters]
  );

  const { mode: themeMode, toggle: toggleTheme } = useThemeMode();

  const menuItems = [
    {
      id: "file",
      label: "File",
      submenu: [
        {
          id: "file-open",
          label: "Open...",
          icon: <Open20Regular />,
          shortcut: "Ctrl+O",
          onClick: () => {
            document.getElementById("file-input")?.click();
          },
        },
        {
          id: "file-save-as",
          label: "Save Log File As...",
          icon: <Save20Regular />,
          shortcut: "Ctrl+S",
          disabled: records.length === 0,
        },
        { id: "file-sep-1", label: "sep", separator: true },
        {
          id: "file-export",
          label: "Export",
          submenu: [
            {
              id: "file-export-json",
              label: "Export as JSON...",
              onClick: () => handleExport("json"),
              disabled: records.length === 0,
            },
            {
              id: "file-export-xml",
              label: "Export as XML...",
              onClick: () => handleExport("xml"),
              disabled: records.length === 0,
            },
          ],
        },
        { id: "file-sep-2", label: "sep", separator: true },
        {
          id: "file-print",
          label: "Print...",
          icon: <Print20Regular />,
          shortcut: "Ctrl+P",
          disabled: true,
        },
        {
          id: "file-exit",
          label: "Exit",
          shortcut: "Alt+F4",
          onClick: () => window.close(),
        },
      ],
    },
    {
      id: "view",
      label: "View",
      submenu: [
        {
          id: "view-filter",
          label: showFilters ? "Hide Filters" : "Filter Current Log",
          icon: <Filter20Regular />,
          disabled: records.length === 0 || ingestProgress < 1,
          onClick: () => setShowFilters((prev) => !prev),
        },
        {
          id: "view-columns",
          label: showColumnMgr ? "Hide Columns" : "Manage Columns",
          icon: <TableIcon />,
          disabled: !dataSource,
          onClick: () => setShowColumnMgr((p) => !p),
        },
        { id: "view-sep-1", label: "sep", separator: true },
        {
          id: "view-refresh",
          label: "Refresh",
          icon: <ArrowClockwise20Regular />,
          shortcut: "F5",
          onClick: handleRefresh,
        },
        {
          id: "view-dark-mode",
          label:
            themeMode === "dark"
              ? "Switch to Light Mode"
              : "Switch to Dark Mode",
          onClick: toggleTheme,
        },
      ],
    },
    {
      id: "help",
      label: "Help",
      submenu: [
        {
          id: "help-about",
          label: "About EVTX Viewer",
          icon: <Info20Regular />,
          onClick: () => {
            alert(
              "EVTX Viewer v1.0.0\nA Windows Event Log viewer built with React and WebAssembly"
            );
          },
        },
      ],
    },
  ];

  const currentTheme = useTheme();

  // Determine which progress value to display in the global loading overlay.
  // While the core engines are still loading we show assetProgress.
  // Once they are ready but a file is being ingested we show ingestProgress.
  const overlayProgress = !isDuckDbReady
    ? assetProgress
    : isLoading
    ? ingestProgress
    : undefined;

  return (
    <>
      <GlobalStyles />
      <AppContainer>
        <MenuBar items={menuItems} />

        <Panel
          elevation="flat"
          padding="none"
          style={{
            background: currentTheme.colors.background.tertiary,
            border: "none",
            borderRadius: 0,
          }}
        >
          <Toolbar>
            <ToolbarButton
              icon={<Open20Regular />}
              title="Open"
              onClick={() => document.getElementById("file-input")?.click()}
            />
            <ToolbarSeparator />
            <ToolbarButton
              icon={<Filter20Regular />}
              title="Filter"
              isActive={showFilters}
              disabled={records.length === 0 || ingestProgress < 1}
              onClick={() => setShowFilters((prev) => !prev)}
            />
            <ToolbarButton
              icon={<TableIcon />}
              title="Columns"
              isActive={showColumnMgr}
              disabled={!dataSource}
              onClick={() => setShowColumnMgr((p) => !p)}
            />
            <ToolbarSeparator />
            <ToolbarButton
              icon={<ArrowClockwise20Regular />}
              title="Refresh"
              onClick={handleRefresh}
              disabled={!parser}
            />
            <ToolbarSeparator />
            <ToolbarButton
              icon={<ArrowExportLtr20Regular />}
              title="Export"
              disabled={matchedCount === 0}
              onClick={() => handleExport("json")}
            />
            <ToolbarSeparator />
            <Dropdown
              label="Log"
              value={logLevel}
              onChange={handleLogLevelChange}
              options={logLevelOptions}
            />
          </Toolbar>
        </Panel>

        <MainContent>
          <Sidebar>
            <FileTree
              onNodeSelect={handleNodeSelect}
              selectedNodeId={selectedNodeId}
              activeFileId={currentFileId}
              ingestProgress={ingestProgress}
              refreshVersion={fileTreeVersion}
            />
          </Sidebar>
          <ContentArea>
            <RecordsArea>
              {dataSource ? (
                <LogTableVirtual
                  key={currentFileId ?? "no-file"}
                  dataSource={dataSource}
                />
              ) : (
                <div style={{ padding: 16 }}>No data source</div>
              )}
            </RecordsArea>
            {showFilters && ingestProgress === 1 && (
              <FilterPanel $width={filterPanelWidth}>
                <FilterDivider onMouseDown={handleFilterDividerMouseDown} />
                <FilterSidebar />
              </FilterPanel>
            )}

            {showColumnMgr && (
              <ColumnPanel $width={260}>
                <ColumnDivider onMouseDown={(e) => e.preventDefault()} />
                <ColumnManager
                  allColumns={columns /* TODO: extend list */}
                  active={columns}
                  onChange={setColumns}
                  onClose={() => setShowColumnMgr(false)}
                />
              </ColumnPanel>
            )}
          </ContentArea>
        </MainContent>

        <StatusBarView
          isWasmReady={isWasmReady}
          isDuckDbReady={isDuckDbReady}
        />

        <DragDropOverlay onFileSelect={handleFileSelect} />

        {/* Global loading overlay – show either during file ingest or while core WASM/DB engines are loading. */}
        {(isLoading || !isDuckDbReady) && (
          <LoadingOverlay>
            <LoadingContent>
              <h3>Loading...</h3>
              <ProgressBar value={overlayProgress} />
              <p>
                {isLoading
                  ? loadingMessage
                  : "Downloading & compiling WASM assets..."}
              </p>
            </LoadingContent>
          </LoadingOverlay>
        )}

        {/* standalone overlay removed – ColumnPanel handles sidebar */}
      </AppContainer>
    </>
  );
}

export default App;

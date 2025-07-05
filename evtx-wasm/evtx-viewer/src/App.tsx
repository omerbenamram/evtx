import React, { useState, useCallback, useEffect, useMemo } from "react";
/* eslint-disable @typescript-eslint/no-explicit-any */
import styled, { ThemeProvider } from "styled-components";
import { GlobalStyles } from "./styles/GlobalStyles";
import { theme } from "./styles/theme";
import {
  MenuBar,
  Panel,
  Toolbar,
  ToolbarButton,
  ToolbarSeparator,
  Dropdown,
} from "./components/Windows";
import { FileTree } from "./components/FileTree";
import { LogTable } from "./components/LogTable";
import { DragDropOverlay } from "./components/DragDropOverlay";
import {
  Open20Regular,
  Save20Regular,
  Print20Regular,
  Filter20Regular,
  ArrowClockwise20Regular,
  Info20Regular,
  ArrowExportLtr20Regular,
} from "@fluentui/react-icons";
import { EvtxParser } from "./lib/parser";
import {
  type EvtxRecord,
  type EvtxFileInfo,
  type FilterOptions,
  type BucketCounts,
} from "./lib/types";
import { logger, LogLevel } from "./lib/logger";
import EvtxStorage from "./lib/storage";
import init from "./wasm/evtx_wasm.js";
import { FilterSidebar } from "./components/FilterSidebar";
import { LazyEvtxReader } from "./lib/lazyReader";
import { ChunkDataSource } from "./lib/chunkDataSource";
import { LogTableVirtual } from "./components/LogTableVirtual";

const AppContainer = styled.div`
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: ${theme.colors.background.primary};
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
  border-right: 1px solid ${theme.colors.border.light};
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

const FilterDivider = styled.div`
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
  cursor: col-resize;
  background: ${theme.colors.border.light};
  transition: background 0.2s ease;

  &:hover {
    background: ${theme.colors.accent.primary};
  }
`;

const StatusBar = styled.div`
  height: 24px;
  background: ${theme.colors.background.secondary};
  border-top: 1px solid ${theme.colors.border.light};
  display: flex;
  align-items: center;
  padding: 0 ${theme.spacing.sm};
  font-size: ${theme.fontSize.caption};
  color: ${theme.colors.text.secondary};
  gap: ${theme.spacing.lg};
`;

const StatusItem = styled.span`
  display: flex;
  align-items: center;
  gap: ${theme.spacing.xs};
`;

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
  background: ${theme.colors.background.secondary};
  padding: ${theme.spacing.xl};
  border-radius: ${theme.borderRadius.md};
  box-shadow: ${theme.shadows.elevation};
  text-align: center;
`;

function App() {
  const [isLoading, setIsLoading] = useState(false);
  const [loadingMessage, setLoadingMessage] = useState("");
  const [records, setRecords] = useState<EvtxRecord[]>([]);
  const [fileInfo, setFileInfo] = useState<EvtxFileInfo | null>(null);
  const [parser, setParser] = useState<EvtxParser | null>(null);
  const [selectedNodeId, setSelectedNodeId] = useState<string>("");
  const [isWasmReady, setIsWasmReady] = useState(false);
  const [filters, setFilters] = useState<FilterOptions>({});
  const [showFilters, setShowFilters] = useState(false);
  const [filterPanelWidth, setFilterPanelWidth] = useState(300);
  const [dataSource, setDataSource] = useState<ChunkDataSource | null>(null);
  const [bucketCounts, setBucketCounts] = useState<BucketCounts | null>(null);
  const [currentFile, setCurrentFile] = useState<File | null>(null);
  const [currentFileId, setCurrentFileId] = useState<string | null>(null);

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
    const initWasm = async () => {
      try {
        logger.info("Initializing WASM module...");
        await init();
        setIsWasmReady(true);
        logger.info("WASM module initialized successfully");
      } catch (error) {
        logger.error("Failed to initialize WASM module", error);
      }
    };
    initWasm();
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

  const handleFileSelect = useCallback(
    async (file: File) => {
      if (!isWasmReady) {
        alert("WASM module is still loading. Please try again.");
        return;
      }

      setIsLoading(true);
      setLoadingMessage("Loading file...");
      logger.info(`Loading file: ${file.name}`);

      try {
        setCurrentFile(file);
        setFilters({}); // reset any active filters
        // ----------------- NEW LAZY PATH -----------------
        const reader = await LazyEvtxReader.fromFile(file);
        const ds = new ChunkDataSource(reader);
        setDataSource(ds);

        // We still parse the first window eagerly so that filters/sidebar can
        // display something immediate (optional – small performance hit).
        const initial = await reader.getWindow({
          chunkIndex: 0,
          start: 0,
          limit: 1000,
        });
        setRecords(initial);

        // TODO: update filters to stream, for now they only work on loaded slice.

        // Legacy EvtxParser kept for export functionality.
        const evtxParser = new EvtxParser();
        const info = await evtxParser.parseFile(file);
        // Retrieve fileId derived during saveFile inside parser
        const storage = await EvtxStorage.getInstance();
        const fileId = await storage.deriveFileId(file);
        setCurrentFileId(fileId);

        const cachedBuckets = await storage.getBucketCounts(fileId);
        setBucketCounts(cachedBuckets ?? null);
        setFileInfo(info);
        setParser(evtxParser);
      } catch (error) {
        logger.error("Failed to load file via lazy reader", error);
        alert("Failed to parse file. Please check if it's a valid EVTX file.");
      } finally {
        setIsLoading(false);
        setLoadingMessage("");
      }
    },
    [isWasmReady]
  );

  // Handler to compute full-file bucket counts via WASM
  const handleComputeBuckets = useCallback(async () => {
    if (!currentFile || !currentFileId) return;
    try {
      setIsLoading(true);
      setLoadingMessage("Computing filter buckets (full file scan)...");

      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const wasmMod: any = await import("./wasm/evtx_wasm.js");
      const buffer = await currentFile.arrayBuffer();
      const raw = await wasmMod.compute_buckets(new Uint8Array(buffer));

      // Convert potential Map structures to plain objects
      // Helper to recursively convert Map → plain object
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const mapToObj = (input: unknown): any => {
        if (input instanceof Map) {
          const o: Record<string, any> = {};
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          (input as Map<any, any>).forEach((v: any, k: any) => {
            o[String(k)] = mapToObj(v);
          });
          return o;
        }
        if (Array.isArray(input)) return input.map(mapToObj);
        return input;
      };

      const buckets: BucketCounts = mapToObj(raw) as BucketCounts;
      setBucketCounts(buckets);

      // Persist in IndexedDB
      const storage = await EvtxStorage.getInstance();
      await storage.saveBucketCounts(currentFileId, buckets);

      logger.info("Computed and cached bucket counts", {
        levels: Object.keys(buckets.level).length,
        providers: Object.keys(buckets.provider).length,
      });
    } catch (err) {
      logger.error("Failed to compute buckets", err);
      alert("Failed to compute filter buckets – see console for details");
    } finally {
      setIsLoading(false);
      setLoadingMessage("");
    }
  }, [currentFile, currentFileId]);

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

      // legacy demo paths
      if (node.logPath) {
        logger.info(`Would load log: ${node.logPath}`);
      }
    },
    [handleFileSelect]
  );

  const handleRefresh = useCallback(async () => {
    if (!parser || !fileInfo) return;

    setIsLoading(true);
    setLoadingMessage("Refreshing records...");

    try {
      const result = await parser.parseAllRecords();
      setRecords(result.records);
      logger.info("Records refreshed", { count: result.records.length });
    } catch (error) {
      logger.error("Failed to refresh records", error);
    } finally {
      setIsLoading(false);
    }
  }, [parser, fileInfo]);

  // Helper – apply current filters to full record list
  const applyFilters = useCallback(
    (allRecords: EvtxRecord[], opts: FilterOptions): EvtxRecord[] => {
      const getProvider = (sys: any): string =>
        sys.Provider?.Name ?? sys.Provider_attributes?.Name ?? "";
      if (
        !opts.searchTerm &&
        (!opts.level || opts.level.length === 0) &&
        (!opts.provider || opts.provider.length === 0) &&
        (!opts.channel || opts.channel.length === 0) &&
        (!opts.eventId || opts.eventId.length === 0)
      ) {
        return allRecords;
      }

      const term = (opts.searchTerm ?? "").toLowerCase();

      return allRecords.filter((rec) => {
        const sys = rec.Event?.System ?? {};
        const levelMatch =
          !opts.level || opts.level.length === 0
            ? true
            : opts.level.includes(sys.Level ?? 4);

        const providerName = getProvider(sys);
        const providerMatch =
          !opts.provider || opts.provider.length === 0
            ? true
            : opts.provider.includes(providerName);

        const channelMatch =
          !opts.channel || opts.channel.length === 0
            ? true
            : opts.channel.includes(sys.Channel ?? "");

        const eventIdMatch =
          !opts.eventId || opts.eventId.length === 0
            ? true
            : opts.eventId.includes(Number(sys.EventID ?? -1));

        const searchStr = `${providerName} ${sys.Computer ?? ""} ${
          sys.EventID ?? ""
        }`.toLowerCase();
        const termMatch = term === "" || searchStr.includes(term);

        return (
          levelMatch &&
          providerMatch &&
          channelMatch &&
          eventIdMatch &&
          termMatch
        );
      });
    },
    []
  );

  const filteredRecords = useMemo(
    () => applyFilters(records, filters),
    [records, filters, applyFilters]
  );

  const handleExport = useCallback(
    async (format: "json" | "xml") => {
      if (filteredRecords.length === 0) return;

      try {
        const data =
          parser?.exportRecords(filteredRecords, format) ||
          JSON.stringify(filteredRecords, null, 2);
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
          `Exported ${
            filteredRecords.length
          } records as ${format.toUpperCase()}`
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
    [filteredRecords, parser]
  );

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
          disabled: records.length === 0,
          onClick: () => setShowFilters((prev) => !prev),
        },
        { id: "view-sep-1", label: "sep", separator: true },
        {
          id: "view-refresh",
          label: "Refresh",
          icon: <ArrowClockwise20Regular />,
          shortcut: "F5",
          onClick: handleRefresh,
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

  return (
    <ThemeProvider theme={theme}>
      <GlobalStyles />
      <AppContainer>
        <MenuBar items={menuItems} />

        <Panel
          elevation="flat"
          padding="none"
          style={{
            background: theme.colors.background.tertiary,
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
              disabled={records.length === 0}
              onClick={() => setShowFilters((prev) => !prev)}
            />
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
              disabled={filteredRecords.length === 0}
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
            />
          </Sidebar>
          <ContentArea>
            <RecordsArea>
              {dataSource ? (
                <LogTableVirtual dataSource={dataSource} filters={filters} />
              ) : (
                <LogTable data={filteredRecords} />
              )}
            </RecordsArea>
            {showFilters && (
              <FilterPanel $width={filterPanelWidth}>
                <FilterDivider onMouseDown={handleFilterDividerMouseDown} />
                <FilterSidebar
                  records={records}
                  filters={filters}
                  bucketCounts={bucketCounts}
                  onChange={setFilters}
                />
                {!bucketCounts && currentFile && (
                  <Panel
                    padding="small"
                    style={{
                      borderTop: `1px solid ${theme.colors.border.light}`,
                    }}
                  >
                    <button onClick={handleComputeBuckets}>
                      Compute full counts
                    </button>
                  </Panel>
                )}
              </FilterPanel>
            )}
          </ContentArea>
        </MainContent>

        <StatusBar>
          <StatusItem>
            {fileInfo
              ? `${fileInfo.fileName} - ${filteredRecords.length}/${records.length} events`
              : "No file loaded"}
          </StatusItem>
          <StatusItem>
            {fileInfo && `Chunks: ${fileInfo.totalChunks}`}
          </StatusItem>
          <StatusItem>{isWasmReady ? "Ready" : "Loading WASM..."}</StatusItem>
        </StatusBar>

        <DragDropOverlay onFileSelect={handleFileSelect} />

        {isLoading && (
          <LoadingOverlay>
            <LoadingContent>
              <h3>Loading...</h3>
              <p>{loadingMessage}</p>
            </LoadingContent>
          </LoadingOverlay>
        )}
      </AppContainer>
    </ThemeProvider>
  );
}

export default App;

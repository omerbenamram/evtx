import React, { useState, useCallback, useEffect, useMemo } from "react";
import styled, { ThemeProvider } from "styled-components";
import { GlobalStyles } from "./styles/GlobalStyles";
import { theme } from "./styles/theme";
import {
  MenuBar,
  Panel,
  Toolbar,
  ToolbarButton,
  ToolbarSeparator,
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
  Settings20Regular,
  ArrowExportLtr20Regular,
} from "@fluentui/react-icons";
import { EvtxParser } from "./lib/parser";
import {
  type EvtxRecord,
  type EvtxFileInfo,
  type FilterOptions,
} from "./lib/types";
import { logger } from "./lib/logger";
import init from "./wasm/evtx_wasm.js";
import { FilterSidebar } from "./components/FilterSidebar";

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
        const evtxParser = new EvtxParser();
        const info = await evtxParser.parseFile(file);
        setFileInfo(info);
        setParser(evtxParser);

        logger.info("File loaded successfully", info);
        setLoadingMessage("Parsing records...");

        // Parse up to 1000 records initially
        const result = await evtxParser.parseWithLimit(1000);
        setRecords(result.records);

        // Output first record structure for debugging
        if (result.records.length > 0) {
          logger.info("Sample record structure (logger)", result.records[0]);
          // eslint-disable-next-line no-console
          console.log("Sample record structure", result.records[0]);
        }

        logger.info(`Parsed ${result.records.length} records`, {
          totalRecords: result.totalRecords,
          errors: result.errors.length,
        });

        if (result.errors.length > 0) {
          logger.warn("Some records had parsing errors", result.errors);
        }
      } catch (error) {
        logger.error("Failed to parse file", error);
        alert("Failed to parse file. Please check if it's a valid EVTX file.");
      } finally {
        setIsLoading(false);
        setLoadingMessage("");
      }
    },
    [isWasmReady]
  );

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const handleNodeSelect = useCallback((node: any) => {
    setSelectedNodeId(node.id);
    logger.debug("Tree node selected", node);

    // In a real app, this would filter records based on the selected log
    if (node.logPath) {
      logger.info(`Would load log: ${node.logPath}`);
    }
  }, []);

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

        const providerMatch =
          !opts.provider || opts.provider.length === 0
            ? true
            : opts.provider.includes(sys.Provider?.Name ?? "");

        const channelMatch =
          !opts.channel || opts.channel.length === 0
            ? true
            : opts.channel.includes(sys.Channel ?? "");

        const eventIdMatch =
          !opts.eventId || opts.eventId.length === 0
            ? true
            : opts.eventId.includes(Number(sys.EventID ?? -1));

        const searchStr = `${sys.Provider?.Name ?? ""} ${sys.Computer ?? ""} ${
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
            <ToolbarButton
              icon={<Save20Regular />}
              title="Save"
              disabled={records.length === 0}
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
            <ToolbarButton icon={<Settings20Regular />} title="Settings" />
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
              <LogTable data={filteredRecords} />
            </RecordsArea>
            {showFilters && (
              <FilterPanel $width={filterPanelWidth}>
                <FilterDivider onMouseDown={handleFilterDividerMouseDown} />
                <FilterSidebar
                  records={records}
                  filters={filters}
                  onChange={setFilters}
                />
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

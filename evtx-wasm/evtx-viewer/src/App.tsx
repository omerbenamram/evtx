import { useState, useCallback, useEffect } from "react";
import { styled } from "styled-components";
import { useThemeMode } from "./styles/ThemeModeProvider";
import { GlobalStyles } from "./styles/GlobalStyles";
import { Button, MenuBar, Toolbar, ToolbarButton, ToolbarSeparator } from "./components/Windows";
import { ResizeHandle } from "./components/Windows/ResizeHandle";
import { FileTree } from "./components/FileTree";
import { DragDropOverlay } from "./components/DragDropOverlay";
import { StatusBar } from "./components/StatusBar";
import {
  Open16Regular,
  Save16Regular,
  Filter16Regular,
  ArrowClockwise16Regular,
  ArrowExportLtr16Regular,
  Table16Regular,
  PanelLeft16Regular,
} from "@fluentui/react-icons";
import { setTimeZone, useTimeZone } from "./lib/timeZone";
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
// Below the supported 1024px width the side panes float over the grid instead of docking.
const NARROW = "(max-width: 1023px)";

const Shell = styled.div`
  display: flex;
  flex-direction: column;
  height: 100dvh;
  overflow: hidden;
  background: ${({ theme }) => theme.colors.surface.base};
`;
const Bar = styled(Toolbar)`
  flex-wrap: nowrap;
  overflow: hidden;
`;
// Toolbar labels collapse to icon-only buttons on narrow windows.
const Label = styled.span`
  @media ${NARROW} {
    display: none;
  }
`;
const Main = styled.div`
  position: relative;
  display: flex;
  flex: 1;
  min-height: 0;
  overflow: hidden;
  @media ${NARROW} {
    > hr {
      display: none;
    }
  }
`;
const Pane = styled.aside<{ $width: number; $side: "left" | "right" }>`
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  width: ${({ $width }) => $width}px;
  min-height: 0;
  overflow: auto;
  background: ${({ theme }) => theme.colors.surface.pane};
  @media ${NARROW} {
    position: absolute;
    top: 0;
    bottom: 0;
    ${({ $side }) => $side}: 0;
    z-index: 30;
    max-width: calc(100% - 48px);
    border-${({ $side }) => ($side === "left" ? "right" : "left")}: 1px solid
      ${({ theme }) => theme.colors.stroke.divider};
    box-shadow: ${({ theme }) => theme.shadow.flyout};
  }
`;
const Records = styled.main`
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  min-height: 0;
  background: ${({ theme }) => theme.colors.surface.pane};
`;
const Notice = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  padding: 4px 8px;
  border-bottom: 1px solid ${({ theme }) => theme.colors.stroke.divider};
  background: ${({ theme }) => theme.colors.surface.base};
  summary {
    cursor: default;
  }
  ul {
    margin: 4px 0 0 20px;
    max-height: 120px;
    overflow: auto;
  }
`;
const Empty = styled.div`
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
`;
const DropArea = styled.div`
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  width: min(480px, 100%);
  padding: 32px 24px;
  border: 1px dashed ${({ theme }) => theme.colors.stroke.strong};
  border-radius: ${({ theme }) => theme.radius.flyout};
  text-align: center;
  h1 {
    font-size: ${({ theme }) => theme.fontSize.title};
    line-height: 28px;
    font-weight: 600;
  }
  p {
    color: ${({ theme }) => theme.colors.text.secondary};
  }
  div {
    display: flex;
    gap: 8px;
    margin-top: 8px;
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
  const [showTree, setShowTree] = useState(() => !window.matchMedia(NARROW).matches);
  // The Columns flyout anchors to the toolbar button; null = closed.
  const [columnsAnchor, setColumnsAnchor] = useState<HTMLElement | null>(null);
  const showColumns = useCallback(
    () => setColumnsAnchor(document.getElementById("columns-button")),
    [],
  );
  const zone = useTimeZone();
  const [treeVersion, setTreeVersion] = useState(0);
  const [actionError, setActionError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const { filters, clearFilters } = useFilters();
  const { preference, setPreference } = useThemeMode();
  const {
    isLoading,
    matchedCount,
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
          icon: <Open16Regular />,
          shortcut: "Ctrl/⌘+O",
          onClick: open,
        },
        {
          id: "save",
          label: "Save Original Log As…",
          icon: <Save16Regular />,
          shortcut: "Ctrl/⌘+S",
          disabled: !fileInfo,
          onClick: saveOriginal,
        },
      ],
    },
    {
      id: "action",
      label: "Action",
      submenu: [
        {
          id: "refresh",
          label: "Refresh",
          icon: <ArrowClockwise16Regular />,
          shortcut: "F5",
          disabled: !fileInfo,
          onClick: () => void reload(),
        },
        {
          id: "clear-filters",
          label: "Clear Filters",
          disabled: !dataSource,
          onClick: clearFilters,
        },
        { id: "action-separator", separator: true as const },
        {
          id: "export",
          label: "Export Matching Events as JSON…",
          icon: <ArrowExportLtr16Regular />,
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
          id: "tree",
          label: "Log Tree",
          checked: showTree,
          onClick: () => setShowTree((value) => !value),
        },
        {
          id: "filters",
          label: "Filters Pane",
          checked: showFilters,
          onClick: () => setShowFilters((value) => !value),
        },
        { id: "columns", label: "Columns…", onClick: showColumns },
        { id: "theme-separator", separator: true as const },
        ...(["system", "light", "dark"] as const).map((value) => ({
          id: `theme-${value}`,
          label: { system: "System Theme", light: "Light Theme", dark: "Dark Theme" }[value],
          checked: preference === value,
          radio: true,
          onClick: () => setPreference(value),
        })),
        { id: "zone-separator", separator: true as const },
        ...(["local", "utc"] as const).map((value) => ({
          id: `zone-${value}`,
          label: { local: "Local Time", utc: "UTC" }[value],
          checked: zone === value,
          radio: true,
          onClick: () => setTimeZone(value),
        })),
      ],
    },
    {
      id: "help",
      label: "Help",
      submenu: [
        {
          id: "syntax",
          label: "Search Syntax and Shortcuts",
          onClick: () => document.getElementById("search-help")?.click(),
        },
        {
          id: "about",
          label: "About EVTX Viewer",
          onClick: () =>
            window.open("https://github.com/omerbenamram/evtx", "_blank", "noopener,noreferrer"),
        },
      ],
    },
  ];

  return (
    <>
      <GlobalStyles />
      <Shell>
        <MenuBar items={menus} />
        <Bar>
          <ToolbarButton
            icon={<Open16Regular />}
            label="Open log (Ctrl/⌘+O)"
            aria-label="Open"
            onClick={open}
          >
            <Label>Open</Label>
          </ToolbarButton>
          <ToolbarSeparator />
          <ToolbarButton
            icon={<PanelLeft16Regular />}
            label="Show log tree"
            active={showTree}
            onClick={() => setShowTree((value) => !value)}
          />
          <ToolbarButton
            icon={<Filter16Regular />}
            label="Show filters pane"
            aria-label="Filters"
            active={showFilters}
            onClick={() => setShowFilters((value) => !value)}
          >
            <Label>Filters</Label>
          </ToolbarButton>
          <ToolbarButton
            id="columns-button"
            icon={<Table16Regular />}
            label="Choose columns"
            aria-label="Columns"
            active={!!columnsAnchor}
            onClick={(event) => {
              const button = event.currentTarget;
              setColumnsAnchor((current) => (current ? null : button));
            }}
          >
            <Label>Columns</Label>
          </ToolbarButton>
          <ToolbarSeparator />
          <ToolbarButton
            icon={<ArrowClockwise16Regular />}
            label="Refresh (F5)"
            disabled={!fileInfo}
            onClick={() => void reload()}
          />
          <ToolbarButton
            icon={<ArrowExportLtr16Regular />}
            label="Export matching events as JSON"
            aria-label="Export"
            disabled={exportDisabled}
            onClick={() => void exportJson()}
          >
            <Label>{exporting ? "Exporting…" : "Export"}</Label>
          </ToolbarButton>
        </Bar>
        {(loadError || actionError) && (
          <Notice role="alert">
            <span>{actionError ?? loadError}</span>
            <Button
              onClick={() => {
                setActionError(null);
                void reload();
              }}
            >
              Retry
            </Button>
            <Button onClick={open}>Open another log</Button>
          </Notice>
        )}
        {cancelled && (
          <Notice as="output">
            Import cancelled. Showing the events loaded so far.
            <Button onClick={() => void reload()}>Restart import</Button>
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
          {showTree && (
            <>
              <Pane aria-label="Event logs" $side="left" $width={filesWidth}>
                <FileTree
                  onNodeSelect={handleNodeSelect}
                  selectedNodeId={selectedNodeId}
                  activeFileId={currentFileId}
                  ingestProgress={isLoading ? ingestProgress : 1}
                  refreshVersion={treeVersion}
                />
              </Pane>
              <ResizeHandle
                label="Resize log tree"
                value={filesWidth}
                min={160}
                max={400}
                orientation="vertical"
                onResize={setFilesWidth}
              />
            </>
          )}
          <Records>
            <SearchWorkspace disabled={!dataSource} />
            {dataSource ? (
              <>
                <EventTimeline disabled={isLoading} fileId={currentFileId} />
                <LogTableVirtual
                  key={currentFileId ?? "no-file"}
                  dataSource={dataSource}
                  onManageColumns={showColumns}
                />
              </>
            ) : (
              <Empty>
                <DropArea>
                  <h1>Open a Windows event log</h1>
                  <p>Drop an .evtx file here. It is processed locally in this browser.</p>
                  <div>
                    <Button variant="accent" icon={<Open16Regular />} onClick={open}>
                      Open log…
                    </Button>
                    <Button onClick={() => void openSample()}>Try example log</Button>
                  </div>
                </DropArea>
              </Empty>
            )}
          </Records>
          {showFilters && (
            <>
              <ResizeHandle
                label="Resize filters pane"
                value={filtersWidth}
                min={PANEL_MIN_WIDTH}
                max={PANEL_MAX_WIDTH}
                orientation="vertical"
                reverse
                onResize={setFiltersWidth}
              />
              <Pane aria-label="Event filters" $side="right" $width={filtersWidth}>
                <FilterSidebar />
              </Pane>
            </>
          )}
        </Main>
        <StatusBar onCancel={cancel} />
        <DragDropOverlay onFileSelect={handleFileSelect} />
        {columnsAnchor && (
          <ColumnManager anchor={columnsAnchor} onClose={() => setColumnsAnchor(null)} />
        )}
      </Shell>
    </>
  );
}

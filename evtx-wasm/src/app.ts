import init, { EvtxWasmParser, quick_file_info } from "/pkg/evtx_wasm.js";

// Types
interface FileInfo {
  total_chunks: number;
  next_record_id: number;
  is_dirty: boolean;
  is_full: boolean;
  chunks: ChunkInfo[];
}

interface ChunkInfo {
  chunk_number: number;
  record_count: number;
}

interface ParseResult {
  records: any[];
  total_records: number;
  errors: string[];
}

// Global state
class AppState {
  wasmModule: any = null;
  currentParser: EvtxWasmParser | null = null;
  currentFileData: Uint8Array | null = null;
  currentRecords: any[] = [];
  selectedChunk: { index: number; chunk: ChunkInfo } | null = null;
}

const state = new AppState();

// DOM elements (initialized after DOM loads)
let elements: {
  dropZone: HTMLElement;
  fileInput: HTMLInputElement;
  fileInfoContainer: HTMLElement;
  errorContainer: HTMLElement;
  chunkList: HTMLElement;
  recordList: HTMLElement;
  overlay: HTMLElement;
  recordDetail: HTMLElement;
  jsonViewer: HTMLElement;
  parseAllBtn: HTMLButtonElement;
  parseSelectedChunkBtn: HTMLButtonElement;
  searchBox: HTMLInputElement;
  exportBtn: HTMLButtonElement;
  closeDetailBtn: HTMLElement;
  // Stats elements
  fileName: HTMLElement;
  fileSize: HTMLElement;
  totalChunks: HTMLElement;
  nextRecordId: HTMLElement;
  fileStatus: HTMLElement;
  totalRecords: HTMLElement;
  parsedRecords: HTMLElement;
  errorCount: HTMLElement;
};

// Initialize WASM module
async function initializeWasm(): Promise<void> {
  try {
    console.log("🔄 Initializing WASM module...");
    // Explicitly pass the WASM file URL to ensure correct path resolution
    state.wasmModule = await init("/pkg/evtx_wasm_bg.wasm");
    console.log("✅ WASM module initialized successfully");
  } catch (error) {
    console.error("❌ Failed to initialize WASM module:", error);
    showError("Failed to initialize WASM module. Please refresh the page.");
    throw error;
  }
}

// Setup DOM elements and event handlers
function setupDOM(): void {
  console.log("🔧 Setting up DOM elements...");
  
  // Get all DOM elements
  elements = {
    dropZone: getElementById("dropZone"),
    fileInput: getElementById<HTMLInputElement>("fileInput"),
    fileInfoContainer: getElementById("fileInfoContainer"),
    errorContainer: getElementById("errorContainer"),
    chunkList: getElementById("chunkList"),
    recordList: getElementById("recordList"),
    overlay: getElementById("overlay"),
    recordDetail: getElementById("recordDetail"),
    jsonViewer: getElementById("jsonViewer"),
    parseAllBtn: getElementById<HTMLButtonElement>("parseAllBtn"),
    parseSelectedChunkBtn: getElementById<HTMLButtonElement>("parseSelectedChunkBtn"),
    searchBox: getElementById<HTMLInputElement>("searchBox"),
    exportBtn: getElementById<HTMLButtonElement>("exportBtn"),
    closeDetailBtn: getElementById("closeDetailBtn"),
    fileName: getElementById("fileName"),
    fileSize: getElementById("fileSize"),
    totalChunks: getElementById("totalChunks"),
    nextRecordId: getElementById("nextRecordId"),
    fileStatus: getElementById("fileStatus"),
    totalRecords: getElementById("totalRecords"),
    parsedRecords: getElementById("parsedRecords"),
    errorCount: getElementById("errorCount"),
  };

  // Set up event handlers
  setupEventHandlers();
}

// Helper to get element by ID with type assertion
function getElementById<T extends HTMLElement = HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) {
    throw new Error(`Element with id "${id}" not found`);
  }
  return element as T;
}

// Setup all event handlers
function setupEventHandlers(): void {
  // File input handlers
  elements.dropZone.addEventListener("click", () => elements.fileInput.click());
  elements.fileInput.addEventListener("change", handleFileSelect);

  // Drag and drop handlers
  setupDragAndDrop();

  // Button handlers
  elements.parseAllBtn.addEventListener("click", parseAllRecords);
  elements.parseSelectedChunkBtn.addEventListener("click", parseSelectedChunk);
  elements.exportBtn.addEventListener("click", exportRecords);
  elements.closeDetailBtn.addEventListener("click", hideRecordDetail);
  elements.overlay.addEventListener("click", hideRecordDetail);

  // Search handler
  elements.searchBox.addEventListener("input", (e) => {
    const searchTerm = (e.target as HTMLInputElement).value.toLowerCase();
    filterRecords(searchTerm);
  });

  // Prevent default drag behavior on document
  document.addEventListener("dragover", (e) => e.preventDefault());
  document.addEventListener("drop", (e) => e.preventDefault());
}

// Setup drag and drop functionality
function setupDragAndDrop(): void {
  const { dropZone } = elements;

  dropZone.addEventListener("dragenter", (e) => {
    e.preventDefault();
    e.stopPropagation();
    dropZone.classList.add("dragover");
  });

  dropZone.addEventListener("dragover", (e) => {
    e.preventDefault();
    e.stopPropagation();
    dropZone.classList.add("dragover");
  });

  dropZone.addEventListener("dragleave", (e) => {
    e.preventDefault();
    e.stopPropagation();
    
    // Only remove dragover if we're leaving the drop zone entirely
    const rect = dropZone.getBoundingClientRect();
    const x = e.clientX;
    const y = e.clientY;
    
    if (x <= rect.left || x >= rect.right || y <= rect.top || y >= rect.bottom) {
      dropZone.classList.remove("dragover");
    }
  });

  dropZone.addEventListener("drop", async (e) => {
    e.preventDefault();
    e.stopPropagation();
    dropZone.classList.remove("dragover");
    
    const files = e.dataTransfer?.files;
    if (files && files.length > 0) {
      await handleFile(files[0]);
    }
  });
}

// File handling
function handleFileSelect(e: Event): void {
  const input = e.target as HTMLInputElement;
  const file = input.files?.[0];
  if (file) {
    handleFile(file);
  }
}

async function handleFile(file: File): Promise<void> {
  if (!file.name.toLowerCase().endsWith(".evtx")) {
    showError("Please select a valid EVTX file");
    return;
  }

  clearError();
  showLoading(true);

  try {
    const arrayBuffer = await file.arrayBuffer();
    const uint8Array = new Uint8Array(arrayBuffer);
    state.currentFileData = uint8Array;

    // Get file info
    const fileInfo = await quick_file_info(uint8Array);
    displayFileInfo(file, fileInfo);

    // Create parser instance
    state.currentParser = new EvtxWasmParser(uint8Array);
    
    elements.fileInfoContainer.style.display = "block";
  } catch (error) {
    console.error("Error processing file:", error);
    showError(`Error processing file: ${error instanceof Error ? error.message : String(error)}`);
  } finally {
    showLoading(false);
  }
}

// Display file information
function displayFileInfo(file: File, info: FileInfo): void {
  elements.fileName.textContent = file.name;
  elements.fileSize.textContent = formatFileSize(file.size);
  elements.totalChunks.textContent = String(info.total_chunks);
  elements.nextRecordId.textContent = String(info.next_record_id);
  
  const status: string[] = [];
  if (info.is_dirty) status.push("Dirty");
  if (info.is_full) status.push("Full");
  elements.fileStatus.textContent = status.length > 0 ? status.join(", ") : "Clean";

  // Display chunks
  displayChunks(info.chunks);
}

// Display chunks
function displayChunks(chunks: ChunkInfo[]): void {
  elements.chunkList.innerHTML = "";
  
  chunks.forEach((chunk, index) => {
    const chunkEl = document.createElement("div");
    chunkEl.className = "chunk-item";
    chunkEl.innerHTML = `
      <div>Chunk ${chunk.chunk_number}</div>
      <div style="font-size: 12px; color: #666;">${chunk.record_count} records</div>
    `;
    chunkEl.addEventListener("click", () => selectChunk(index, chunk));
    elements.chunkList.appendChild(chunkEl);
  });
}

// Select a chunk
function selectChunk(index: number, chunk: ChunkInfo): void {
  state.selectedChunk = { index, chunk };
  
  // Update UI
  document.querySelectorAll(".chunk-item").forEach((el, i) => {
    el.classList.toggle("active", i === index);
  });
  
  elements.parseSelectedChunkBtn.disabled = false;
}

// Parse all records
async function parseAllRecords(): Promise<void> {
  if (!state.currentParser) return;
  
  showLoading(true);
  elements.recordList.innerHTML = '<div style="padding: 20px; text-align: center;">Parsing records...</div>';
  
  try {
    const result = await state.currentParser.parse_with_limit(1000);
    handleParseResult(result);
  } catch (error) {
    console.error("Error parsing records:", error);
    showError(`Error parsing records: ${error instanceof Error ? error.message : String(error)}`);
  } finally {
    showLoading(false);
  }
}

// Parse selected chunk
async function parseSelectedChunk(): Promise<void> {
  if (!state.currentParser || !state.selectedChunk) return;
  
  showLoading(true);
  elements.recordList.innerHTML = '<div style="padding: 20px; text-align: center;">Parsing chunk...</div>';
  
  try {
    const result = await state.currentParser.parse_chunk(state.selectedChunk.index);
    handleParseResult(result);
  } catch (error) {
    console.error("Error parsing chunk:", error);
    showError(`Error parsing chunk: ${error instanceof Error ? error.message : String(error)}`);
  } finally {
    showLoading(false);
  }
}

// Handle parse result
function handleParseResult(result: ParseResult): void {
  state.currentRecords = result.records;
  
  // Update stats
  elements.totalRecords.textContent = String(result.total_records);
  elements.parsedRecords.textContent = String(result.records.length);
  elements.errorCount.textContent = String(result.errors.length);
  
  // Display records
  displayRecords(result.records);
  
  // Enable export
  elements.exportBtn.disabled = false;
  
  // Show errors if any
  if (result.errors.length > 0) {
    console.warn("Parsing errors:", result.errors);
  }
}

// Display records
function displayRecords(records: any[]): void {
  elements.recordList.innerHTML = "";
  
  if (records.length === 0) {
    elements.recordList.innerHTML = '<div style="padding: 20px; text-align: center; color: #666;">No records found</div>';
    return;
  }
  
  records.forEach((record, index) => {
    const recordEl = createRecordElement(record, index);
    elements.recordList.appendChild(recordEl);
  });
}

// Create record element
function createRecordElement(record: any, index: number): HTMLElement {
  const div = document.createElement("div");
  div.className = "record-item";
  
  // Debug log to see the structure
  console.log("Record structure:", record);
  
  // The record structure has Event as the root with System inside
  const system = record.Event?.System;
  
  const eventRecordId = system?.EventRecordID || index;
  
  const eventId = system?.EventID || "Unknown";
  
  const timeCreated = system?.TimeCreated_attributes || system?.TimeCreated;
  const timestamp = timeCreated?.SystemTime || "Unknown";
  
  // Create preview
  let preview = "";
  if (record.Event?.EventData) {
    const data = record.Event.EventData;
    if (typeof data === "object") {
      if (data["#text"]) {
        preview = data["#text"];
      } else if (data.Data) {
        // Handle array of Data elements
        if (Array.isArray(data.Data)) {
          preview = data.Data.slice(0, 2).map((d: any) => 
            d["#attributes"]?.Name ? `${d["#attributes"].Name}: ${d["#text"] || ''}` : d["#text"] || ''
          ).join(", ");
        } else {
          preview = JSON.stringify(data.Data).substring(0, 50) + "...";
        }
      } else {
        preview = Object.entries(data).slice(0, 2).map(([k, v]) => `${k}: ${v}`).join(", ");
      }
    }
  } else if (record.Event?.UserData) {
    preview = "UserData event";
  } else if (system) {
    // If no EventData, show some system info
    const provider = system.Provider_attributes?.Name || system.Provider?.Name || "";
    const channel = system.Channel || "";
    if (provider || channel) {
      preview = [provider, channel].filter(Boolean).join(" - ");
    }
  }
  
  div.innerHTML = `
    <div class="record-header">
      <span class="record-id">Record #${eventRecordId} - Event ${eventId}</span>
      <span class="record-time">${formatTimestamp(timestamp)}</span>
    </div>
    <div class="record-preview">${preview || "No event data"}</div>
  `;
  
  div.addEventListener("click", () => showRecordDetail(record));
  
  return div;
}

// Filter records
function filterRecords(searchTerm: string): void {
  if (!searchTerm) {
    displayRecords(state.currentRecords);
    return;
  }
  
  const filtered = state.currentRecords.filter(record => {
    const recordStr = JSON.stringify(record).toLowerCase();
    return recordStr.includes(searchTerm);
  });
  
  displayRecords(filtered);
}

// Show record detail
function showRecordDetail(record: any): void {
  const formatted = JSON.stringify(record, null, 2);
  elements.jsonViewer.innerHTML = `<pre>${escapeHtml(formatted)}</pre>`;
  
  elements.overlay.classList.add("show");
  elements.recordDetail.classList.add("show");
}

// Hide record detail
function hideRecordDetail(): void {
  elements.overlay.classList.remove("show");
  elements.recordDetail.classList.remove("show");
}

// Export records
function exportRecords(): void {
  if (state.currentRecords.length === 0) return;
  
  const dataStr = JSON.stringify(state.currentRecords, null, 2);
  const blob = new Blob([dataStr], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  
  const a = document.createElement("a");
  a.href = url;
  a.download = `evtx_records_${new Date().toISOString()}.json`;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

// Utility functions
function formatFileSize(bytes: number): string {
  const sizes = ["Bytes", "KB", "MB", "GB"];
  if (bytes === 0) return "0 Bytes";
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return Math.round(bytes / Math.pow(1024, i) * 100) / 100 + " " + sizes[i];
}

function formatTimestamp(timestamp: string): string {
  if (!timestamp || timestamp === "Unknown") return timestamp;
  
  try {
    const date = new Date(timestamp);
    return date.toLocaleString();
  } catch {
    return timestamp;
  }
}

function escapeHtml(str: string): string {
  const div = document.createElement("div");
  div.textContent = str;
  return div.innerHTML;
}

function showError(message: string): void {
  elements.errorContainer.innerHTML = `<div class="error">${message}</div>`;
}

function clearError(): void {
  elements.errorContainer.innerHTML = "";
}

function showLoading(show: boolean): void {
  document.body.style.cursor = show ? "wait" : "default";
}

// Initialize on DOM ready
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", async () => {
    setupDOM();
    await initializeWasm();
  });
} else {
  // DOM already loaded
  setupDOM();
  initializeWasm();
}

// Export for debugging
(window as any).evtxApp = {
  state,
  elements,
  parseAllRecords,
  exportRecords,
};
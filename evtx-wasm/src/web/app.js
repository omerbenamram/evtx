import init, { EvtxWasmParser, quick_file_info } from '../pkg/evtx_wasm.js';

// Global state
let wasmModule = null;
let currentParser = null;
let currentFileData = null;
let currentRecords = [];
let selectedChunk = null;

// DOM elements (will be initialized after DOM loads)
let dropZone, fileInput, fileInfoContainer, errorContainer, chunkList, recordList, overlay, recordDetail, jsonViewer;

// Initialize WASM module
async function initializeWasm() {
    try {
        console.log('Starting WASM initialization...');
        wasmModule = await init();
        console.log('WASM module initialized successfully');
        
        // Add debug info
        console.log('Drop zone element:', dropZone);
        console.log('File input element:', fileInput);
    } catch (error) {
        console.error('Failed to initialize WASM module:', error);
        showError('Failed to initialize WASM module. Please refresh the page.');
    }
}

// Setup DOM and event handlers
function setupDOM() {
    console.log('Setting up DOM elements and event handlers...');
    
    // Get DOM elements
    dropZone = document.getElementById('dropZone');
    fileInput = document.getElementById('fileInput');
    fileInfoContainer = document.getElementById('fileInfoContainer');
    errorContainer = document.getElementById('errorContainer');
    chunkList = document.getElementById('chunkList');
    recordList = document.getElementById('recordList');
    overlay = document.getElementById('overlay');
    recordDetail = document.getElementById('recordDetail');
    jsonViewer = document.getElementById('jsonViewer');
    
    // Verify elements exist
    if (!dropZone || !fileInput) {
        console.error('Critical DOM elements not found!');
        return;
    }

    // File handling
    dropZone.addEventListener('click', () => fileInput.click());
    fileInput.addEventListener('change', handleFileSelect);

    // Drag and drop handlers
    dropZone.addEventListener('dragenter', (e) => {
        e.preventDefault();
        e.stopPropagation();
        dropZone.classList.add('dragover');
    });

    dropZone.addEventListener('dragover', (e) => {
        e.preventDefault();
        e.stopPropagation();
        dropZone.classList.add('dragover');
    });

    dropZone.addEventListener('dragleave', (e) => {
        e.preventDefault();
        e.stopPropagation();
        dropZone.classList.remove('dragover');
    });

    dropZone.addEventListener('drop', (e) => {
        e.preventDefault();
        e.stopPropagation();
        dropZone.classList.remove('dragover');
        
        const files = e.dataTransfer.files;
        if (files.length > 0) {
            handleFile(files[0]);
        }
    });
    
    // Setup other event handlers
    document.getElementById('parseAllBtn').addEventListener('click', parseAllRecords);
    document.getElementById('parseSelectedChunkBtn').addEventListener('click', parseSelectedChunk);
    document.getElementById('searchBox').addEventListener('input', (e) => {
        const searchTerm = e.target.value.toLowerCase();
        filterRecords(searchTerm);
    });
    document.getElementById('exportBtn').addEventListener('click', exportRecords);
    document.getElementById('closeDetailBtn').addEventListener('click', hideRecordDetail);
    overlay.addEventListener('click', hideRecordDetail);
}

function handleFileSelect(e) {
    const file = e.target.files[0];
    if (file) {
        handleFile(file);
    }
}

async function handleFile(file) {
    if (!file.name.toLowerCase().endsWith('.evtx')) {
        showError('Please select a valid EVTX file');
        return;
    }

    clearError();
    showLoading(true);

    try {
        const arrayBuffer = await file.arrayBuffer();
        const uint8Array = new Uint8Array(arrayBuffer);
        currentFileData = uint8Array;

        // Get file info
        const fileInfo = await quick_file_info(uint8Array);
        displayFileInfo(file, fileInfo);

        // Create parser instance
        currentParser = new EvtxWasmParser(uint8Array);
        
        fileInfoContainer.style.display = 'block';
        showLoading(false);
    } catch (error) {
        console.error('Error processing file:', error);
        showError(`Error processing file: ${error.message}`);
        showLoading(false);
    }
}

function displayFileInfo(file, info) {
    document.getElementById('fileName').textContent = file.name;
    document.getElementById('fileSize').textContent = formatFileSize(file.size);
    document.getElementById('totalChunks').textContent = info.total_chunks;
    document.getElementById('nextRecordId').textContent = info.next_record_id;
    
    const status = [];
    if (info.is_dirty) status.push('Dirty');
    if (info.is_full) status.push('Full');
    document.getElementById('fileStatus').textContent = status.length > 0 ? status.join(', ') : 'Clean';

    // Display chunks
    displayChunks(info.chunks);
}

function displayChunks(chunks) {
    chunkList.innerHTML = '';
    chunks.forEach((chunk, index) => {
        const chunkEl = document.createElement('div');
        chunkEl.className = 'chunk-item';
        chunkEl.innerHTML = `
            <div>Chunk ${chunk.chunk_number}</div>
            <div style="font-size: 12px; color: #666;">${chunk.record_count} records</div>
        `;
        chunkEl.addEventListener('click', () => selectChunk(index, chunk));
        chunkList.appendChild(chunkEl);
    });
}

function selectChunk(index, chunk) {
    selectedChunk = { index, chunk };
    
    // Update UI
    document.querySelectorAll('.chunk-item').forEach((el, i) => {
        el.classList.toggle('active', i === index);
    });
    
    document.getElementById('parseSelectedChunkBtn').disabled = false;
}

// Record parsing
async function parseAllRecords() {
    if (!currentParser) return;
    
    showLoading(true);
    recordList.innerHTML = '<div style="padding: 20px; text-align: center;">Parsing records...</div>';
    
    try {
        const result = await currentParser.parse_with_limit(1000); // Limit to 1000 records for performance
        handleParseResult(result);
    } catch (error) {
        console.error('Error parsing records:', error);
        showError(`Error parsing records: ${error.message}`);
    } finally {
        showLoading(false);
    }
}

async function parseSelectedChunk() {
    if (!currentParser || !selectedChunk) return;
    
    showLoading(true);
    recordList.innerHTML = '<div style="padding: 20px; text-align: center;">Parsing chunk...</div>';
    
    try {
        const result = await currentParser.parse_chunk(selectedChunk.index);
        handleParseResult(result);
    } catch (error) {
        console.error('Error parsing chunk:', error);
        showError(`Error parsing chunk: ${error.message}`);
    } finally {
        showLoading(false);
    }
}

function handleParseResult(result) {
    currentRecords = result.records;
    
    // Update stats
    document.getElementById('totalRecords').textContent = result.total_records;
    document.getElementById('parsedRecords').textContent = result.records.length;
    document.getElementById('errorCount').textContent = result.errors.length;
    
    // Display records
    displayRecords(result.records);
    
    // Enable export
    document.getElementById('exportBtn').disabled = false;
    
    // Show errors if any
    if (result.errors.length > 0) {
        console.warn('Parsing errors:', result.errors);
    }
}

function displayRecords(records) {
    recordList.innerHTML = '';
    
    if (records.length === 0) {
        recordList.innerHTML = '<div style="padding: 20px; text-align: center; color: #666;">No records found</div>';
        return;
    }
    
    records.forEach((record, index) => {
        const recordEl = createRecordElement(record, index);
        recordList.appendChild(recordEl);
    });
}

function createRecordElement(record, index) {
    const div = document.createElement('div');
    div.className = 'record-item';
    
    // Extract basic info
    const eventId = record.Event?.System?.EventID || 'Unknown';
    const timestamp = record.Event?.System?.TimeCreated?.['#attributes']?.SystemTime || 
                     record.Event?.System?.TimeCreated?.SystemTime || 'Unknown';
    const eventRecordId = record.Event?.System?.EventRecordID || index;
    
    // Create preview
    let preview = '';
    if (record.Event?.EventData) {
        const data = record.Event.EventData;
        if (typeof data === 'object') {
            preview = Object.entries(data).slice(0, 2).map(([k, v]) => `${k}: ${v}`).join(', ');
        }
    }
    
    div.innerHTML = `
        <div class="record-header">
            <span class="record-id">Record #${eventRecordId} - Event ${eventId}</span>
            <span class="record-time">${formatTimestamp(timestamp)}</span>
        </div>
        <div class="record-preview">${preview || 'No event data'}</div>
    `;
    
    div.addEventListener('click', () => showRecordDetail(record));
    
    return div;
}

// Search functionality
function filterRecords(searchTerm) {
    if (!searchTerm) {
        displayRecords(currentRecords);
        return;
    }
    
    const filtered = currentRecords.filter(record => {
        const recordStr = JSON.stringify(record).toLowerCase();
        return recordStr.includes(searchTerm);
    });
    
    displayRecords(filtered);
}

// Record detail view
function showRecordDetail(record) {
    const formatted = JSON.stringify(record, null, 2);
    jsonViewer.innerHTML = `<pre>${escapeHtml(formatted)}</pre>`;
    
    overlay.classList.add('show');
    recordDetail.classList.add('show');
}

function hideRecordDetail() {
    overlay.classList.remove('show');
    recordDetail.classList.remove('show');
}

// Export functionality
function exportRecords() {
    if (currentRecords.length === 0) return;
    
    const dataStr = JSON.stringify(currentRecords, null, 2);
    const blob = new Blob([dataStr], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    
    const a = document.createElement('a');
    a.href = url;
    a.download = `evtx_records_${new Date().toISOString()}.json`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
}

// Utility functions
function formatFileSize(bytes) {
    const sizes = ['Bytes', 'KB', 'MB', 'GB'];
    if (bytes === 0) return '0 Bytes';
    const i = Math.floor(Math.log(bytes) / Math.log(1024));
    return Math.round(bytes / Math.pow(1024, i) * 100) / 100 + ' ' + sizes[i];
}

function formatTimestamp(timestamp) {
    if (!timestamp || timestamp === 'Unknown') return timestamp;
    
    try {
        const date = new Date(timestamp);
        return date.toLocaleString();
    } catch {
        return timestamp;
    }
}

function escapeHtml(str) {
    const div = document.createElement('div');
    div.textContent = str;
    return div.innerHTML;
}

function showError(message) {
    errorContainer.innerHTML = `<div class="error">${message}</div>`;
}

function clearError() {
    errorContainer.innerHTML = '';
}

function showLoading(show) {
    if (show) {
        document.body.style.cursor = 'wait';
    } else {
        document.body.style.cursor = 'default';
    }
}

// Prevent default drag behavior on document
document.addEventListener('dragover', (e) => {
    e.preventDefault();
});

document.addEventListener('drop', (e) => {
    e.preventDefault();
});

// Initialize on load
window.addEventListener('DOMContentLoaded', async () => {
    setupDOM();
    await initializeWasm();
});
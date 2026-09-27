import { Decompress } from 'fzstd';
import init, { BrowserScanner, zstd_compress_bytes } from '../wasm/pkg/valheim_backup_cheat_scanner.js';
import { errorMessage } from './errors';
import { MAX_DECOMPRESSED_BYTES, MAX_FILE_BYTES, scrubKind, type FailedFile, type FinalReport, type ScanProgress, type ScrubResult, type WorkerRequest, type WorkerResponse, type WorkerResponseBody } from './scan';

const COMPRESSED_CHUNK_BYTES = 1024 * 1024;
// SAFETY: this module is only ever loaded as a dedicated module Worker, where
// `self` is a DedicatedWorkerGlobalScope exposing postMessage/onmessage. The
// DOM lib types `self` as Window, so the narrowed shape is asserted here.
const scope = self as unknown as {
  onmessage: ((event: MessageEvent<WorkerRequest>) => void) | null;
  postMessage: (message: WorkerResponse, transfer?: Transferable[]) => void;
};

let scannerPromise: Promise<BrowserScanner> | null = null;
let queue = Promise.resolve();

function post(id: number, response: WorkerResponseBody, transfer: Transferable[] = []): void {
  scope.postMessage({ id, ...response } as WorkerResponse, transfer);
}

async function getScanner(): Promise<BrowserScanner> {
  if (!scannerPromise) {
    scannerPromise = init().then(() => new BrowserScanner());
  }
  return scannerPromise;
}

function partialJson(json: string, failures: FailedFile[]): string {
  if (!failures.length) return json;
  let report: Record<string, unknown>;
  try {
    report = JSON.parse(json) as Record<string, unknown>;
  } catch (error) {
    throw new Error(`scanner returned malformed report JSON: ${error instanceof Error ? error.message : String(error)}`);
  }
  report.partial = true;
  report.failed_files = failures;
  return JSON.stringify(report);
}

function csvEscape(value: string): string {
  return /[",\r\n]/.test(value) ? `"${value.replaceAll('"', '""')}"` : value;
}

function partialCsv(csv: string, failures: FailedFile[]): string {
  if (!failures.length) return csv;
  const header = csv.split(/\r?\n/, 1)[0] ?? '';
  const headers = header.split(',');
  const column = (name: string): number => {
    const index = headers.indexOf(name);
    if (index < 0) throw new Error(`scanner CSV header is missing ${name} column`);
    return index;
  };
  const parseErrorColumn = column('parse_error');
  const recordTypeColumn = column('record_type');
  const statusColumn = column('status');
  const archiveColumn = column('archive');
  const rows = failures.map(({ name, error }) => {
    const fields = Array<string>(headers.length).fill('');
    fields[recordTypeColumn] = 'error';
    fields[statusColumn] = 'error';
    fields[archiveColumn] = name;
    fields[parseErrorColumn] = error;
    return fields.map(csvEscape).join(',');
  });
  return `${csv}${rows.join('\n')}\n`;
}

function markdownCell(value: string): string {
  return value.replaceAll('\\', '\\\\').replaceAll('|', '\\|').replaceAll('`', '\\`').replace(/[\r\n]/g, ' ');
}

function partialMarkdown(markdown: string, failures: FailedFile[]): string {
  if (!failures.length) return markdown;
  return `${markdown}\n## Partial report\n\n${failures.length} file(s) failed; successful files remain included.\n\n${failures.map(({ name, error }) => `- ${markdownCell(name)}: ${markdownCell(error)}`).join('\n')}\n`;
}

async function report(id: number, failures: FailedFile[]): Promise<void> {
  try {
    const scanner = await getScanner();
    const result: FinalReport = {
      json: partialJson(scanner.report_json(), failures),
      csv: partialCsv(scanner.report_csv(), failures),
      markdown: partialMarkdown(scanner.report_markdown(), failures),
    };
    post(id, { type: 'report', report: result });
  } catch (error: unknown) {
    post(id, { type: 'error', message: errorMessage(error) });
  }
}

function decompressStreaming(buffer: ArrayBuffer, id: number, label = 'decompressing'): ArrayBuffer {
  const input = new Uint8Array(buffer);
  const chunks: Uint8Array[] = [];
  let outputBytes = 0;
  const stream = new Decompress((chunk) => {
    if (outputBytes + chunk.byteLength > MAX_DECOMPRESSED_BYTES) {
      throw new Error(`decompressed file exceeds ${Math.ceil(MAX_DECOMPRESSED_BYTES / 1048576)} MiB`);
    }
    outputBytes += chunk.byteLength;
    // fzstd allocates each streamed block; retain it and concatenate once for Rust's contiguous tar API.
    chunks.push(chunk);
  });
  if (!input.byteLength) {
    stream.push(new Uint8Array(0), true);
  } else {
    for (let offset = 0; offset < input.byteLength; offset += COMPRESSED_CHUNK_BYTES) {
      const end = Math.min(offset + COMPRESSED_CHUNK_BYTES, input.byteLength);
      stream.push(input.subarray(offset, end), end === input.byteLength);
      post(id, {
        type: 'progress',
        progress: {
          stage: 'decompressing',
          percent: 5 + Math.round((end / input.byteLength) * 30),
          message: `${label} ${Math.round((end / input.byteLength) * 100)}%`,
        },
      });
    }
  }
  const output = new Uint8Array(outputBytes);
  let offset = 0;
  for (const chunk of chunks) {
    output.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return output.buffer;
}

async function scan(id: number, name: string, buffer: ArrayBuffer, modifiedUnixMillis: number, inputId: number): Promise<void> {
  if (buffer.byteLength > MAX_FILE_BYTES) {
    throw new Error(`file is too large (${Math.ceil(buffer.byteLength / 1048576)} MiB; limit ${MAX_FILE_BYTES / 1048576} MiB)`);
  }
  const scanner = await getScanner();
  post(id, { type: 'progress', progress: { stage: 'reading', percent: 1, message: 'reading file' } satisfies ScanProgress });
  let bytes = buffer;
  if (/\.tar\.zst$/i.test(name)) {
    post(id, { type: 'progress', progress: { stage: 'decompressing', percent: 5, message: 'decompressing' } satisfies ScanProgress });
    bytes = decompressStreaming(buffer, id);
    post(id, { type: 'progress', progress: { stage: 'parsing', percent: 40, message: 'parsing decompressed tar' } satisfies ScanProgress });
    scanner.add_tar(name, new Uint8Array(bytes));
  } else {
    post(id, { type: 'progress', progress: { stage: 'parsing', percent: 40, message: 'parsing' } satisfies ScanProgress });
    scanner.add_file_with_metadata(name, new Uint8Array(bytes), modifiedUnixMillis, inputId);
  }
  post(id, { type: 'progress', progress: { stage: 'complete', percent: 100, message: 'complete' } satisfies ScanProgress });
}

function sameBytes(left: Uint8Array, right: Uint8Array): boolean {
  if (left.byteLength !== right.byteLength) return false;
  for (let index = 0; index < left.byteLength; index += 1) {
    if (left[index] !== right[index]) return false;
  }
  return true;
}

// Experimental: a copy of the input with every cheat flag cleared, in the input's own format.
async function scrub(id: number, name: string, buffer: ArrayBuffer): Promise<ScrubResult> {
  if (buffer.byteLength > MAX_FILE_BYTES) {
    throw new Error(`file is too large (${Math.ceil(buffer.byteLength / 1048576)} MiB; limit ${MAX_FILE_BYTES / 1048576} MiB)`);
  }
  const kind = scrubKind(name);
  if (!kind) throw new Error('Only .tar.zst, .tar and v41 .chunk world files can be scrubbed; legacy .db worlds and character profiles are not supported.');
  const scanner = await getScanner();
  const progress = (stage: ScanProgress['stage'], percent: number, message: string) => post(id, { type: 'progress', progress: { stage, percent, message } });
  const tar = kind === 'tar.zst' ? decompressStreaming(buffer, id) : buffer;
  progress('scrubbing', 40, 'clearing flags and verifying');
  const scrubbed = kind === 'chunk' ? scanner.scrub_chunk(name, new Uint8Array(tar)) : scanner.scrub_tar(new Uint8Array(tar));
  const zdoFlags = scrubbed.zdo_flags();
  const itemBits = scrubbed.item_bits();
  const auditJson = scrubbed.audit_json();
  const auditMarkdown = scrubbed.audit_markdown();
  const zdoCount = scrubbed.zdo_count();
  let bytes = scrubbed.take_bytes();
  scrubbed.free();
  if (zdoFlags + itemBits === 0) throw new Error('No cheat flags found in this file; there is nothing to clear.');
  if (kind === 'tar.zst') {
    progress('compressing', 60, 'compressing (zstd)');
    const compressed = zstd_compress_bytes(bytes);
    // Decode with fzstd, an independent implementation, before anything leaves the worker.
    const roundTrip = new Uint8Array(decompressStreaming(compressed.buffer as ArrayBuffer, id, 'verifying'));
    if (!sameBytes(roundTrip, bytes)) throw new Error('compressed copy did not decompress to the scrubbed tar; nothing was saved');
    bytes = compressed;
  }
  progress('complete', 100, 'complete');
  return { name, bytes: bytes.buffer as ArrayBuffer, auditJson, auditMarkdown, zdoFlags, itemBits, zdoCount };
}

function canonical(id: number, index: number): Promise<void> {
  return getScanner()
    .then((scanner) => scanner.set_canonical(index))
    .then(() => post(id, { type: 'scanned' }))
    .catch((error: unknown) => post(id, { type: 'error', message: errorMessage(error) }));
}

function handle(request: WorkerRequest): Promise<void> {
  if (request.type === 'report') return report(request.id, request.failures);
  if (request.type === 'canonical') return canonical(request.id, request.index);
  if (request.type === 'scrub') {
    return scrub(request.id, request.name, request.buffer)
      .then((result) => post(request.id, { type: 'scrubbed', result }, [result.bytes]))
      .catch((error: unknown) => post(request.id, { type: 'error', message: errorMessage(error) }));
  }
  return scan(request.id, request.name, request.buffer, request.modifiedUnixMillis, request.inputId)
    .then(() => post(request.id, { type: 'scanned' }))
    .catch((error: unknown) => post(request.id, { type: 'error', message: errorMessage(error) }));
}

scope.onmessage = (event) => {
  queue = queue.then(() => handle(event.data));
};

export {};

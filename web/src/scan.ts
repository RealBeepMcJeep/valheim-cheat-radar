import type { ScrubMode } from './scrub';
import type { Report } from './types';

export const MAX_FILE_BYTES = 128 * 1024 * 1024;
export const MAX_DECOMPRESSED_BYTES = 256 * 1024 * 1024;

export type ScanStage = 'reading' | 'decompressing' | 'parsing' | 'scrubbing' | 'compressing' | 'verifying' | 'complete';
export type ScanProgress = { stage: ScanStage; percent: number; message: string };
export type FailedFile = { name: string; error: string };
export type FinalReport = { json: string; csv: string; markdown: string };
/** One snapshot of a valheim-vault store, as the worker lists it for the page's picker. */
export type VaultSnapshotRow = {
  archive: string;
  snapshot: string;
  /** In the report now. Starts true for snapshots on the timeline. */
  included: boolean;
  /** False for a snapshot that holds no world: listed, never scanned into the report. */
  selectable: boolean;
  /** Why it starts left out: no world, a duplicate of an earlier snapshot, or a scrubbed copy. */
  note: string | null;
  zdo_count: number;
  flagged: number;
};

/**
 * Above this zstd window the worker hands a `.tar.zst` to the Rust decoder: fzstd copies its whole
 * window for every block, which a vault's 1 GiB window makes hopeless. Host backups use 2 MiB.
 */
export const LONG_WINDOW_BYTES = 8 * 1024 * 1024;

/** The window a zstd frame declares (RFC 8878, frame header), or 0 when this is no zstd frame. */
export function zstdWindowBytes(bytes: Uint8Array): number {
  if (bytes.length < 6 || bytes[0] !== 0x28 || bytes[1] !== 0xb5 || bytes[2] !== 0x2f || bytes[3] !== 0xfd) return 0;
  const descriptor = bytes[4];
  if (descriptor & 0x20) {
    // Single segment: no window descriptor; the window is the content size.
    const sizeBytes = [1, 2, 4, 8][descriptor >> 6];
    const start = 5 + [0, 1, 2, 4][descriptor & 3];
    let size = 0;
    for (let index = sizeBytes - 1; index >= 0; index -= 1) size = size * 256 + (bytes[start + index] ?? 0);
    return sizeBytes === 2 ? size + 256 : size;
  }
  const base = 2 ** (10 + (bytes[5] >> 3));
  return base + (base / 8) * (bytes[5] & 7);
}

/** A scrubbed copy in the input file's format, with the audit of everything the scrub did. */
export type ScrubResult = { name: string; mode: ScrubMode; bytes: ArrayBuffer; auditJson: string; auditMarkdown: string; zdoCountBefore: number; zdoCountAfter: number };

/** Which world inputs the experimental scrub can hand back in their own format. */
export function scrubKind(name: string): 'tar.zst' | 'tar' | 'chunk' | null {
  if (/\.tar\.zst$/i.test(name)) return 'tar.zst';
  if (/\.tar$/i.test(name)) return 'tar';
  if (/\.chunk$/i.test(name)) return 'chunk';
  return null;
}

/**
 * `backup.tar.zst` -> `backup-scrubbed.tar.zst`, so the copy never passes for the original. A lone
 * `.chunk` keeps its name: the game only loads a chunk under the exact name its index lists and
 * deletes any other `.chunk` in the world folder.
 */
export function scrubbedName(name: string): string {
  const kind = scrubKind(name);
  if (kind !== 'tar.zst' && kind !== 'tar') return name;
  const extension = name.slice(name.length - (kind === 'tar.zst' ? 8 : 4));
  return `${name.slice(0, name.length - extension.length)}-scrubbed${extension}`;
}

type WorkerRequestBody =
  | { type: 'scan'; name: string; modifiedUnixMillis: number; inputId: number; buffer: ArrayBuffer }
  | { type: 'scrub'; name: string; mode: ScrubMode; buffer: ArrayBuffer }
  | { type: 'wasm'; bytes: ArrayBuffer }
  | { type: 'canonical'; index: number }
  | { type: 'select'; archives: string[] }
  | { type: 'report'; failures: FailedFile[] };
export type WorkerRequest = WorkerRequestBody & { id: number };

export type WorkerResponseBody =
  | { type: 'progress'; progress: ScanProgress }
  | { type: 'scanned'; vault?: VaultSnapshotRow[] }
  | { type: 'report'; report: FinalReport }
  | { type: 'scrubbed'; result: ScrubResult }
  | { type: 'error'; message: string };
export type WorkerResponse = WorkerResponseBody & { id: number };

export interface ScannerWorkerLike {
  postMessage(message: WorkerRequest): void;
  postMessage(message: WorkerRequest, transfer: Transferable[]): void;
  addEventListener(type: 'message' | 'error', listener: EventListener): void;
  removeEventListener(type: 'message' | 'error', listener: EventListener): void;
  terminate(): void;
}

type Pending = {
  resolve: (value: void | FinalReport | ScrubResult | VaultSnapshotRow[]) => void;
  reject: (error: Error) => void;
  onProgress?: (progress: ScanProgress) => void;
};

export class ScanCancelledError extends Error {
  constructor() {
    super('Scan cancelled.');
    this.name = 'ScanCancelledError';
  }
}

export class ScannerWorkerClient {
  private readonly workerFactory: () => ScannerWorkerLike;
  private worker: ScannerWorkerLike | null = null;
  private nextRequestId = 1;
  private generation = 0;
  private readonly pending = new Map<number, Pending>();

  constructor(workerFactory: () => ScannerWorkerLike = () => new Worker(new URL('./scanner-worker.ts', import.meta.url), { type: 'module' })) {
    this.workerFactory = workerFactory;
  }

  /** Resolves with the snapshot list when the file was a valheim-vault store. */
  async scanFile(file: File, onProgress?: (progress: ScanProgress) => void, inputId = 0): Promise<VaultSnapshotRow[] | undefined> {
    if (file.size > MAX_FILE_BYTES) {
      throw new Error(`file is too large (${formatBytes(file.size)}; limit ${MAX_FILE_BYTES / 1048576} MiB)`);
    }
    const generation = this.generation;
    const buffer = await file.arrayBuffer();
    if (generation !== this.generation) throw new ScanCancelledError();
    return this.send(
      {
        type: 'scan',
        name: file.name,
        modifiedUnixMillis: file.lastModified,
        inputId,
        buffer,
      },
      [buffer],
      onProgress,
    ) as Promise<VaultSnapshotRow[] | undefined>;
  }

  async scrubFile(file: File, mode: ScrubMode, onProgress?: (progress: ScanProgress) => void): Promise<ScrubResult> {
    if (file.size > MAX_FILE_BYTES) {
      throw new Error(`file is too large (${formatBytes(file.size)}; limit ${MAX_FILE_BYTES / 1048576} MiB)`);
    }
    const generation = this.generation;
    const buffer = await file.arrayBuffer();
    if (generation !== this.generation) throw new ScanCancelledError();
    return this.send({ type: 'scrub', name: file.name, mode, buffer }, [buffer], onProgress) as Promise<ScrubResult>;
  }

  setCanonical(index: number): Promise<void> {
    return this.send({ type: 'canonical', index }) as Promise<void>;
  }

  /** Includes exactly these vault snapshots (by archive name) in the report; other vault snapshots are left out. */
  selectVaultArchives(archives: string[]): Promise<void> {
    return this.send({ type: 'select', archives }) as Promise<void>;
  }

  report(failures: FailedFile[] = []): Promise<FinalReport> {
    return this.send({ type: 'report', failures }) as Promise<FinalReport>;
  }

  cancel(): void {
    this.generation += 1;
    const error = new ScanCancelledError();
    for (const pending of this.pending.values()) pending.reject(error);
    this.pending.clear();
    this.worker?.terminate();
    this.worker = null;
  }

  dispose(): void {
    this.cancel();
  }

  private ensureWorker(): ScannerWorkerLike {
    if (this.worker) return this.worker;
    const worker = this.workerFactory();
    const onMessage: EventListener = (event) => this.handleMessage(event as MessageEvent<WorkerResponse>);
    const onError: EventListener = (event) => {
      const detail = event as ErrorEvent;
      this.failWorker(new Error(detail.message || 'Scanner worker stopped unexpectedly.'));
    };
    worker.addEventListener('message', onMessage);
    worker.addEventListener('error', onError);
    // The single-file build embeds the WASM in the page (see build-single-file.mjs); hand it over
    // before any work so the worker never tries to fetch it.
    const inlineWasm = (globalThis as { __VCR_WASM_BASE64__?: string }).__VCR_WASM_BASE64__;
    if (inlineWasm) {
      const bytes = Uint8Array.from(atob(inlineWasm), (char) => char.charCodeAt(0)).buffer;
      worker.postMessage({ type: 'wasm', bytes, id: 0 }, [bytes]);
    }
    this.worker = worker;
    return worker;
  }

  private send(request: WorkerRequestBody, transfer: Transferable[] = [], onProgress?: (progress: ScanProgress) => void): Promise<void | FinalReport | ScrubResult | VaultSnapshotRow[]> {
    const worker = this.ensureWorker();
    const id = this.nextRequestId++;
    const message: WorkerRequest = { ...request, id };
    return new Promise<void | FinalReport | ScrubResult | VaultSnapshotRow[]>((resolve, reject) => {
      this.pending.set(id, { resolve, reject, onProgress });
      try {
        worker.postMessage(message, transfer);
      } catch (error) {
        this.pending.delete(id);
        reject(error instanceof Error ? error : new Error('Could not send work to scanner worker.'));
      }
    });
  }

  private handleMessage(event: MessageEvent<WorkerResponse>): void {
    const response = event.data;
    const pending = this.pending.get(response.id);
    if (!pending) return;
    if (response.type === 'progress') {
      pending.onProgress?.(response.progress);
      return;
    }
    this.pending.delete(response.id);
    if (response.type === 'error') pending.reject(new Error(response.message));
    else if (response.type === 'scanned') pending.resolve(response.vault);
    else if (response.type === 'scrubbed') pending.resolve(response.result);
    else pending.resolve(response.report);
  }

  private failWorker(error: Error): void {
    for (const pending of this.pending.values()) pending.reject(error);
    this.pending.clear();
    this.worker?.terminate();
    this.worker = null;
  }
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KiB`;
  return `${(bytes / 1048576).toFixed(1)} MiB`;
}

export function parseReport(json: string): Report {
  try {
    return JSON.parse(json) as Report;
  } catch (error) {
    throw new Error(`scanner returned malformed report JSON: ${error instanceof Error ? error.message : String(error)}`);
  }
}

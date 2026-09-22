import type { Report } from './types';

export const MAX_FILE_BYTES = 128 * 1024 * 1024;
export const MAX_DECOMPRESSED_BYTES = 256 * 1024 * 1024;

export type ScanStage = 'reading' | 'decompressing' | 'parsing' | 'complete';
export type ScanProgress = { stage: ScanStage; percent: number; message: string };
export type FailedFile = { name: string; error: string };
export type FinalReport = { json: string; csv: string; markdown: string };

type WorkerRequestBody =
  | { type: 'scan'; name: string; modifiedUnixMillis: number; inputId: number; buffer: ArrayBuffer }
  | { type: 'canonical'; index: number }
  | { type: 'report'; failures: FailedFile[] };
export type WorkerRequest = WorkerRequestBody & { id: number };

export type WorkerResponseBody =
  | { type: 'progress'; progress: ScanProgress }
  | { type: 'scanned' }
  | { type: 'report'; report: FinalReport }
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
  resolve: (value: void | FinalReport) => void;
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

  async scanFile(file: File, onProgress?: (progress: ScanProgress) => void, inputId = 0): Promise<void> {
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
    ) as Promise<void>;
  }

  setCanonical(index: number): Promise<void> {
    return this.send({ type: 'canonical', index }) as Promise<void>;
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
    this.worker = worker;
    return worker;
  }

  private send(request: WorkerRequestBody, transfer: Transferable[] = [], onProgress?: (progress: ScanProgress) => void): Promise<void | FinalReport> {
    const worker = this.ensureWorker();
    const id = this.nextRequestId++;
    const message: WorkerRequest = { ...request, id };
    return new Promise<void | FinalReport>((resolve, reject) => {
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
    else if (response.type === 'scanned') pending.resolve();
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

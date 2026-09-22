import { describe, expect, it } from 'vitest';
import { errorMessage } from './errors';
import { ScannerWorkerClient, ScanCancelledError, type ScannerWorkerLike, type WorkerRequest, type WorkerResponse } from './scan';

class FakeWorker implements ScannerWorkerLike {
  readonly messages: WorkerRequest[] = [];
  readonly transfers: Transferable[][] = [];
  terminated = false;
  private readonly listeners = new Map<'message' | 'error', EventListener[]>();

  postMessage(message: WorkerRequest): void;
  postMessage(message: WorkerRequest, transfer: Transferable[]): void;
  postMessage(message: WorkerRequest, transfer: Transferable[] = []): void {
    this.messages.push(message);
    this.transfers.push(transfer);
  }

  addEventListener(type: 'message' | 'error', listener: EventListener): void {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }

  removeEventListener(type: 'message' | 'error', listener: EventListener): void {
    this.listeners.set(type, (this.listeners.get(type) ?? []).filter((candidate) => candidate !== listener));
  }

  terminate(): void {
    this.terminated = true;
  }

  respond(response: WorkerResponse): void {
    for (const listener of this.listeners.get('message') ?? []) listener(new MessageEvent('message', { data: response }));
  }
}

const file = (name = 'save.db') => new File([new Uint8Array([1, 2, 3])], name, { lastModified: 1_700_000_000_000 });

describe('ScannerWorkerClient', () => {
  it('uses unique request IDs, transfers file bytes, and forwards progress', async () => {
    const worker = new FakeWorker();
    const client = new ScannerWorkerClient(() => worker);
    const progress: string[] = [];
    const first = client.scanFile(file(), (value) => progress.push(value.message), 42);
    await new Promise((resolve) => setTimeout(resolve, 0));
    const request = worker.messages[0];
    expect(request.type).toBe('scan');
    expect(worker.transfers[0]).toHaveLength(1);
    expect(request.id).toBe(1);
    if (request.type === 'scan') {
      expect(request.modifiedUnixMillis).toBe(1_700_000_000_000);
      expect(request.inputId).toBe(42);
    }
    worker.respond({ id: request.id, type: 'progress', progress: { stage: 'parsing', percent: 40, message: 'parsing' } });
    worker.respond({ id: request.id, type: 'scanned' });
    await expect(first).resolves.toBeUndefined();
    expect(progress).toEqual(['parsing']);

    const second = client.scanFile(file('next.db'));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(worker.messages[1].id).toBe(2);
    worker.respond({ id: worker.messages[1].id, type: 'scanned' });
    await expect(second).resolves.toBeUndefined();

    const canonical = client.setCanonical(1);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(worker.messages[2]).toMatchObject({ id: 3, type: 'canonical', index: 1 });
    worker.respond({ id: worker.messages[2].id, type: 'scanned' });
    await expect(canonical).resolves.toBeUndefined();
  });

  it('rejects pending work and terminates the worker on cancel', async () => {
    const worker = new FakeWorker();
    const client = new ScannerWorkerClient(() => worker);
    const pending = client.scanFile(file());
    await new Promise((resolve) => setTimeout(resolve, 0));
    client.cancel();
    await expect(pending).rejects.toBeInstanceOf(ScanCancelledError);
    expect(worker.terminated).toBe(true);
  });

  it('keeps wasm-bindgen string errors and only falls back when there is nothing to show', () => {
    expect(errorMessage('tar checksum mismatch')).toBe('tar checksum mismatch');
    expect(errorMessage(new Error('scan failed'))).toBe('scan failed');
    expect(errorMessage(undefined)).toBe('Scanner worker failed.');
    expect(errorMessage(42)).toBe('Scanner worker failed.');
    expect(errorMessage({})).toBe('Scanner worker failed.');
  });
});

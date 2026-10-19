import { describe, expect, it } from 'vitest';
import { errorMessage } from './errors';
import { ScannerWorkerClient, ScanCancelledError, scrubKind, scrubbedName, zstdWindowBytes, type ScannerWorkerLike, type WorkerRequest, type WorkerResponse } from './scan';

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

describe('scrubKind', () => {
  it('accepts world inputs it can hand back in their own format and nothing else', () => {
    expect(scrubKind('2026-09-25_abc.tar.zst')).toBe('tar.zst');
    expect(scrubKind('backup.TAR')).toBe('tar');
    expect(scrubKind('00_00__1_1.chunk')).toBe('chunk');
    expect(scrubKind('Dedicated.db')).toBeNull();
    expect(scrubKind('hero.fch')).toBeNull();
    expect(scrubKind('notes.tar.zst.txt')).toBeNull();
  });
});

describe('scrubbedName', () => {
  it('suffixes backups before the extension but keeps a chunk loadable', () => {
    expect(scrubbedName('2026-09-25_abc-340946ab.tar.zst')).toBe('2026-09-25_abc-340946ab-scrubbed.tar.zst');
    expect(scrubbedName('world.TAR')).toBe('world-scrubbed.TAR');
    expect(scrubbedName('00_00__1_1.chunk')).toBe('00_00__1_1.chunk');
  });
});

describe('zstdWindowBytes', () => {
  const magic = [0x28, 0xb5, 0x2f, 0xfd];
  it('reads the window descriptor, the single-segment content size, and rejects non-zstd bytes', () => {
    // Window descriptor 0x58: exponent 11 -> 2^21, mantissa 0 (a host backup's 2 MiB).
    expect(zstdWindowBytes(new Uint8Array([...magic, 0x04, 0x58]))).toBe(2 * 1024 * 1024);
    // valheim-vault's --long=30: exponent 20 -> 1 GiB.
    expect(zstdWindowBytes(new Uint8Array([...magic, 0x04, 20 << 3]))).toBe(1024 ** 3);
    // Mantissa 4 adds four eighths: 1.5 MiB.
    expect(zstdWindowBytes(new Uint8Array([...magic, 0x04, (10 << 3) | 4]))).toBe(1.5 * 1024 * 1024);
    // Single segment with a 4-byte content size of 1,000,000.
    expect(zstdWindowBytes(new Uint8Array([...magic, 0xa0, 0x40, 0x42, 0x0f, 0x00]))).toBe(1_000_000);
    // Single segment, 2-byte size field: value + 256.
    expect(zstdWindowBytes(new Uint8Array([...magic, 0x60, 0x00, 0x01]))).toBe(512);
    expect(zstdWindowBytes(new Uint8Array([1, 2, 3, 4, 5, 6]))).toBe(0);
  });

  it('hands a vault scan its snapshot list', async () => {
    const worker = new FakeWorker();
    const client = new ScannerWorkerClient(() => worker);
    const scan = client.scanFile(file('valheim-backups.tar.zst'));
    await new Promise((resolve) => setTimeout(resolve, 0));
    const vault = [{ archive: 'v#2026-09-09', snapshot: '2026-09-09', included: true, selectable: true, note: null, zdo_count: 6, flagged: 1 }];
    worker.respond({ id: worker.messages[0].id, type: 'scanned', vault });
    await expect(scan).resolves.toEqual(vault);
    const select = client.selectVaultArchives(['v#2026-09-09']);
    expect(worker.messages[1]).toMatchObject({ type: 'select', archives: ['v#2026-09-09'] });
    worker.respond({ id: worker.messages[1].id, type: 'scanned' });
    await expect(select).resolves.toBeUndefined();
  });
});

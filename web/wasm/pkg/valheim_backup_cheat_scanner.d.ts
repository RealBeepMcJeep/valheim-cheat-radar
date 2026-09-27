/* tslint:disable */
/* eslint-disable */
/**
* @param {Uint8Array} bytes
* @returns {Uint8Array}
*/
export function zstd_compress_bytes(bytes: Uint8Array): Uint8Array;
/**
*/
export class BrowserScanner {
  free(): void;
/**
* @returns {string}
*/
  report_csv(): string;
/**
* @returns {string}
*/
  report_json(): string;
/**
* Experimental Mode A scrub of one standalone v41 chunk file.
* @param {string} name
* @param {Uint8Array} bytes
* @returns {BrowserScrub}
*/
  scrub_chunk(name: string, bytes: Uint8Array): BrowserScrub;
/**
* @returns {number}
*/
  archive_count(): number;
/**
* @param {number} index
*/
  set_canonical(index: number): void;
/**
* @returns {number}
*/
  character_count(): number;
/**
* @returns {string}
*/
  report_markdown(): string;
/**
* @param {string} name
* @param {Uint8Array} bytes
* @param {number} modified_unix_millis
* @param {number} input_id
*/
  add_file_with_metadata(name: string, bytes: Uint8Array, modified_unix_millis: number, input_id: number): void;
/**
* @param {string} name
* @param {Uint8Array} bytes
* @param {number} modified_unix_millis
*/
  add_file_with_modified_unix(name: string, bytes: Uint8Array, modified_unix_millis: number): void;
/**
*/
  constructor();
/**
*/
  reset(): void;
/**
* @param {string} name
* @param {Uint8Array} bytes
*/
  add_tar(name: string, bytes: Uint8Array): void;
/**
* @param {string} name
* @param {Uint8Array} bytes
*/
  add_file(name: string, bytes: Uint8Array): void;
/**
* Experimental Mode A scrub of a decompressed backup tar; never touches the scan results.
* @param {Uint8Array} bytes
* @returns {BrowserScrub}
*/
  scrub_tar(bytes: Uint8Array): BrowserScrub;
}
/**
* A scrubbed copy plus its audit, handed to the browser worker.
*/
export class BrowserScrub {
  free(): void;
/**
* @returns {string}
*/
  audit_json(): string;
/**
* The scrubbed bytes; moved out, so a second call returns an empty array.
* @returns {Uint8Array}
*/
  take_bytes(): Uint8Array;
/**
* @returns {string}
*/
  audit_markdown(): string;
/**
* @returns {number}
*/
  item_bits(): number;
/**
* @returns {number}
*/
  zdo_count(): number;
/**
* @returns {number}
*/
  zdo_flags(): number;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly memory: WebAssembly.Memory;
  readonly __wbg_browserscanner_free: (a: number) => void;
  readonly __wbg_browserscrub_free: (a: number) => void;
  readonly browserscanner_add_file: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
  readonly browserscanner_add_file_with_metadata: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => void;
  readonly browserscanner_add_file_with_modified_unix: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
  readonly browserscanner_add_tar: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
  readonly browserscanner_archive_count: (a: number) => number;
  readonly browserscanner_character_count: (a: number) => number;
  readonly browserscanner_new: () => number;
  readonly browserscanner_report_csv: (a: number, b: number) => void;
  readonly browserscanner_report_json: (a: number, b: number) => void;
  readonly browserscanner_report_markdown: (a: number, b: number) => void;
  readonly browserscanner_reset: (a: number) => void;
  readonly browserscanner_scrub_chunk: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
  readonly browserscanner_scrub_tar: (a: number, b: number, c: number, d: number) => void;
  readonly browserscanner_set_canonical: (a: number, b: number, c: number) => void;
  readonly browserscrub_audit_json: (a: number, b: number) => void;
  readonly browserscrub_audit_markdown: (a: number, b: number) => void;
  readonly browserscrub_item_bits: (a: number) => number;
  readonly browserscrub_take_bytes: (a: number, b: number) => void;
  readonly browserscrub_zdo_count: (a: number) => number;
  readonly browserscrub_zdo_flags: (a: number) => number;
  readonly zstd_compress_bytes: (a: number, b: number, c: number) => void;
  readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
  readonly __wbindgen_malloc: (a: number, b: number) => number;
  readonly __wbindgen_free: (a: number, b: number, c: number) => void;
  readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;
/**
* Instantiates the given `module`, which can either be bytes or
* a precompiled `WebAssembly.Module`.
*
* @param {SyncInitInput} module
*
* @returns {InitOutput}
*/
export function initSync(module: SyncInitInput): InitOutput;

/**
* If `module_or_path` is {RequestInfo} or {URL}, makes a request and
* for everything else, calls `WebAssembly.instantiate` directly.
*
* @param {InitInput | Promise<InitInput>} module_or_path
*
* @returns {Promise<InitOutput>}
*/
export default function __wbg_init (module_or_path?: InitInput | Promise<InitInput>): Promise<InitOutput>;

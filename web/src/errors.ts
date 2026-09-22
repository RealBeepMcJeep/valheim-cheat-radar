/**
 * wasm-bindgen surfaces Rust `Result` errors as thrown *strings*, so `instanceof Error` alone hides
 * the real reason a save was rejected. Keep whatever text exists and only fall back when there is
 * genuinely nothing to show.
 */
export function errorMessage(error: unknown): string {
  if (typeof error === 'string' && error.trim()) return error;
  if (error instanceof Error && error.message) return error.message;
  const text = typeof error === 'object' && error !== null ? String(error) : '';
  return text && text !== '[object Object]' ? text : 'Scanner worker failed.';
}

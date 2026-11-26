import { packOptions, type MarkOptions } from './options';
import {
  errorForStatus,
  STATUS,
  type PochoirModule,
  type PreviewBitmap,
} from './module';

/**
 * The raw WebAssembly ABI, exactly as agreed with the bindings work.
 *
 *   - Synchronous, called from a Web Worker, no wasm-bindgen: plain exports
 *     over linear memory.
 *   - The caller copies bytes in with `pochoir_alloc` and releases them with
 *     `pochoir_free`.
 *   - Both render entry points return a handle to a small result header
 *     `{ status, ptr, len, width, height }`, read through the
 *     `pochoir_result_*` accessors and released with `pochoir_result_free`.
 *   - The mark text is a separate UTF-8 buffer; the options are the packed
 *     little-endian struct in `options.ts`, not JSON.
 */
export interface PochoirExports {
  readonly memory: WebAssembly.Memory;

  pochoir_alloc(len: number): number;
  pochoir_free(ptr: number, len: number): void;

  pochoir_render_pdf(
    imgPtr: number,
    imgLen: number,
    textPtr: number,
    textLen: number,
    optsPtr: number,
    optsLen: number,
  ): number;

  pochoir_render_preview(
    imgPtr: number,
    imgLen: number,
    textPtr: number,
    textLen: number,
    optsPtr: number,
    optsLen: number,
    maxPx: number,
  ): number;

  pochoir_result_status(handle: number): number;
  pochoir_result_ptr(handle: number): number;
  pochoir_result_len(handle: number): number;
  pochoir_result_width(handle: number): number;
  pochoir_result_height(handle: number): number;
  pochoir_result_free(handle: number): void;
}

/** Where the compiled engine is served from, relative to the site root. */
export const WASM_URL = '/pochoir_core.wasm';

/**
 * ADR-0004 Decision 2: the module is fetched with an `integrity` hash and the
 * `Response` handed to `instantiateStreaming`, so a mismatch fails closed.
 * The hash is a build output; until the build emits one this stays undefined
 * and the fetch is plain — which is exactly why `select.ts` does not pick this
 * implementation yet.
 */
export const WASM_INTEGRITY: string | undefined = undefined;

export async function instantiate(
  url: string = WASM_URL,
  integrity: string | undefined = WASM_INTEGRITY,
): Promise<PochoirExports> {
  const response = await fetch(url, integrity ? { integrity } : undefined);
  if (!response.ok) {
    throw new Error(`engine module ${url} responded ${response.status}`);
  }
  // `Content-Type: application/wasm` is required here: instantiateStreaming
  // rejects with a TypeError on anything else.
  const { instance } = await WebAssembly.instantiateStreaming(response, {});
  return instance.exports as unknown as PochoirExports;
}

/** A `PochoirModule` backed by the real engine. */
export class WasmModule implements PochoirModule {
  readonly implementation = 'wasm' as const;

  private constructor(private readonly ex: PochoirExports) {}

  static async load(url?: string, integrity?: string): Promise<WasmModule> {
    return new WasmModule(await instantiate(url, integrity));
  }

  async renderPreview(
    image: Uint8Array,
    text: string,
    options: MarkOptions,
    maxPx: number,
  ): Promise<PreviewBitmap> {
    return this.call(image, text, options, (i, t, o) =>
      this.ex.pochoir_render_preview(i.ptr, i.len, t.ptr, t.len, o.ptr, o.len, maxPx),
    ) as Promise<PreviewBitmap>;
  }

  async renderPdf(
    image: Uint8Array,
    text: string,
    options: MarkOptions,
  ): Promise<Uint8Array> {
    const out = await this.call(image, text, options, (i, t, o) =>
      this.ex.pochoir_render_pdf(i.ptr, i.len, t.ptr, t.len, o.ptr, o.len),
    );
    return (out as { bytes: Uint8Array }).bytes;
  }

  private async call(
    image: Uint8Array,
    text: string,
    options: MarkOptions,
    invoke: (
      img: Span,
      txt: Span,
      opts: Span,
    ) => number,
  ): Promise<PreviewBitmap | { bytes: Uint8Array }> {
    const img = this.copyIn(image);
    const txt = this.copyIn(new TextEncoder().encode(text));
    const opts = this.copyIn(packOptions(options));
    let handle = 0;
    try {
      handle = invoke(img, txt, opts);
      const status = this.ex.pochoir_result_status(handle);
      if (status !== STATUS.OK) throw errorForStatus(status);

      const ptr = this.ex.pochoir_result_ptr(handle);
      const len = this.ex.pochoir_result_len(handle);
      const width = this.ex.pochoir_result_width(handle);
      const height = this.ex.pochoir_result_height(handle);

      // Copy out before any further wasm call: growing the memory detaches the
      // existing ArrayBuffer and any view over it.
      const bytes = new Uint8Array(this.ex.memory.buffer, ptr, len).slice();
      return width > 0 && height > 0
        ? { pixels: new Uint8ClampedArray(bytes.buffer), width, height }
        : { bytes };
    } finally {
      if (handle !== 0) this.ex.pochoir_result_free(handle);
      this.release(opts);
      this.release(txt);
      this.release(img);
    }
  }

  private copyIn(bytes: Uint8Array): Span {
    const ptr = this.ex.pochoir_alloc(bytes.length);
    if (ptr === 0 && bytes.length > 0) {
      throw errorForStatus(STATUS.OUT_OF_MEMORY);
    }
    new Uint8Array(this.ex.memory.buffer, ptr, bytes.length).set(bytes);
    return { ptr, len: bytes.length };
  }

  private release(span: Span): void {
    if (span.len > 0) this.ex.pochoir_free(span.ptr, span.len);
  }
}

interface Span {
  ptr: number;
  len: number;
}

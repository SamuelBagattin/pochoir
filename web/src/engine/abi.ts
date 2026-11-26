import { packOptions, type MarkOptions } from './options';
import {
  EngineError,
  EngineFault,
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
    return this.call(image, text, options, (ip, il, tp, tl, op, ol) =>
      this.ex.pochoir_render_preview(ip, il, tp, tl, op, ol, maxPx),
    ) as Promise<PreviewBitmap>;
  }

  async renderPdf(
    image: Uint8Array,
    text: string,
    options: MarkOptions,
  ): Promise<Uint8Array> {
    const out = await this.call(image, text, options, (ip, il, tp, tl, op, ol) =>
      this.ex.pochoir_render_pdf(ip, il, tp, tl, op, ol),
    );
    return (out as { bytes: Uint8Array }).bytes;
  }

  private async call(
    image: Uint8Array,
    text: string,
    options: MarkOptions,
    invoke: (
      imgPtr: number,
      imgLen: number,
      textPtr: number,
      textLen: number,
      optsPtr: number,
      optsLen: number,
    ) => number,
  ): Promise<PreviewBitmap | { bytes: Uint8Array }> {
    // Packed before the boundary is crossed, so that a rejected option is an
    // ordinary argument error and not mistaken for a fault.
    const packed = packOptions(options);
    const encoded = new TextEncoder().encode(text);

    let img: Span | null = null;
    let txt: Span | null = null;
    let opts: Span | null = null;
    let handle = 0;
    try {
      img = this.copyIn(image);
      txt = this.copyIn(encoded);
      opts = this.copyIn(packed);

      handle = invoke(img.ptr, img.len, txt.ptr, txt.len, opts.ptr, opts.len);
      const status = this.ex.pochoir_result_status(handle);
      if (status !== STATUS.OK) throw errorForStatus(status, this.detailOf(handle));

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
    } catch (error) {
      // An EngineError is a status that came back cleanly; anything else
      // thrown between here and the module is the module misbehaving.
      if (error instanceof EngineError) throw error;
      throw new EngineFault(error);
    } finally {
      // A trapped instance traps again on every call, so the cleanup must not
      // be able to throw over the original error. Leaking a handle in a worker
      // that is about to be discarded costs nothing.
      try {
        if (handle !== 0) this.ex.pochoir_result_free(handle);
        if (opts) this.release(opts);
        if (txt) this.release(txt);
        if (img) this.release(img);
      } catch {
        /* the instance is already gone; the worker goes with it */
      }
    }
  }

  /**
   * Reads `ptr`/`len` as a UTF-8 detail on a failing result, if the engine put
   * anything there. Speculative and bounds-checked: today's contract says
   * nothing about those fields on failure, so an engine that leaves them zero
   * simply yields no detail. It is here so the Rust side can start reporting
   * which character it refused (ADR-0007 Decision 6) without a change on this
   * side as well.
   */
  private detailOf(handle: number): string | undefined {
    const ptr = this.ex.pochoir_result_ptr(handle);
    const len = this.ex.pochoir_result_len(handle);
    const size = this.ex.memory.buffer.byteLength;
    if (ptr === 0 || len === 0 || len > 4096 || ptr + len > size) return undefined;
    try {
      return new TextDecoder('utf-8', { fatal: true }).decode(
        new Uint8Array(this.ex.memory.buffer, ptr, len),
      );
    } catch {
      return undefined;
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

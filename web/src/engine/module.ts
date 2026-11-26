import type { MarkOptions } from './options';

/**
 * The seam. Everything above this interface — the worker, the client, the UI —
 * is written against it and does not know whether the real WebAssembly engine
 * or the browser-side stub is behind it.
 *
 * Swapping the stub for the real module is a one-line change in `select.ts`.
 */
export interface PochoirModule {
  readonly implementation: 'wasm' | 'stub';

  /**
   * Render a preview bitmap, with the longest edge capped at `maxPx`.
   * Returns RGBA8 pixels, row-major, no padding.
   */
  renderPreview(
    image: Uint8Array,
    text: string,
    options: MarkOptions,
    maxPx: number,
  ): Promise<PreviewBitmap>;

  /** Render the final document. */
  renderPdf(image: Uint8Array, text: string, options: MarkOptions): Promise<Uint8Array>;
}

export interface PreviewBitmap {
  pixels: Uint8ClampedArray;
  width: number;
  height: number;
}

/** The status codes the ABI's result header can carry. 0 is success. */
export const STATUS = {
  OK: 0,
  BAD_ARGUMENT: 1,
  DECODE_FAILED: 2,
  UNSUPPORTED_FORMAT: 3,
  OUT_OF_MEMORY: 4,
  INTERNAL: 5,
} as const;

export class EngineError extends Error {
  constructor(
    readonly kind: 'decode' | 'unsupported' | 'oom' | 'argument' | 'internal',
    message: string,
    /**
     * Engine-supplied detail, when there is any — the offending characters for
     * an unsupported-character refusal (ADR-0007 Decision 6), for instance.
     * Data for the UI to interpolate into its own French wording, never a
     * sentence to show the user as it stands.
     */
    readonly detail?: string,
  ) {
    super(message);
    this.name = 'EngineError';
  }
}

/**
 * The engine did not fail — it *faulted*. Thrown when a call into the module
 * raises instead of returning a result header: a trap, which with ADR-0003's
 * `panic = "abort"` poisons the wasm instance for good.
 *
 * The distinction matters and is the whole point of having two classes. An
 * `EngineError` means a status code came back cleanly, through an instance
 * that is still perfectly healthy — an unsupported character, say. Treating
 * that as a fault would throw away a good instance every time somebody typed
 * an emoji. An `EngineFault` means the instance cannot be trusted again and
 * the worker holding it has to go.
 */
export class EngineFault extends Error {
  constructor(readonly reason: unknown) {
    super(`the engine faulted: ${String((reason as Error)?.message ?? reason)}`);
    this.name = 'EngineFault';
  }
}

export function errorForStatus(status: number, detail?: string): EngineError {
  switch (status) {
    case STATUS.BAD_ARGUMENT:
      return new EngineError('argument', 'engine rejected the arguments', detail);
    case STATUS.DECODE_FAILED:
      return new EngineError('decode', 'the input could not be decoded', detail);
    case STATUS.UNSUPPORTED_FORMAT:
      return new EngineError('unsupported', 'the input format is not supported', detail);
    case STATUS.OUT_OF_MEMORY:
      return new EngineError('oom', 'the engine ran out of memory', detail);
    default:
      return new EngineError('internal', `engine failed with status ${status}`, detail);
  }
}

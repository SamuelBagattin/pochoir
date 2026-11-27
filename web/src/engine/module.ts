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
  renderPdf(
    image: Uint8Array,
    text: string,
    options: MarkOptions,
  ): Promise<RenderedDocument>;
}

export interface PreviewBitmap {
  pixels: Uint8ClampedArray;
  width: number;
  height: number;
  warnings: MarkWarnings;
}

export interface RenderedDocument {
  bytes: Uint8Array;
  warnings: MarkWarnings;
}

/**
 * What the engine says about a result it produced anyway.
 *
 * Neither of these is a failure: ADR-0007 Decision 2 and ADR-0010 both draw
 * the mark and tell the caller rather than refusing, because refusing on a
 * document that can be marked imperfectly is worse than marking it and saying
 * so. They arrive over the ABI as a bitmask on the result header, read through
 * `pochoir_result_warnings`.
 *
 * **The browser must show them.** The CLI prints both; a browser that stays
 * silent where the terminal warns lies by omission to the users who matter
 * most — the ones who are not going to run a CLI.
 */
export interface MarkWarnings {
  /**
   * The mark text did not fit at the size asked for and was drawn markedly
   * smaller. ADR-0007 Decision 2.
   */
  shrunk: boolean;
  /**
   * The mark is invisible over more than a tenth of the page. ADR-0010.
   *
   * Which sentence this deserves depends on what was asked for, and the
   * caller knows that without being told: with `colour: 'auto'` it means the
   * page defeats both candidates, because it has light and dark regions and
   * no single colour contrasts with all of it; with a fixed colour it means
   * that colour is close to the page's own tone. That is why the wire carries
   * one bit and not a structure.
   */
  poorContrast: boolean;
}

/** The warning bits of the result header. Mask before reading. */
export const WARNING = {
  /** ADR-0007 Decision 2. */
  MARK_SHRUNK: 1 << 0,
  /** ADR-0010. */
  POOR_CONTRAST: 1 << 1,
} as const;

export const NO_WARNINGS: MarkWarnings = { shrunk: false, poorContrast: false };

/**
 * Decode the warning mask.
 *
 * Unknown bits are ignored rather than treated as an error, so that a later
 * engine can add one without this build reporting a failure on a document it
 * rendered perfectly well.
 */
export function warningsOf(mask: number): MarkWarnings {
  return {
    shrunk: (mask & WARNING.MARK_SHRUNK) !== 0,
    poorContrast: (mask & WARNING.POOR_CONTRAST) !== 0,
  };
}

/** The status codes the ABI's result header can carry. 0 is success. */
export const STATUS = {
  OK: 0,
  BAD_ARGUMENT: 1,
  DECODE_FAILED: 2,
  UNSUPPORTED_FORMAT: 3,
  OUT_OF_MEMORY: 4,
  INTERNAL: 5,
  /** ADR-0007 Decision 2: wrap, shrink, then refuse. Never clip. */
  TEXT_DOES_NOT_FIT: 6,
  /** ADR-0007 Decision 6. Carries every offending character in `detail`. */
  UNSUPPORTED_CHARACTER: 7,
} as const;

export type EngineErrorKind =
  | 'decode'
  | 'unsupported'
  | 'oom'
  | 'argument'
  | 'internal'
  | 'does-not-fit'
  | 'unsupported-character';

export class EngineError extends Error {
  constructor(
    readonly kind: EngineErrorKind,
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
    case STATUS.TEXT_DOES_NOT_FIT:
      return new EngineError('does-not-fit', 'the mark will not fit the page', detail);
    case STATUS.UNSUPPORTED_CHARACTER:
      return new EngineError(
        'unsupported-character',
        'the mark contains characters the font cannot draw',
        detail,
      );
    default:
      return new EngineError('internal', `engine failed with status ${status}`, detail);
  }
}

/// <reference lib="webworker" />
import { EngineError, type PochoirModule } from './module';
import type { Failure, Request, Response } from './protocol';
import { loadModule } from './select';

/**
 * The engine runs here and nowhere else. Two reasons, both load-bearing:
 *
 *   1. A 12 megapixel photograph must not freeze the page while it renders.
 *   2. ADR-0003 builds the engine with `panic = "abort"`, so a panic traps the
 *      wasm instance for good. The containment is a worker the main thread can
 *      `terminate()` and replace, which also reclaims the high-water mark of
 *      linear memory. That is why this file holds no state the client cannot
 *      rebuild: being killed is a normal event here, not an error path.
 */

let modulePromise: Promise<PochoirModule> | null = null;
/** The last image handed to us, kept so a slider drag does not re-upload it. */
let image: Uint8Array | null = null;

function engine(): Promise<PochoirModule> {
  modulePromise ??= loadModule();
  return modulePromise;
}

self.onmessage = async (event: MessageEvent<Request>) => {
  const request = event.data;
  try {
    const reply = await handle(request);
    (self as DedicatedWorkerGlobalScope).postMessage(reply, transfersOf(reply));
  } catch (error) {
    const failure: Failure =
      error instanceof EngineError
        ? { kind: error.kind, message: error.message }
        : { kind: 'internal', message: String((error as Error)?.message ?? error) };
    (self as DedicatedWorkerGlobalScope).postMessage({
      id: request.id,
      ok: false,
      failure,
    } satisfies Response);
  }
};

async function handle(request: Request): Promise<Response> {
  switch (request.kind) {
    case 'load': {
      image = new Uint8Array(request.image);
      const module = await engine();
      return {
        id: request.id,
        ok: true,
        kind: 'load',
        implementation: module.implementation,
      };
    }
    case 'preview': {
      const bytes = requireImage();
      const module = await engine();
      const bitmap = await module.renderPreview(
        bytes,
        request.text,
        request.options,
        request.maxPx,
      );
      return {
        id: request.id,
        ok: true,
        kind: 'preview',
        pixels: bitmap.pixels.buffer as ArrayBuffer,
        width: bitmap.width,
        height: bitmap.height,
      };
    }
    case 'pdf': {
      const bytes = requireImage();
      const module = await engine();
      const pdf = await module.renderPdf(bytes, request.text, request.options);
      return { id: request.id, ok: true, kind: 'pdf', bytes: pdf.buffer as ArrayBuffer };
    }
  }
}

function requireImage(): Uint8Array {
  if (!image) throw new EngineError('argument', 'no image loaded in this worker');
  return image;
}

function transfersOf(reply: Response): Transferable[] {
  if (!reply.ok) return [];
  if (reply.kind === 'preview') return [reply.pixels];
  if (reply.kind === 'pdf') return [reply.bytes];
  return [];
}

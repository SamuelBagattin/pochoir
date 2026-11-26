import type { PochoirModule } from './module';
import { StubModule } from './module-stub';

/**
 * THE ONE FILE TO CHANGE when the Rust/WebAssembly engine lands.
 *
 * Today it returns the stub, because `crates/pochoir-core` has no wasm
 * bindings yet. When it does:
 *
 *   import { WasmModule } from './abi';
 *   export async function loadModule() { return WasmModule.load(); }
 *
 * Nothing else in `web/` needs to know. The worker, the client and every
 * component are written against `PochoirModule`.
 */
export async function loadModule(): Promise<PochoirModule> {
  return new StubModule();
}

/** Shown in the UI so nobody mistakes the demonstration for the product. */
export const USING_STUB = true;

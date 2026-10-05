/// <reference lib="webworker" />
import { seed } from "./examples";
import type { Request, Reply } from "./types";

// Bindings are generated into public/wasm by build:wasm and served locally.
interface Engine {
  run(query: string): string;
  free(): void;
}
let engine: Engine | undefined;
let EngineConstructor: (new () => Engine) | undefined;
self.onmessage = async (event: MessageEvent<Request>) => {
  const { id, action, query } = event.data;
  const started = performance.now();
  try {
    if (action === "init") {
      const url = new URL("/wasm/graphfusion.js", self.location.origin).href;
      const bindings = await import(/* @vite-ignore */ url);
      const compressed = typeof DecompressionStream !== "undefined";
      const response = await fetch(
        new URL(
          `/wasm/graphfusion_bg.wasm${compressed ? ".gz" : ""}`,
          self.location.origin,
        ),
      );
      if (!response.ok)
        throw new Error(`Engine download failed (${response.status}).`);
      const total = response.headers.get("content-encoding")
        ? 0
        : Number(response.headers.get("content-length"));
      let received = 0;
      const reader = response.body?.getReader();
      const chunks: Uint8Array[] = [];
      if (!reader)
        throw new Error("This browser cannot stream the engine download.");
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        chunks.push(value);
        received += value.byteLength;
        self.postMessage({
          id,
          progress: total
            ? `Loading engine… ${Math.min(100, Math.round((received / total) * 100))}%`
            : `Loading engine… ${(received / 1048576).toFixed(1)} MB`,
        } satisfies Reply);
      }
      const bytes = new Uint8Array(received);
      let offset = 0;
      for (const chunk of chunks) {
        bytes.set(chunk, offset);
        offset += chunk.byteLength;
      }
      chunks.length = 0;
      self.postMessage({ id, progress: "Starting engine…" } satisfies Reply);
      // Some hosts set Content-Encoding on .gz assets, so fetch has already
      // decoded them. Detect the WASM magic bytes to avoid decoding twice.
      const isWasm =
        bytes[0] === 0 &&
        bytes[1] === 97 &&
        bytes[2] === 115 &&
        bytes[3] === 109;
      const wasmBytes =
        compressed && !isWasm
          ? new Uint8Array(
              await new Response(
                new Blob([bytes])
                  .stream()
                  .pipeThrough(new DecompressionStream("gzip")),
              ).arrayBuffer(),
            )
          : bytes;
      await bindings.default({ module_or_path: wasmBytes });
      EngineConstructor = bindings.Engine;
      engine = new bindings.Engine();
      self.postMessage({
        id,
        progress: "Loading sample graph…",
      } satisfies Reply);
      engine!.run(seed);
    } else if (action === "reset") {
      if (!EngineConstructor) throw new Error("The engine is not ready.");
      engine?.free();
      engine = new EngineConstructor();
      engine.run(seed);
    } else {
      if (!engine)
        throw new Error(
          "The engine is not ready. Reset the session to try again.",
        );
      const result = JSON.parse(engine.run(query || ""));
      self.postMessage({
        id,
        result,
        elapsed: performance.now() - started,
      } satisfies Reply);
      return;
    }
    self.postMessage({
      id,
      elapsed: performance.now() - started,
    } satisfies Reply);
  } catch (error) {
    const fatal =
      action === "init" || error instanceof WebAssembly.RuntimeError;
    self.postMessage({
      id,
      error: error instanceof Error ? error.message : String(error),
      fatal,
    } satisfies Reply);
  }
};

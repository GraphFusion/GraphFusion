---
title: "Build the browser playground"
description: "Build, test and publish the real WebAssembly engine and graph explorer."
sidebar:
  order: 4
---

`crates/graphfusion-wasm` exposes an in-memory `Engine` backed by the same GraphFusion crate as the CLI. A dedicated worker drives DataFusion on a current-thread Tokio executor, with one target partition and filesystem spilling disabled. Native CLI dependencies and browser clock/randomness dependencies are selected by target.

The optional core `visualization` feature captures labels, properties and edge endpoints from each query's own source snapshot, before its transaction lease is released. It uses indexed selected-row providers, including for persistent native tests. Result serialization preserves graph identities as strings and represents integers outside JavaScript's exact range as strings.

## Toolchain

Install the project's Rust toolchain, a C compiler with a WebAssembly backend, the matching wasm-bindgen CLI and Binaryen 133:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked
```

On macOS, `brew install llvm binaryen` supplies the compiler and optimizer. The build script locates Homebrew LLVM without changing your shell configuration. Check that `wasm-opt --version` reports `wasm-opt version 133`.

On Ubuntu, install `clang` and download the matching archive from the [official Binaryen 133 release](https://github.com/WebAssembly/binaryen/releases/tag/version_133), then add its `bin/` directory to `PATH`. CI verifies the release checksum. The Ubuntu 24.04 `binaryen` package is version 108; its optimized engine fails during initialization when growing the external-reference table. The build script rejects mismatched optimizer versions before compiling.

## Build and test

```sh
cd website
npm ci
npm run build:wasm
npm run check
npm run build
npx playwright install chromium
npm run test:browser
```

The first Rust build can take several minutes. `build:wasm` uses the dedicated `wasm` Cargo profile and Binaryen to optimize the bundle, then generates a gzip asset for the initial download. The worker decodes it locally; browsers without decompression streams use the raw WASM asset. Generated assets under `public/wasm/` are ignored by Git and included in Astro's static output. `build` rejects missing engine assets instead of producing a broken Playground.

Use `npm run dev` after building the engine, and open `/playground/`. The browser tests use the static build and cover result graphs, paths, properties, plans, mutations, transactions, recovery after query errors and offline execution. Core projection tests cover persistent snapshots, graph identity collisions and large integers.

## Publishing

The documentation CI builds the engine before the site. Before publishing this version, update the separate root-site publishing workflow to install the tools above, allow 45 minutes, and run `npm run build:wasm` before `npm run build`. Its current documentation-only build does not generate the engine. Copying only `website/` is insufficient because the engine build needs the Rust workspace and lockfile. Serve the generated `.wasm` with `application/wasm` and standard HTTP compression. No API server, database service or cross-origin isolation headers are required.

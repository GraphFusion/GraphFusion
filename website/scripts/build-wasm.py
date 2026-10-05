"""Build the real Rust engine and its version-matched browser bindings."""
from pathlib import Path
import os
import shutil
import subprocess
import sys
import gzip

ROOT = Path(__file__).resolve().parents[2]
VERSION = '0.2.128'
BINARYEN_VERSION = '133'
env = os.environ.copy()
if sys.platform == 'darwin' and not env.get('CC_wasm32_unknown_unknown'):
    # Apple's system clang has no WebAssembly backend. Homebrew LLVM does.
    for prefix in ['/opt/homebrew/opt/llvm/bin', '/usr/local/opt/llvm/bin']:
        if Path(prefix, 'clang').exists():
            env['CC_wasm32_unknown_unknown'] = str(Path(prefix, 'clang'))
            env['AR_wasm32_unknown_unknown'] = str(Path(prefix, 'llvm-ar'))
            break
if not shutil.which('wasm-bindgen'):
    sys.exit(f'Install bindings: cargo install wasm-bindgen-cli --version {VERSION} --locked')
installed = subprocess.check_output(['wasm-bindgen', '--version'], text=True).strip()
if installed != f'wasm-bindgen {VERSION}':
    sys.exit(f'Expected wasm-bindgen {VERSION}; found {installed}')
if not shutil.which('wasm-opt'):
    sys.exit('Install Binaryen (wasm-opt) to optimize the browser engine. See development/browser-playground.')
optimizer = subprocess.check_output(['wasm-opt', '--version'], text=True).strip()
if optimizer != f'wasm-opt version {BINARYEN_VERSION}':
    sys.exit(f'Expected Binaryen {BINARYEN_VERSION}; found {optimizer}. Older optimizers can break the browser engine. See development/browser-playground.')
subprocess.run(['cargo', 'build', '--locked', '-p', 'graphfusion-wasm',
                '--target', 'wasm32-unknown-unknown', '--profile', 'wasm'], cwd=ROOT, env=env, check=True)
output = ROOT / 'website/public/wasm'
output.mkdir(parents=True, exist_ok=True)
subprocess.run(['wasm-bindgen', str(ROOT / 'target/wasm32-unknown-unknown/wasm/graphfusion_wasm.wasm'),
                '--target', 'web', '--out-dir', str(output), '--out-name', 'graphfusion'], check=True)
wasm = output / 'graphfusion_bg.wasm'
optimized = output / 'graphfusion_bg.optimized.wasm'
subprocess.run(['wasm-opt', str(wasm), '-Oz', '--strip-debug', '--strip-producers', '-o', str(optimized)], check=True)
optimized.replace(wasm)
compressed = output / 'graphfusion_bg.wasm.gz'
compressed.write_bytes(gzip.compress(wasm.read_bytes(), compresslevel=9, mtime=0))
print(f'Optimized engine: {wasm.stat().st_size / 1048576:.1f} MiB')
print(f'Compressed download: {compressed.stat().st_size / 1048576:.1f} MiB')
print(f'Browser engine: {output / "graphfusion_bg.wasm"}')

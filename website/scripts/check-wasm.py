"""Never publish a Playground whose real engine assets are missing."""
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
for name in ['graphfusion.js', 'graphfusion_bg.wasm', 'graphfusion_bg.wasm.gz']:
    if not (ROOT / 'public/wasm' / name).is_file():
        raise SystemExit('Browser engine missing. Run npm run build:wasm before npm run build.')

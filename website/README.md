# GraphFusion documentation

Astro Starlight source for https://graphfusion.github.io/GraphFusion/.

```sh
npm ci
npm run dev
npm run check
npm run build
npm run test:examples
```

Use Node.js 22.12+ and Python 3. Example checks also build the Rust CLI and parser.
On environments that prohibit writing user preferences, set
`ASTRO_TELEMETRY_DISABLED=1` when invoking Astro.

See [the authoring guide](src/content/docs/development/website.md) for content,
validation and deployment conventions. `npm run test:browser` checks the built
site's navigation, search and mobile layout with Playwright Chromium; install its
browser with `npx playwright install chromium` first.

# Vendored front-end libraries

preact, htm and @preact/signals, as published on npm — the `*.module.js` ESM
builds, unmodified.

They are here rather than behind an import map pointing at a CDN because the
dashboards have to load with no internet: reading happens offline, and an
overlay that works beside a dashboard that will not open is not a product.

Each file imports the others by bare specifier, so the import map in every
`spa.html` is what resolves them. Updating one means updating the version in
that map's comment too.

| file | package | version |
|---|---|---|
| `preact.module.js` | preact | 10.25.4 |
| `preact-hooks.module.js` | preact/hooks | 10.25.4 |
| `htm.module.js` | htm | 3.1.1 |
| `htm-preact.module.js` | htm/preact | 3.1.1 |
| `preact-signals.module.js` | @preact/signals | 1.3.1 |
| `preact-signals-core.module.js` | @preact/signals-core | 1.8.0 |

All MIT.

# Dashboard (npm) dependency licenses

Generated 2026-07-15 via `npx license-checker --production --json` in `dashboard/`.

**Scope:** production dependency tree of the Next.js dashboard only.
Rust tree: see [dependency-licenses.md](./dependency-licenses.md).

## Summary

| License | Packages |
|---------|----------|
| MIT | 60 |
| Apache-2.0 | 8 |
| ISC | 2 |
| 0BSD | 1 |
| BSD-3-Clause | 1 |
| CC-BY-4.0 | 1 |
| LGPL-3.0-or-later | 1 |
| MIT-0 | 1 |
| Python-2.0 | 1 |
| UNLICENSED | 1 |
| Unlicense | 1 |

## Notes

- **LGPL-3.0-or-later:** `@img/sharp-libvips-*` (optional native image pipeline via `sharp`).
  Typically platform-specific optional deps for image optimization; not part of the
  Rust validation gateway. Confirm redistribution model for hosted dashboard builds.
- **UNLICENSED:** usually the private app package itself (`dashboard`), not a third-party lib.
- No AGPL/GPL (non-LGPL) packages observed in this production scan.

## Packages by license

### 0BSD

- `tslib@2.8.1`

### Apache-2.0

- `@img/sharp-darwin-arm64@0.34.5`
- `@playwright/test@1.61.1`
- `@swc/helpers@0.5.15`
- `baseline-browser-mapping@2.10.18`
- `detect-libc@2.1.2`
- `playwright-core@1.61.1`
- `playwright@1.61.1`
- `sharp@0.34.5`

### BSD-3-Clause

- `source-map-js@1.2.1`

### CC-BY-4.0

- `caniuse-lite@1.0.30001788`

### ISC

- `picocolors@1.1.1`
- `semver@7.7.4`

### LGPL-3.0-or-later

- `@img/sharp-libvips-darwin-arm64@1.2.4`

### MIT

- `@floating-ui/core@1.8.0`
- `@floating-ui/dom@1.8.0`
- `@floating-ui/react-dom@2.1.9`
- `@floating-ui/utils@0.2.12`
- `@img/colour@1.1.0`
- `@next/env@16.2.10`
- `@next/swc-darwin-arm64@16.2.10`
- `@radix-ui/primitive@1.1.5`
- `@radix-ui/react-arrow@1.1.11`
- `@radix-ui/react-compose-refs@1.1.3`
- `@radix-ui/react-context@1.2.0`
- `@radix-ui/react-dismissable-layer@1.1.15`
- `@radix-ui/react-id@1.1.2`
- `@radix-ui/react-popper@1.3.3`
- `@radix-ui/react-portal@1.1.13`
- `@radix-ui/react-presence@1.1.7`
- `@radix-ui/react-primitive@2.1.7`
- `@radix-ui/react-slot@1.3.0`
- `@radix-ui/react-tooltip@1.2.12`
- `@radix-ui/react-use-callback-ref@1.1.2`
- `@radix-ui/react-use-controllable-state@1.2.3`
- `@radix-ui/react-use-effect-event@0.0.3`
- `@radix-ui/react-use-layout-effect@1.1.2`
- `@radix-ui/react-use-rect@1.1.2`
- `@radix-ui/react-use-size@1.1.2`
- `@radix-ui/react-visually-hidden@1.2.7`
- `@radix-ui/rect@1.1.2`
- `@stablelib/base64@1.0.1`
- `@supabase/auth-js@2.110.3`
- `@supabase/functions-js@2.110.3`
- `@supabase/phoenix@0.4.4`
- `@supabase/postgrest-js@2.110.3`
- `@supabase/realtime-js@2.110.3`
- `@supabase/ssr@0.12.1`
- `@supabase/storage-js@2.110.3`
- `@supabase/supabase-js@2.110.3`
- `@types/node@26.1.1`
- `@types/react-dom@19.2.3`
- `@types/react@19.2.17`
- `client-only@0.0.1`
- `clsx@2.1.1`
- `cookie@1.1.1`
- `csstype@3.2.3`
- `dequal@2.0.3`
- `fsevents@2.3.2`
- `iceberg-js@0.8.1`
- `js-yaml@5.2.1`
- `nanoid@3.3.16`
- `next@16.2.10`
- `postcss@8.5.19`
- `react-dom@19.2.7`
- `react@19.2.7`
- `resend@6.17.2`
- `scheduler@0.27.0`
- `standardwebhooks@1.0.0`
- `stripe@22.3.1`
- `styled-jsx@5.1.6`
- `swr@2.4.2`
- `undici-types@8.3.0`
- `use-sync-external-store@1.6.0`

### MIT-0

- `postal-mime@2.7.4`

### Python-2.0

- `argparse@2.0.1`

### Unlicense

- `fast-sha256@1.3.0`

### UNLICENSED

- `contractgate-dashboard@0.1.0`

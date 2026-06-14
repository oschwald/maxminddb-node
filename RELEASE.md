# Release Notes

This package loads a native N-API addon at runtime. A publishable npm tarball
must therefore include at least one built `.node` file that matches the target
platform.

## Current Artifact Shape

`index.js` probes for platform-specific native bindings first, then falls back
to `index.node`:

- Linux: `index.linux-*-*.node`
- macOS: `index.darwin-*.node`
- Windows: `index.win32-*-*.node`
- FreeBSD: `index.freebsd-x64.node`
- Generic fallback: `index.node`

`package.json` includes `*.node` files in the tarball, but it does not build a
native addon during package installation. Build the addon before packing or
publishing:

```sh
npm ci
npm run build
npm pack --dry-run
```

The dry run should show `index.js`, `index.d.ts`, `package.json`, `README.md`,
and one or more `.node` files.

## Publishing Strategy

The current repository is ready for a single-platform tarball built on the
release machine. A multi-platform npm release should use one of these models:

- Publish separate optional dependency packages per target triple, each
  containing one `index.<triple>.node`.
- Publish one package containing every supported `index.<triple>.node`.
- Publish source only and add an install-time build path.

The first option is the preferred long-term shape because it keeps install size
small while preserving install-time reliability. The loader already supports
platform-specific filenames, so the remaining work is CI release automation and
optional dependency package metadata.

## Release Checklist

1. Build in release mode with `npm run build`.
2. Run `cargo fmt -- --check`.
3. Run `cargo check`.
4. Run `npm test`.
5. Run `npm run typecheck`.
6. Run `npm pack --dry-run` and verify the tarball contents.
7. Run `npm run bench -- --compare-node-maxmind` when benchmark databases are
   available.

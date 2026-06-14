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

## Trusted Publishing

Publishing is configured through `.github/workflows/publish.yml` and npm trusted
publishing. The workflow does not use an `NPM_TOKEN`; it grants GitHub Actions
`id-token: write` permission so npm can authenticate the publish through OIDC.

Before the first publish, configure the package on npmjs.com:

- Publisher: GitHub Actions
- Organization or user: the GitHub owner of this repository
- Repository: the GitHub repository name
- Workflow filename: `publish.yml`
- Allowed action: `npm publish`

The workflow uses Node 24 and upgrades npm before publishing so the npm CLI
meets trusted publishing requirements. It runs the same validation as CI, builds
the native addon, verifies the package with `npm pack --dry-run`, and then runs
`npm publish`.

This publishes the Linux x64 GNU native artifact produced by the GitHub-hosted
Ubuntu runner. Add per-platform packages or a multi-platform artifact assembly
step before using this workflow for broad platform support.

## Release Checklist

1. Configure npm trusted publishing for `.github/workflows/publish.yml`.
2. Build in release mode with `npm run build`.
3. Run `cargo fmt -- --check`.
4. Run `cargo check`.
5. Run `npm test`.
6. Run `npm run typecheck`.
7. Run `npm pack --dry-run` and verify the tarball contents.
8. Run `npm run bench -- --compare-node-maxmind` when benchmark databases are
   available.
9. Create a GitHub release to trigger trusted publishing.

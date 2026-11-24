# Release Notes

This package loads a native N-API addon at runtime. A publishable npm tarball
must therefore include built `.node` files for the platforms it supports.

## Current Artifact Shape

`index.js` probes for platform-specific native bindings first, then falls back
to `index.node`:

- Linux: `index.linux-*-*.node`
- macOS: `index.darwin-*.node`
- Windows: `index.win32-*-*.node`
- FreeBSD: `index.freebsd-x64.node`
- Generic fallback: `index.node`

`package.json` includes `*.node` files in the tarball, but it does not build a
native addon during package installation. For local packaging checks, build the
addon before packing:

```sh
npm ci
npm run build
npm pack --dry-run
```

The dry run should show `index.js`, `index.d.ts`, `package.json`, `README.md`,
and one or more `.node` files.

The trusted publishing workflow assembles one npm package containing these
prebuilt native modules:

- Linux x64 GNU: `index.linux-x64-gnu.node`
- Linux arm64 GNU: `index.linux-arm64-gnu.node`
- Linux x64 musl: `index.linux-x64-musl.node`
- Linux arm64 musl: `index.linux-arm64-musl.node`
- macOS x64: `index.darwin-x64.node`
- macOS arm64: `index.darwin-arm64.node`
- Windows x64 MSVC: `index.win32-x64-msvc.node`
- Windows arm64 MSVC: `index.win32-arm64-msvc.node`

The Linux GNU modules are built with the napi-rs cross toolchain
(`napi build --use-napi-cross`), which links against glibc 2.17. The workflow
fails if a GNU module needs a newer glibc symbol than that, and it loads each
GNU module with Node.js in an AlmaLinux 8 (glibc 2.28) container, the oldest
glibc that official Node.js 20, 22, and 24 Linux builds support. The musl
modules are built and smoke-tested inside a `node:24-alpine` container on a
runner of the same architecture. On musl systems the loader tries the musl
module before the GNU one.

The workflow does not currently build Linux armv7, Linux ppc64/s390x/riscv64,
Windows ia32, or FreeBSD artifacts. The loader supports platform-specific
filenames for several of those targets, so adding them is a release automation
task rather than a runtime API change.

Pull requests that change `.github/workflows/publish.yml`, the Rust sources,
the loader, or the dependency manifests run the `build-binaries` job so that
release build problems, such as a dependency that needs a newer glibc, show up
before a release. The `publish` job does not run for pull requests.

## Publishing Strategy

The repository currently publishes `@oschwald/maxminddb` as one package
containing every supported `index.<triple>.node` file. This keeps installation
simple and avoids install-time native builds. If the tarball becomes too large,
the next packaging shape to consider is separate optional dependency packages
per target triple:

- Publish separate optional dependency packages per target triple, each
  containing one `index.<triple>.node`.

That model keeps install size small while preserving install-time reliability.
Publishing source only and building during package installation is intentionally
not the default release strategy.

The package is scoped and public. `package.json` sets `publishConfig.access` to
`public`, and local bootstrap publishes should also pass `--access public`.

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
meets trusted publishing requirements. It builds native modules on hosted
Linux, macOS, and Windows runners, smoke-tests that each native module loads on
its build runner, downloads those artifacts into the package root, runs the same
validation as CI, and verifies the package with `npm pack --dry-run`. GitHub
release events then run `npm publish`. Manual `workflow_dispatch` runs are
validation-only unless the `publish` input is set to `true`.

## Release Checklist

1. Configure npm trusted publishing for `.github/workflows/publish.yml`.
2. Add a top entry to `CHANGELOG.md` with today's date.
3. Run `npm run release`.
4. Run `npm run bench -- --compare-node-maxmind` when benchmark databases are
   available.
5. Verify the `build-binaries` matrix completed for every supported target
   before the `publish` job runs `npm publish`.

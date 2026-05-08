# Postgres Viewer

A fast, read-only PostgreSQL/Supabase desktop viewer built with Rust, Tauri, and React.

## Run

```sh
npm install
npm run tauri:dev
```

Paste a PostgreSQL connection URL, including Supabase connection strings such as:

```text
postgresql://postgres:password@host:5432/postgres?sslmode=require
```

## Build

```sh
npm run tauri -- build
```

The release app bundle is written to:

```text
src-tauri/target/release/bundle/macos/Postgres Viewer.app
```

## Releases

CI runs on every push to `main` and every pull request. Pushing a `v*` tag creates a GitHub Release with a universal macOS app ZIP:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The GitHub Release artifact is unsigned by default. Mac App Store distribution requires Apple Developer certificates, provisioning profiles, App Sandbox entitlements, and App Store Connect metadata. See [docs/APP_STORE.md](docs/APP_STORE.md).

## Current Scope

- Connects with PostgreSQL URLs, including Supabase URLs that require TLS.
- Stores saved connection URLs in the macOS Keychain.
- Lists schemas, tables, columns, and indexes.
- Fetches table rows in pages.
- Runs one read-only SQL statement at a time.
- Shows query/table results in a virtualized grid with sorting, column resizing, wrapped cells, and row detail tabs.
- Does not edit data.

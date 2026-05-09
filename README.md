# Postgres Viewer

A fast PostgreSQL/Supabase desktop viewer built with Rust, Tauri, and React.

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
git tag v0.2.0
git push origin v0.2.0
```

The GitHub Release artifact is unsigned by default. Mac App Store distribution requires Apple Developer certificates, provisioning profiles, App Sandbox entitlements, and App Store Connect metadata. See [docs/APP_STORE.md](docs/APP_STORE.md).

## Current Scope

- Connects with PostgreSQL URLs, including Supabase URLs that require TLS.
- Stores saved connection URLs and the optional OpenAI API key in the macOS Keychain.
- Lets saved connections be renamed so the sidebar does not only show the database name.
- Lists schemas, tables, columns, and indexes.
- Caches schema/table details and recent table pages for fast table switching, then refreshes in the background.
- Fetches table rows in pages with sorting and column resizing.
- Runs read-only SQL, plus confirmed `INSERT`, `UPDATE`, and `DELETE` statements.
- Supports grid edits, inserts, and deletes when a table has a primary key or non-null unique key.
- Generates SQL from a natural-language prompt with `gpt-5.5` after an API key is added in Settings.
- Shows query/table results in a virtualized grid with sorting, column resizing, wrapped cells, and row detail tabs.
- Does not perform schema admin operations such as creating or dropping tables.

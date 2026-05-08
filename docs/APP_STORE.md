# Mac App Store Release

This app can be submitted to the Mac App Store, but Apple account work cannot be automated from the repository alone.

## Required Apple Setup

- Enroll in the Apple Developer Program.
- Create an App Store Connect app record.
- Use bundle identifier `com.postgresviewer.desktop`.
- Create a matching explicit App ID.
- Create a `Mac App Store Connect` provisioning profile for that App ID.
- Install a `Mac App Distribution` certificate and a `Mac Installer Distribution` certificate on the build Mac.
- Prepare App Store metadata, screenshots, privacy answers, and encryption/export compliance answers.

## Current Local Build

```sh
npm ci
npm run tauri -- build --bundles app
```

The unsigned app bundle is created under:

```text
src-tauri/target/release/bundle/macos/Postgres Viewer.app
```

## GitHub Release Build

The `Release` workflow builds a universal macOS app ZIP when a `v*` tag is pushed:

```sh
git tag v0.1.0
git push origin v0.1.0
```

That workflow intentionally creates an unsigned ZIP until Apple signing credentials are configured.

## App Store Build Notes

For a Mac App Store upload, the app needs App Sandbox entitlements, a provisioning profile embedded at `Contents/embedded.provisionprofile`, an App Store compatible `.pkg`, and Apple signing identities. Tauri's App Store build path uses a separate config such as `src-tauri/tauri.appstore.conf.json` with:

```json
{
  "bundle": {
    "category": "DeveloperTool",
    "macOS": {
      "entitlements": "./Entitlements.mas.plist",
      "infoPlist": "./Info.mas.plist",
      "files": {
        "embedded.provisionprofile": "../private/profile.provisionprofile"
      }
    }
  }
}
```

Minimum entitlements for this app:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>com.apple.security.app-sandbox</key>
  <true/>
  <key>com.apple.security.network.client</key>
  <true/>
  <key>com.apple.application-identifier</key>
  <string>TEAM_ID.com.postgresviewer.desktop</string>
  <key>com.apple.developer.team-identifier</key>
  <string>TEAM_ID</string>
</dict>
</plist>
```

`Info.mas.plist` should include the encryption declaration that matches the App Store Connect answer. This app uses TLS to connect to user-provided database hosts, so verify the correct export compliance answer in your Apple account.

After a signed `.app` is produced, package it with a `Mac Installer Distribution` certificate:

```sh
xcrun productbuild \
  --sign "3rd Party Mac Developer Installer: Your Name (TEAM_ID)" \
  --component "src-tauri/target/universal-apple-darwin/release/bundle/macos/Postgres Viewer.app" \
  /Applications \
  "Postgres Viewer.pkg"
```

Upload the package with Transporter, Xcode Organizer, or App Store Connect tooling.

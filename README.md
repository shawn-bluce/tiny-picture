# Tiny Picture

[![CI](https://github.com/shawn-bluce/tiny-picture/actions/workflows/ci.yml/badge.svg)](https://github.com/shawn-bluce/tiny-picture/actions/workflows/ci.yml)

A batch image compressor for macOS, built with Rust and [Slint](https://slint.dev) (cupertino style).
Drop images or folders onto the window and compress them all at once.

## Download

- **Latest release**: [Releases](https://github.com/shawn-bluce/tiny-picture/releases/latest)
- **Nightly build** (latest `master`, may contain unreleased changes):
  [Tiny-Picture-nightly-arm64.zip](https://github.com/shawn-bluce/tiny-picture/releases/download/nightly/Tiny-Picture-nightly-arm64.zip)

Unzip it and move **Tiny Picture.app** to Applications. The app is ad-hoc signed and not
notarized, so macOS blocks it the first time it is opened. To open it anyway, either:

- open **System Settings → Privacy & Security** and click **Open Anyway** next to the
  message about Tiny Picture, or
- run `xattr -dr com.apple.quarantine "/Applications/Tiny Picture.app"`.

## Requirements

- Supported: Apple silicon Macs running macOS 26 or later. Intel Macs and older
  macOS versions are not blocked from building or running, but are not supported.
- To build: Rust 1.88 or later and the Xcode Command Line Tools (for the bundled C libraries).

## Usage

Drag images or folders onto the window. Folders are scanned recursively for
`jpg`, `jpeg`, `png`, `webp` and `gif` files (by extension only). Dropping more
files appends them to the list; **Clear** empties it.

The list shows each file's status (waiting, compressing, compressed, skipped,
failed; hover the icon for details) and its original size, compressed size,
saved size (KB, 1 KB = 1000 bytes) and saved percentage.

- **Compression**: Lossless, Ultra, High, Medium, Tiny
- **Powerful**: use all CPU cores instead of half of them
- **Replace**: overwrite originals; when off, writes `tiny_<name>` next to each original
- **Convert to**: Original, JPG, PNG

## Compression

| Level    | JPEG / WebP | PNG                                  |
|----------|-------------|--------------------------------------|
| Lossless | lossless optimization (JPEG) / lossless WebP | lossless optimization |
| Ultra    | quality 90  | lossless optimization                |
| High     | quality 80  | 256-color palette, dithered          |
| Medium   | quality 70  | 128-color palette, dithered          |
| Tiny     | quality 55  | 64-color palette, dithered           |

- Lossless JPEG optimization works like `jpegtran -optimize -progressive -copy all`: the DCT coefficients are re-encoded with optimized Huffman tables, so pixels and metadata stay unchanged (`src/jpegtran.rs`).
- Libraries: [mozjpeg](https://crates.io/crates/mozjpeg) (JPEG), [oxipng](https://crates.io/crates/oxipng) + [exoquant](https://crates.io/crates/exoquant) (PNG), [webp](https://crates.io/crates/webp) (libwebp), [image](https://crates.io/crates/image) (decoding).
- Files that are already compressed well are skipped and left untouched (same format only):
  lossy levels must save at least 5%, lossless levels must save anything at all.
  Format conversions are never skipped.
- Compress only processes files that are waiting or failed before, so dropping more
  files and compressing again never re-compresses earlier results.
- EXIF orientation is applied and ICC color profiles are kept for JPEG and PNG.
- Converting to JPG blends transparency onto white. Converting keeps the source file.
- GIF can only be converted (first frame), not re-compressed.

## Development

```bash
cargo run                                   # debug build
cargo test                                  # unit tests
cargo fmt --check                           # formatting (checked by CI)
cargo clippy --all-targets -- -D warnings   # lints (checked by CI)
./scripts/bundle.sh                         # build target/bundle/Tiny Picture.app and a zip
```

Live-preview the UI with the Slint VS Code extension, or run `cargo install slint-viewer`
and then `slint-viewer --style cupertino ui/app-window.slint`.

### Project layout

```
├── .github/workflows/    # CI: checks, tests, bundle, nightly and release publishing
├── assets/               # App icon (placeholder)
├── build.rs              # Compiles .slint files with the cupertino style
├── scripts/bundle.sh     # Builds the .app bundle and release zip
├── src/
│   ├── main.rs           # Entry point: drag and drop, UI callbacks, background jobs
│   ├── scan.rs           # Finds images in dropped files/folders (by extension)
│   ├── options.rs        # Compression level, output format, output path rules
│   ├── compress.rs       # Codecs, skip rules and the worker pool
│   └── jpegtran.rs       # Lossless JPEG optimization (mozjpeg FFI)
└── ui/
    ├── app-window.slint   # Main window: drop zone, settings form, action bar
    ├── theme.slint        # Design tokens (colors, spacing, type, motion)
    ├── widgets.slint      # Small shared widgets (icon, spinner, info badge, ...)
    ├── level-slider.slint # Discrete slider with clickable stop labels
    ├── popup-button.slint # macOS-style pop-up button
    ├── file-row.slint     # File list header and rows
    └── icons/             # SVG icons
```

## Versioning and releases

Tiny Picture follows [Semantic Versioning](https://semver.org). The `version` in
`Cargo.toml` is the single source of truth: it becomes the app version shown in Finder
(`CFBundleShortVersionString`), and the build number (`CFBundleVersion`) is the number of
commits on `master`. While the version is `0.x`, minor versions add features and patch
versions fix bugs. Notable changes are recorded in [CHANGELOG.md](CHANGELOG.md).

CI runs on every push and pull request. Every push to `master` replaces the
[nightly build](https://github.com/shawn-bluce/tiny-picture/releases/tag/nightly).

To publish a release:

1. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD` and start a new
   empty `## [Unreleased]` section above it.
2. Set `version = "X.Y.Z"` in `Cargo.toml` and run `cargo check` to update `Cargo.lock`.
3. Commit, then tag and push: `git tag vX.Y.Z && git push origin master vX.Y.Z`.

CI refuses tags that don't match the `Cargo.toml` version or have no CHANGELOG section,
and publishes the release with that CHANGELOG section as release notes.

## License

[MIT](LICENSE)

Tiny Picture's user interface is built with [Slint](https://slint.dev), used under the
[Slint Royalty-free Desktop, Mobile, and Web Applications License](https://slint.dev/terms-and-conditions#royalty-free).

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)

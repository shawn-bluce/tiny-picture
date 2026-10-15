# Changelog

All notable changes to this project are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Batch compression of JPG, PNG, WebP and GIF files dropped onto the window
  (folders are scanned recursively).
- Five compression levels: Lossless, Ultra, High, Medium and Tiny.
- Lossless JPEG optimization (optimized Huffman tables and progressive scans,
  pixels and metadata unchanged), lossless PNG optimization, palette-based
  lossy PNG, and WebP re-encoding.
- Convert to JPG or PNG.
- Replace originals, or save prefixed copies (`tiny_<name>`) next to them.
- Powerful mode that uses all CPU cores.
- Files that are already compressed well are skipped and left untouched.
- File list with status, original size, compressed size, saved size and
  saved percentage.

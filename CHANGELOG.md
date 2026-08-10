# Changelog

All notable changes to this project are documented in this file.

## [0.1.4]

### Added
- `view` command now renders RFC content through an external Markdown renderer
  when the `RFC_VIEWER` environment variable is set (content is piped on stdin),
  mirroring how `edit` uses `$EDITOR` (RFC-0009).
- `--raw` flag for `view` to force raw Markdown output, ignoring `RFC_VIEWER`.

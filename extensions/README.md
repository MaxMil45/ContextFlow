# Extensions

- `browser/`: Manifest V3 extension for Chrome and Edge. It tells the ContextFlow
  helper (`contextflow-browser-host`, over Native Messaging) which tabs are open.
  It never reads cookies, history or page content, and ignores private windows.

VS Code, Visual Studio, JetBrains IDEs and File Explorer need no extension:
`crates/cf-detect` works out their folders from window titles.

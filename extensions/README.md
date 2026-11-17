# Extensions

- `browser/`: Manifest V3 extension for Chrome and Edge. It tells the ContextFlow
  helper (`contextflow-browser-host`, over Native Messaging) which tabs are open.
  It never reads cookies, history or page content, and ignores private windows.
  The one thing it receives is a request to pin tabs that ContextFlow has just
  reopened. Its test runs without a browser: `node browser/test/pinning.test.js`.

VS Code, Visual Studio, JetBrains IDEs, File Explorer and Windows Terminal need
no extension: `crates/cf-detect` works out their folders from window titles,
the Windows Shell and the terminal's own processes.

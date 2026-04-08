+++
title = "Troubleshooting"
weight = 10
+++

# Troubleshooting

## Log Files

Kimün writes a log file on every run. In release builds only warnings and errors are recorded, keeping the file small. Debug builds log everything.

### Log file location

| Platform | Path |
|----------|------|
| macOS | `~/Library/Application Support/kimun/kimun.log` |
| Linux | `$XDG_DATA_HOME/kimun/kimun.log` (or `~/.local/share/kimun/kimun.log` if `XDG_DATA_HOME` is not set) |
| Windows | `%APPDATA%\kimun\kimun.log` |

The file is created automatically the first time Kimün runs. It is appended to across restarts — there is no rotation, so you can delete it manually at any time if it grows large.

### Reading the log

Open the file in any text editor. Each line is one event:

```
2026-04-08T10:23:01.456Z  WARN kimun: could not open last workspace: path not found
2026-04-08T10:23:05.789Z ERROR kimun: fatal error: broken pipe
```

### Crash reports

If Kimün crashes, the panic message and a full stack trace are appended to the same log file. Look for a line starting with `[PANIC]`:

```
[PANIC] panicked at 'index out of bounds: the len is 3 but the index is 5', src/...
   0: kimun::app::...
   1: ...
```

When reporting a bug, please include the relevant section of `kimun.log`.

### Fallback location

If Kimün cannot write to the platform directory (for example, because the home directory is unavailable), it falls back to the system temporary directory:

| Platform | Fallback |
|----------|---------|
| macOS / Linux | `/tmp/kimun.log` (or wherever `$TMPDIR` points) |
| Windows | `%TEMP%\kimun.log` |

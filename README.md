# Notes App
_Damn I need a better name_

A notes app. Focus on simplicity, but powerful on searchability.

## Building
The app uses gxhash that uses some specific hardware acceleration features. If when compiling you get an error, make sure you add the flag `RUSTFLAGS="-C target-cpu=native"`

On Windows (PowerShell):
```powershell
$env:RUSTFLAGS="-C target-cpu=native"
```

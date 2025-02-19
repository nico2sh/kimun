#!/bin/bash

rm -rf ./kimun.app
mkdir -p ./kimun.app/Contents/MacOS
cargo build --release --manifest-path ./desktop/Cargo.toml
cp ./desktop/target/release/kimun_desktop ./kimun.app/Contents/MacOS/kimun
touch ./kimun.app/Info.plist
echo "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
  <key>CFBundleExecutable</key>
  <string>kimun</string>
  <key>CFBundleIdentifier</key>
  <string>com.nico2sh.kimun</string>
</dict>
</plist>
" > ./kimun.app/Info.plist

#!/bin/sh
set -eu
npm run build -- --ci --bundles app
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
app='target/release/bundle/macos/Slop Mail.app'
ditto "$app" "$stage/Slop Mail.app"
ln -s /Applications "$stage/Applications"
version=$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$app/Contents/Info.plist")
arch=$(rustc -vV | sed -n 's/^host: \([^-]*\)-.*/\1/p')
output="target/release/bundle/dmg/Slop Mail_${version}_${arch}.dmg"
mkdir -p target/release/bundle/dmg
hdiutil create -ov -volname 'Slop Mail' -srcfolder "$stage" -format UDZO "$output"
hdiutil verify "$output"

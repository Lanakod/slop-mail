#!/bin/sh
set -eu
npm run build -- --ci --bundles app
bundle_dir=target/release/bundle/dmg
mkdir -p "$bundle_dir"
stage=$(mktemp -d "$bundle_dir/.package.XXXXXX")
trap 'rm -rf "$stage"' EXIT
mkdir "$stage/contents"
app='target/release/bundle/macos/Slop Mail.app'
ditto "$app" "$stage/contents/Slop Mail.app"
ln -s /Applications "$stage/contents/Applications"
version=$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$app/Contents/Info.plist")
arch=$(rustc -vV | sed -n 's/^host: \([^-]*\)-.*/\1/p')
output="$bundle_dir/Slop Mail_${version}_${arch}.dmg"
hdiutil create -volname 'Slop Mail' -srcfolder "$stage/contents" -format UDZO "$stage/created.dmg"
# diskimages-helper can still hold the inode used by create. Verify an independent
# copy and atomically publish that exact file only after its checksum passes.
cp "$stage/created.dmg" "$stage/release.dmg"
hdiutil verify "$stage/release.dmg"
mv -f "$stage/release.dmg" "$output"

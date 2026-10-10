#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
out="$here/assets"
esbuild="$repo/node_modules/.pnpm/esbuild@0.28.1/node_modules/esbuild/bin/esbuild"
rm -rf "$out"
mkdir -p "$out/plugins"
printf 'export * from "zod";\n' > "$out/zod-entry.mjs"
"$esbuild" "$out/zod-entry.mjs" --bundle --format=esm --platform=neutral --minify \
  --outfile="$out/zod.mjs" --log-level=warning \
  --alias:zod="$repo/packages/plugin-sdk/node_modules/zod"
rm "$out/zod-entry.mjs"
cp "$repo/packages/plugin-sdk/dist/index.js" "$out/plugin-sdk.mjs"
cp "$repo/packages/plugin-sdk/dist/provider-bridge-worker-entry.mjs" "$out/provider-bridge-worker-entry.mjs"
for dir in "$repo"/plugins/*/.bundled-runtime; do
  name="$(basename "$(dirname "$dir")")"
  cp -r "$dir" "$out/plugins/$name"
done
ls -la "$out" "$out/plugins"

#!/bin/bash
# Build the FormaJS client with esbuild
set -e
cd "$(dirname "$0")"

# Install dependencies if needed
if [ ! -d "node_modules/@getforma" ]; then
  npm install
fi

# Ensure dist/client exists
mkdir -p dist/client

# Version is injected at build time so the UI can never drift from the release.
# Read it from the published meta package (the root package.json is private and
# carries no version); this is the same file the release workflow checks
# against the git tag.
KMD_VERSION=$(node -p "require('./npm/kmd/package.json').version")

# Bundle with esbuild
npx esbuild client/app.ts \
  --bundle \
  --outfile=dist/client/app.js \
  --format=esm \
  --target=es2022 \
  --define:__KMD_VERSION__="\"$KMD_VERSION\"" \
  --minify

# Copy static assets
cp client/index.html dist/client/index.html
cp client/styles/dev.css dist/client/dev.css
cp node_modules/@xterm/xterm/css/xterm.css dist/client/xterm.css

# Copy vendored libraries (mermaid.js for offline diagram rendering)
mkdir -p dist/client/vendor
cp client/vendor/mermaid.min.js dist/client/vendor/mermaid.min.js

echo "Client build complete -> dist/client/"

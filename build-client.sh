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

# Bundle with esbuild
npx esbuild client/app.ts \
  --bundle \
  --outfile=dist/client/app.js \
  --format=esm \
  --target=es2022 \
  --minify

# Copy static assets
cp client/index.html dist/client/index.html
cp client/styles/dev.css dist/client/dev.css
cp node_modules/@xterm/xterm/css/xterm.css dist/client/xterm.css

# Copy vendored libraries (mermaid.js for offline diagram rendering)
mkdir -p dist/client/vendor
cp client/vendor/mermaid.min.js dist/client/vendor/mermaid.min.js

echo "Client build complete -> dist/client/"

#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."

rm -rf dist

echo "Building RepoRocket with Cargo..."
cargo build --release

mkdir -p dist
cp target/release/reporocket dist/RepoRocket

if [ -d img ]; then
    cp -r img dist/img
else
    echo "img folder not found"
fi

echo "Build completed successfully."

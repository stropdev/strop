#!/bin/sh
# The demo's project: demos/fixtures/proj copied to /tmp/proj as a git
# repository with one commit, so the tape has something to change, stage
# and commit. demo.tape runs this before it starts recording.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
rm -rf /tmp/proj
cp -r "$here/../fixtures/proj" /tmp/proj
cd /tmp/proj
git init -q -b main
git config user.name "Demo"
git config user.email "demo@example.invalid"
git add .
git commit -qm "Sharpen a blade through three stones"

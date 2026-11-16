#!/bin/bash
set -euo pipefail
source "$(dirname "$0")/macos-signing.sh"
# A function intercepts only security discovery; production always uses its absolute path.
function /usr/bin/security() { printf '%s\n' "$FIXTURE"; }
unset SKYLIS_SIGNING_IDENTITY
FIXTURE='  0 valid identities found'
[[ "$(skylis_signing_identity 2>/dev/null)" == - ]]
FIXTURE='  1) AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA "Apple Development: Fixture"
  1 valid identities found'
[[ "$(skylis_signing_identity)" == AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA ]]
FIXTURE="$FIXTURE
  2) BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB \"Developer ID Application: Fixture\""
if skylis_signing_identity >/dev/null 2>&1; then exit 1; fi
SKYLIS_SIGNING_IDENTITY=explicit-identity
[[ "$(skylis_signing_identity)" == explicit-identity ]]
SKYLIS_SIGNING_IDENTITY=-
[[ "$(skylis_signing_identity)" == - ]]
SKYLIS_SIGNING_IDENTITY=''
if skylis_signing_identity >/dev/null 2>&1; then exit 1; fi
# Explicit values go unchanged to codesign; build uses set -e, so a rejection is fatal.
SKYLIS_SIGNING_IDENTITY=nonexistent
[[ "$(skylis_signing_identity)" == nonexistent ]]
echo 'Signing selection fixtures passed.'

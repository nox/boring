#!/usr/bin/env bash

set -euo pipefail

git apply -v --whitespace=fix ../../../../rpk-patch/include/*/*.patch ../../../../rpk-patch/ssl/*.patch
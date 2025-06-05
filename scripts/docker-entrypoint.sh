#!/bin/bash

set -e

if [[ -n "$1" ]]; then
    SUFFIX="-$1"
else
    SUFFIX=""
fi

make "bundle${SUFFIX}"
install -o "$(stat -c '%u' /output)" -g "$(stat -c '%g' /output)" \
        -m 0644 \
        -t "/output" "output/builtin-actors${SUFFIX}.car"

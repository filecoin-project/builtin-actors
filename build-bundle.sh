#!/bin/bash

set -e

bundle=$(cargo build 2>&1 | grep "warning: bundle=" | cut -d = -f 2)
cp -v "$bundle" output/builtin-actors.car

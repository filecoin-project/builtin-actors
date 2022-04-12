#!/bin/bash

set -e

bundle=output/builtin-actors.car

shasum -a 256 $bundle > $bundle.sha256sum
cat $bundle.sha256sum

curl -k -X POST -F "data=@${bundle};type=application/octet-stream;filename=\"${bundle}\"" -H "Authorization: Bearer $ESTUARY_TOKEN" -H "Content-Type: multipart/form-data" https://shuttle-4.estuary.tech/content/add > output/upload.json
cat output/upload.json

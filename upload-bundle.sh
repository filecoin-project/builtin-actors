#!/bin/bash

set -e

bundle=output/builtin-actors.car
curl -k -X POST -F "data=@${bundle};type=application/octet-stream;filename=\"${bundle}\"" -H "Authorization: Bearer $ESTUARY_TOKEN" -H "Content-Type: multipart/form-data" https://shuttle-4.estuary.tech/content/add > output/upload.json
cat output/upload.json

#!/usr/bin/env bash

set -e

die() {
    echo "$1"
    exit 1
}

# make sure we have a token set, api requests won't work otherwise
if [ -z "$GITHUB_TOKEN" ]; then
    die "no GITHUB_TOKEN"
fi

# make sure we have a release tag set
if [ -z "$GITHUB_SHA" ]; then
    die "no GITHUB_SHA"
fi

# make sure we have a target set
if [ -z "$BUILD_FIL_NETWORK" ]; then
    die "no BUILD_FIL_NETWORK"
fi


release_input=builtin-actors.car
release_target=builtin-actors-${BUILD_FIL_NETWORK}.car
release_target_hash=builtin-actors-${BUILD_FIL_NETWORK}.sha256
release_file=output/$release_target
release_file_hash=output/$relesae_target_hash
release_tag="${GITHUB_SHA:0:16}"

# prepare artifacts
pushd output
mv $release_input $release_target
shasum -a 256 $release_target > $release_target_hash
popd

# prepare release
ORG="filecoin-project"
REPO="builtin-actors"

# see if the release already exists by tag
__release_response=`
  curl \
   --header "Authorization: token $GITHUB_TOKEN" \
   "https://api.github.com/repos/$ORG/$REPO/releases/tags/$release_tag"
`
__release_id=`echo $__release_response | jq '.id'`
if [ "$__release_id" = "null" ]; then
    echo "creating release $release_tag"
    release_data="{
      \"tag_name\": \"$release_tag\",
      \"target_commitish\": \"$GITHUB_SHA\",
      \"name\": \"$release_tag\",
      \"body\": \"\"
      }"

    __release_response=`
      curl \
       --request POST \
       --header "Authorization: token $GITHUB_TOKEN" \
       --header "Content-Type: application/json" \
       --data "$release_data" \
       "https://api.github.com/repos/$ORG/$REPO/releases"
     `
else
    echo "release $release_tag already exists"
fi

__release_upload_url=`echo $__release_response | jq -r '.upload_url' | cut -d'{' -f1`

echo "uploading $release_target"
curl \
 --request POST \
 --header "Authorization: token $GITHUB_TOKEN" \
 --header "Content-Type: application/octet-stream" \
 --data-binary "@$release_file" \
 "$__release_upload_url?name=$release_target"

echo "uploading $release_target_hash"
curl \
 --request POST \
 --header "Authorization: token $GITHUB_TOKEN" \
 --header "Content-Type: application/octet-stream" \
 --data-binary "@$release_file_hash" \
 "$__release_upload_url?name=$release_target_hash"

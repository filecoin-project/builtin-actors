#!/bin/bash
# Checks and updates bundle size file, printing delta between the previous and current bundle size.

# Check if the user provided both arguments
if [ $# -ne 2 ]; then
  echo "Usage: ./update-bundle-size.sh <bundle-size-path> <bundle-path>"
  exit 1
fi

# Check if paths exist
if [[ ! -f $1 || ! -f $2 ]]; then
  echo "Invalid arguments. Please check that the files exist."
  exit 1
fi

bundle_size_path=$1
bundle_path=$2

# Grab the current bundle size
size_old=$(head -n 1 "$bundle_size_path")

# Update bundle size
wc -c < "$bundle_path" > "$bundle_size_path"

# Grab the new bundle size
size_new=$(head -n 1 "$bundle_size_path")

# Calculate the difference
diff=$((size_new - size_old))

# Print stats
echo "Old bundle size: $size_old"
echo "New bundle size: $size_new"
echo "Delta:           $diff byte(s)"

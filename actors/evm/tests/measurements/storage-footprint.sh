#!/usr/bin/env bash

set -e

# Name of the scenario to plot.
NAME=$1

if [ -z "$NAME" ]; then
  echo "usage: storage-footprint.sh <scenario-name>"
  exit 1
fi

# Separate the two series (the current max) with newlines for gnuplot
# Ignoring warnings from gnuplot because sometimes we only have 1 series.

rm -f $NAME.dat

for S in 1 2; do
  cat $NAME.jsonline \
    | jq -r "select(.series == $S) | [.series, .i, .stats.get_count, .stats.get_bytes, .stats.put_count, .stats.put_bytes] | @tsv" \
    >> $NAME.dat
  echo $'\n' >> $NAME.dat
done

gnuplot \
  -e "scenario='$(echo $NAME | tr _ - )'" \
  -e "filein='$NAME.dat'" \
  -e "fileout='$NAME.png'" \
  storage-footprint.plt \
  2>/dev/null

rm $NAME.dat

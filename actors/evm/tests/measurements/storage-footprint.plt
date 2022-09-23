set term png size 1200,800;
set output fileout;
set y2tics;
set ytics nomirror;
set ylabel "Bytes";
set y2label "Count";
set key outside;

# Show two plots in 2 rows, 1 column;
set multiplot layout 2, 1 ;

set title scenario . ": GET Stats";
plot for [i=0:*] filein index i using 2:4 with lines  axis x1y1 title sprintf("GET bytes (%d)", i+1), \
     for [i=0:*] filein index i using 2:3 with points axis x1y2 title sprintf("GET count (%d)", i+1)

set title scenario . ": PUT Stats";
plot for [i=0:*] filein index i using 2:6 with lines  axis x1y1 title sprintf("PUT bytes (%d)", i+1), \
     for [i=0:*] filein index i using 2:5 with points axis x1y2 title sprintf("PUT count (%d)", i+1)

unset multiplot

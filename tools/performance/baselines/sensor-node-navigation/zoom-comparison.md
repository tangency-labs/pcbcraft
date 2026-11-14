# Continuous zoom comparison

Thirty samples per zoom direction/document on one development host. Values measure CPU UI service cost, not presentation latency. The baseline is the initial navigation fix; the candidate adds canvas text raster reuse.

| Document | Direction | Before median ms | After median ms | After max ms |
| --- | --- | ---: | ---: | ---: |
| Sensor Node | zoom_in.ui | 0.567 | 0.552 | 0.856 |
| Sensor Node | zoom_out.ui | 0.548 | 0.549 | 0.705 |
| Power | zoom_in.ui | 2.316 | 0.613 | 0.969 |
| Power | zoom_out.ui | 2.256 | 0.588 | 0.891 |
| Microcontroller | zoom_in.ui | 5.733 | 0.891 | 2.568 |
| Microcontroller | zoom_out.ui | 5.623 | 0.856 | 2.050 |
| Sensors | zoom_in.ui | 4.442 | 0.775 | 1.990 |
| Sensors | zoom_out.ui | 4.352 | 0.745 | 0.916 |
| PCB | zoom_in.ui | 4.781 | 1.651 | 1.880 |
| PCB | zoom_out.ui | 4.921 | 1.641 | 1.687 |

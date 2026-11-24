# Fabrication drill files

**Manufacture › Fabrication › Outputs** and `pcb.export_fabrication` keep
plated pad drills and through vias in `-PTH.drl`. If the board contains circular
non-plated footprint holes, `-NPTH.drl` contains just those holes and the
fabrication archive's manifest lists both files. A board without those holes
has no `-NPTH.drl` member. Non-plated openings do not create copper, mask or
paste features.

Both files measure from the selected placement origin, with Y reversed from
board coordinates and lengths written in millimeters to three decimal places.
The non-plated tool size is the finished diameter, with no plating allowance.
A non-plated diameter that rounds to zero stops export instead of writing a
zero-size tool. This is a file precision limit, not a fabrication rule.

Each drill file's header names its content with Gerber X2 file attributes in
XNC standard comments (`; #@! TF.…`): `FileFunction,Plated,1,N,PTH` or
`FileFunction,NonPlated,1,N,NPTH`, where N is the board's copper layer count,
plus `GenerationSoftware` and a deterministic `SameCoordinates` GUID derived
from the resolved export frame (origin, axis convention and unit), matching the
Gerber layers of that export. Readers that ignore attributes see ordinary
comments. Tool and hole attributes, slots and rout paths are not written yet.

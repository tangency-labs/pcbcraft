#!/usr/bin/env python3
"""Write the synthetic STEP (ISO 10303-21) fixtures of crates/geometry/tests/fixtures/step.

Each file is built entity by entity from the structures of ISO 10303-42 (geometry and
topology), -41 (products and units) and -43/-44 (representations and assembly placement), with
no CAD system involved. expected.json beside them records, for each file, how many bodies and
body placements it holds and the bounding box of the placed bodies in nanometres. The boxes are
worked out here from the dimensions each file is written with, not measured by a kernel, so the
same file gates any kernel.

Run from the repository root; the output is the same on every run:

    python3 scripts/generate-step-fixtures.py
"""

import json
import math
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "crates" / "geometry" / "tests" / "fixtures" / "step"

NM_PER_MM = 1_000_000
MM_PER_UNIT = {"mm": 1.0, "inch": 25.4, "m": 1000.0}
SCHEMAS = {
    "ap203": ("CONFIG_CONTROL_DESIGN", "config_control_design", "config control design", 1994),
    "ap214": ("AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }", "automotive_design", "automotive design", 2000),
    "ap242": (
        "AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF { 1 0 10303 442 1 1 4 }",
        "ap242_managed_model_based_3d_engineering",
        "managed model based 3d engineering",
        2014,
    ),
}
X, Y, Z = (1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)


def real(x):
    """A STEP REAL: always with a decimal point, exponent after it."""
    if x == 0:
        return "0."
    text = repr(float(x))
    mantissa, _, exponent = text.partition("e")
    if "." not in mantissa:
        mantissa += "."
    if mantissa.endswith(".0"):
        mantissa = mantissa[:-1]
    return mantissa + ("E" + exponent if exponent else "")


def sub(a, b):
    return tuple(p - q for p, q in zip(a, b))


def add(a, b):
    return tuple(p + q for p, q in zip(a, b))


def scale(a, k):
    return tuple(p * k for p in a)


def cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def dot(a, b):
    return sum(p * q for p, q in zip(a, b))


class Writer:
    """The data section of one file, one entity at a time."""

    def __init__(self):
        self.lines = []

    def add(self, text):
        self.lines.append(text)
        return len(self.lines)

    def refs(self, ids):
        return "(" + ",".join(f"#{i}" for i in ids) + ")"

    def point(self, p):
        return self.add(f"CARTESIAN_POINT('',({','.join(real(v) for v in p)}))")

    def direction(self, d):
        return self.add(f"DIRECTION('',({','.join(real(v) for v in d)}))")

    def axis(self, origin, z=Z, x=X):
        o, zd, xd = self.point(origin), self.direction(z), self.direction(x)
        return self.add(f"AXIS2_PLACEMENT_3D('',#{o},#{zd},#{xd})")

    def vertex(self, p):
        return self.add(f"VERTEX_POINT('',#{self.point(p)})")

    def line_edge(self, va, pa, vb, pb):
        d = sub(pb, pa)
        length = dot(d, d) ** 0.5
        o, dd = self.point(pa), self.direction(scale(d, 1.0 / length))
        v = self.add(f"VECTOR('',#{dd},{real(length)})")
        line = self.add(f"LINE('',#{o},#{v})")
        return self.add(f"EDGE_CURVE('',#{va},#{vb},#{line},.T.)")

    def circle_edge(self, vertex, centre, z, x, r):
        """A whole circle about `z` from its point along `x`, closed at `vertex`."""
        c = self.add(f"CIRCLE('',#{self.axis(centre, z, x)},{real(r)})")
        return self.add(f"EDGE_CURVE('',#{vertex},#{vertex},#{c},.T.)")

    def oriented(self, edge, forward):
        return self.add(f"ORIENTED_EDGE('',*,*,#{edge},{'.T.' if forward else '.F.'})")

    def bound(self, uses, outer):
        loop = self.add(f"EDGE_LOOP('',{self.refs(self.oriented(e, f) for e, f in uses)})")
        return self.add(f"{'FACE_OUTER_BOUND' if outer else 'FACE_BOUND'}('',#{loop},.T.)")

    def face(self, bounds, surface, same_sense):
        return self.add(f"ADVANCED_FACE('',{self.refs(bounds)},#{surface},{'.T.' if same_sense else '.F.'})")

    def plane(self, origin, z, x):
        return self.add(f"PLANE('',#{self.axis(origin, z, x)})")

    def cylinder_surface(self, origin, z, x, r):
        return self.add(f"CYLINDRICAL_SURFACE('',#{self.axis(origin, z, x)},{real(r)})")

    def solid(self, name, faces):
        shell = self.add(f"CLOSED_SHELL('',{self.refs(faces)})")
        return self.add(f"MANIFOLD_SOLID_BREP('{name}',#{shell})")


def hole_side(w, centre, r, z0, z1):
    """The wall of a vertical through hole of radius `r` from z0 to z1, facing the axis, and its
    bottom and top circles."""
    v0, v1 = w.vertex((centre[0] + r, centre[1], z0)), w.vertex((centre[0] + r, centre[1], z1))
    bottom = w.circle_edge(v0, (centre[0], centre[1], z0), Z, X, r)
    top = w.circle_edge(v1, (centre[0], centre[1], z1), Z, X, r)
    seam = w.line_edge(v0, (centre[0] + r, centre[1], z0), v1, (centre[0] + r, centre[1], z1))
    surface = w.cylinder_surface((centre[0], centre[1], z0), Z, X, r)
    # About the normal towards the axis: up the seam, back along the top, down the seam, round
    # the bottom.
    wall = w.face([w.bound([(seam, True), (top, True), (seam, False), (bottom, False)], True)], surface, False)
    return wall, bottom, top


def block(w, name, lo, hi, holes=()):
    """An axis-aligned block from `lo` to `hi`, with vertical through holes (centre, radius)."""
    corner = lambda i: tuple(hi[a] if i >> a & 1 else lo[a] for a in range(3))
    vertices = [w.vertex(corner(i)) for i in range(8)]
    edges = {}

    def edge(i, j):
        key = (min(i, j), max(i, j))
        if key not in edges:
            a, b = key
            edges[key] = w.line_edge(vertices[a], corner(a), vertices[b], corner(b))
        return edges[key], i < j

    walls = [hole_side(w, c, r, lo[2], hi[2]) for c, r in holes]
    faces = []
    for a in range(3):
        for side in (0, 1):
            normal = tuple((1.0 if side else -1.0) if k == a else 0.0 for k in range(3))
            u = tuple(1.0 if k == (a + 1) % 3 else 0.0 for k in range(3))
            v = cross(normal, u)
            ids = [i for i in range(8) if (i >> a & 1) == side]
            centre = scale(tuple(map(sum, zip(*(corner(i) for i in ids)))), 0.25)
            # Counter-clockwise about the outward normal.
            ids.sort(key=lambda i: math.atan2(dot(sub(corner(i), centre), v), dot(sub(corner(i), centre), u)))
            bounds = [w.bound([edge(ids[k], ids[(k + 1) % 4]) for k in range(4)], True)]
            if a == 2:
                # A hole runs clockwise about the face's outward normal.
                for _, bottom, top in walls:
                    bounds.append(w.bound([(top, False)] if side else [(bottom, True)], False))
            plane = w.plane(corner(ids[0]), normal, u)
            faces.append(w.face(bounds, plane, True))
    faces.extend(wall for wall, _, _ in walls)
    return w.solid(name, faces)


def cylinder(w, name, origin, axis, ref, r, h, seam=True):
    """A cylinder of radius `r` from `origin` along the unit `axis` for `h`. With `seam` its side
    is one face bounded by its two circles and a seam line used twice; without, by the two circles
    alone (a band), as some systems write it."""
    top_centre = add(origin, scale(axis, h))
    p0, p1 = add(origin, scale(ref, r)), add(top_centre, scale(ref, r))
    v0, v1 = w.vertex(p0), w.vertex(p1)
    bottom = w.circle_edge(v0, origin, axis, ref, r)
    top = w.circle_edge(v1, top_centre, axis, ref, r)
    surface = w.cylinder_surface(origin, axis, ref, r)
    if seam:
        s = w.line_edge(v0, p0, v1, p1)
        side = w.face([w.bound([(bottom, True), (s, True), (top, False), (s, False)], True)], surface, True)
    else:
        side = w.face([w.bound([(bottom, True)], True), w.bound([(top, False)], False)], surface, True)
    base = w.face([w.bound([(bottom, False)], True)], w.plane(origin, axis, ref), False)
    cap = w.face([w.bound([(top, True)], True)], w.plane(top_centre, axis, ref), True)
    return w.solid(name, [side, base, cap])


class StepFile:
    """A file of parts and assemblies in one schema and one length unit."""

    def __init__(self, schema, unit):
        self.w = Writer()
        self.schema = schema
        _, protocol, context, year = SCHEMAS[schema]
        w = self.w
        app = w.add(f"APPLICATION_CONTEXT('{context}')")
        w.add(f"APPLICATION_PROTOCOL_DEFINITION('international standard','{protocol}',{year},#{app})")
        self.product_context = w.add(f"PRODUCT_CONTEXT('',#{app},'mechanical')")
        self.definition_context = w.add(f"PRODUCT_DEFINITION_CONTEXT('part definition',#{app},'design')")
        mm = w.add("(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.))")
        if unit == "mm":
            length = mm
        elif unit == "m":
            length = w.add("(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT($,.METRE.))")
        else:
            measure = w.add(f"LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#{mm})")
            exponents = w.add("DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.)")
            length = w.add(f"(CONVERSION_BASED_UNIT('INCH',#{measure})LENGTH_UNIT()NAMED_UNIT(#{exponents}))")
        angle = w.add("(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.))")
        solid_angle = w.add("(NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)SOLID_ANGLE_UNIT())")
        # One nanometre, in the file's unit.
        uncertainty = w.add(
            f"UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE({real(1e-6 / MM_PER_UNIT[unit])}),#{length},"
            "'distance_accuracy_value','confusion accuracy')"
        )
        self.context = w.add(
            f"(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#{uncertainty}))"
            f"GLOBAL_UNIT_ASSIGNED_CONTEXT((#{length},#{angle},#{solid_angle}))REPRESENTATION_CONTEXT('',''))"
        )

    def product(self, name):
        """A product with one definition: (definition, product-definition shape)."""
        w = self.w
        product = w.add(f"PRODUCT('{name}','{name}','',(#{self.product_context}))")
        formation = w.add(f"PRODUCT_DEFINITION_FORMATION('','',#{product})")
        definition = w.add(f"PRODUCT_DEFINITION('design','',#{formation},#{self.definition_context})")
        shape = w.add(f"PRODUCT_DEFINITION_SHAPE('','',#{definition})")
        return definition, shape

    def part(self, name, solids, linked=False):
        """A part holding `solids`, its body representation the shape of its definition, or with
        `linked` related to a plain shape representation that is (the form assemblies use).
        Returns (definition, shape representation, origin placement)."""
        w = self.w
        definition, shape = self.product(name)
        origin = w.axis((0.0, 0.0, 0.0))
        body = w.add(f"ADVANCED_BREP_SHAPE_REPRESENTATION('{name}',{w.refs(solids + [origin])},#{self.context})")
        if not linked:
            w.add(f"SHAPE_DEFINITION_REPRESENTATION(#{shape},#{body})")
            return definition, body, origin
        rep = w.add(f"SHAPE_REPRESENTATION('{name}',(#{origin}),#{self.context})")
        w.add(f"SHAPE_DEFINITION_REPRESENTATION(#{shape},#{rep})")
        w.add(f"SHAPE_REPRESENTATION_RELATIONSHIP('','',#{body},#{rep})")
        return definition, rep, origin

    def assembly(self, name, uses):
        """An assembly placing parts: each use is (part, name, origin, z axis, x axis), the part
        as `part` returns it, the frame in the assembly's coordinates."""
        w = self.w
        definition, shape = self.product(name)
        origin = w.axis((0.0, 0.0, 0.0))
        frames = [w.axis(o, z, x) for _, _, o, z, x in uses]
        rep = w.add(f"SHAPE_REPRESENTATION('{name}',{w.refs([origin] + frames)},#{self.context})")
        w.add(f"SHAPE_DEFINITION_REPRESENTATION(#{shape},#{rep})")
        for n, ((child, child_rep, child_origin), use_name, *_), frame in zip(range(1, len(uses) + 1), uses, frames):
            placement = w.add(f"ITEM_DEFINED_TRANSFORMATION('','',#{child_origin},#{frame})")
            relation = w.add(
                f"(REPRESENTATION_RELATIONSHIP('','',#{child_rep},#{rep})"
                f"REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#{placement})SHAPE_REPRESENTATION_RELATIONSHIP())"
            )
            usage = w.add(f"NEXT_ASSEMBLY_USAGE_OCCURRENCE('{n}','{use_name}','',#{definition},#{child},$)")
            usage_shape = w.add(f"PRODUCT_DEFINITION_SHAPE('','',#{usage})")
            w.add(f"CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#{relation},#{usage_shape})")
        return definition

    def text(self, file_name):
        schema = SCHEMAS[self.schema][0]
        head = [
            "ISO-10303-21;",
            "HEADER;",
            "FILE_DESCRIPTION(('PCBcraft STEP qualification fixture'),'2;1');",
            f"FILE_NAME('{file_name}','',(''),(''),'','','');",
            f"FILE_SCHEMA(('{schema}'));",
            "ENDSEC;",
            "DATA;",
        ]
        data = [f"#{n}={line};" for n, line in enumerate(self.w.lines, 1)]
        return "\n".join(head + data + ["ENDSEC;", "END-ISO-10303-21;", ""])


def box_of(lo, hi, unit="mm"):
    k = MM_PER_UNIT[unit] * NM_PER_MM
    return [round(v * k) for v in lo], [round(v * k) for v in hi]


def placed(lo, hi, origin, z, x):
    """The box of the block `lo`..`hi` placed by the frame (origin, z, x)."""
    y = cross(z, x)
    corners = [tuple(hi[a] if i >> a & 1 else lo[a] for a in range(3)) for i in range(8)]
    world = [add(origin, add(add(scale(x, c[0]), scale(y, c[1])), scale(z, c[2]))) for c in corners]
    return tuple(min(p[a] for p in world) for a in range(3)), tuple(max(p[a] for p in world) for a in range(3))


def union(*boxes):
    return tuple(min(b[0][a] for b in boxes) for a in range(3)), tuple(max(b[1][a] for b in boxes) for a in range(3))


def fixtures():
    """(file name, StepFile, bodies, placements, (least, greatest corner) in the file's unit, unit)."""
    out = []

    f = StepFile("ap214", "mm")
    lo, hi = (-5.0, 2.5, 0.0), (25.0, 22.5, 10.0)
    f.part("box", [block(f.w, "box", lo, hi)])
    out.append(("box.step", f, 1, 1, (lo, hi), "mm"))

    f = StepFile("ap214", "mm")
    f.part("cylinder", [cylinder(f.w, "cylinder", (10.0, -3.0, 1.0), Z, X, 4.0, 12.0)])
    out.append(("cylinder.step", f, 1, 1, ((6.0, -7.0, 1.0), (14.0, 1.0, 13.0)), "mm"))

    f = StepFile("ap203", "mm")
    f.part("rod", [cylinder(f.w, "rod", (-20.0, 5.0, 3.0), X, Y, 2.5, 40.0, seam=False)])
    out.append(("cylinder_x_axis_ap203.step", f, 1, 1, ((-20.0, 2.5, 0.5), (20.0, 7.5, 5.5)), "mm"))

    f = StepFile("ap214", "mm")
    holes = [((4.0, 4.0), 1.6), ((46.0, 4.0), 1.6), ((46.0, 36.0), 1.6), ((4.0, 36.0), 1.6), ((25.0, 20.0), 5.0)]
    f.part("hole_plate", [block(f.w, "hole_plate", (0.0, 0.0, 0.0), (50.0, 40.0, 1.6), holes)])
    out.append(("hole_plate.step", f, 1, 1, ((0.0, 0.0, 0.0), (50.0, 40.0, 1.6)), "mm"))

    f = StepFile("ap214", "mm")
    board = f.part("board", [block(f.w, "board", (0.0, 0.0, 0.0), (60.0, 40.0, 1.6), [((5.0, 5.0), 1.6)])], linked=True)
    part = f.part("part", [block(f.w, "part", (0.0, 0.0, 0.0), (10.0, 8.0, 3.0))], linked=True)
    f.assembly("assembly", [(board, "board", (0.0, 0.0, 0.0), Z, X), (part, "part", (45.0, 30.0, 1.6), Z, X)])
    out.append(("assembly.step", f, 2, 2, ((0.0, 0.0, 0.0), (60.0, 40.0, 4.6)), "mm"))

    f = StepFile("ap214", "mm")
    lo, hi = (0.0, 0.0, 0.0), (10.0, 4.0, 2.0)
    part = f.part("part", [block(f.w, "part", lo, hi)], linked=True)
    first = ((0.0, 0.0, 0.0), Z, X)
    turned = ((30.0, 10.0, 0.0), Z, Y)  # a quarter turn about z
    f.assembly("pair", [(part, "first", *first), (part, "turned", *turned)])
    box = union(placed(lo, hi, *first), placed(lo, hi, *turned))
    out.append(("assembly_shared_part_turned.step", f, 1, 2, box, "mm"))

    f = StepFile("ap214", "inch")
    lo, hi = (0.5, 0.25, 0.0), (2.5, 1.25, 0.0625)
    f.part("inch_block", [block(f.w, "inch_block", lo, hi)])
    out.append(("box_inch.step", f, 1, 1, (lo, hi), "inch"))

    f = StepFile("ap214", "inch")
    board = f.part("board", [block(f.w, "board", (0.0, 0.0, 0.0), (4.0, 3.0, 0.0625), [((0.25, 0.25), 0.0625)])], linked=True)
    part = f.part("part", [block(f.w, "part", (0.0, 0.0, 0.0), (0.5, 0.25, 0.125))], linked=True)
    f.assembly("inch_assembly", [(board, "board", (0.0, 0.0, 0.0), Z, X), (part, "part", (3.0, 2.5, 0.0625), Z, X)])
    out.append(("assembly_inch.step", f, 2, 2, ((0.0, 0.0, 0.0), (4.0, 3.0, 0.1875)), "inch"))

    f = StepFile("ap242", "mm")
    board_lo, board_hi = (0.0, 0.0, 0.0), (100.0, 80.0, 1.6)
    part_lo, part_hi = (40.0, 35.0, 1.6), (60.0, 45.0, 6.6)
    board = f.part("board", [block(f.w, "board", board_lo, board_hi), block(f.w, "part", part_lo, part_hi)], linked=True)
    # The board's z (its thickness) along the assembly's y: the y-up convention of some systems.
    y_up = ((0.0, 0.0, 0.0), Y, X)
    f.assembly("y_up", [(board, "board", *y_up)])
    box = union(placed(board_lo, board_hi, *y_up), placed(part_lo, part_hi, *y_up))
    out.append(("y_up.step", f, 2, 2, box, "mm"))

    f = StepFile("ap214", "m")
    lo, hi = (-0.0125, -0.005, 0.0), (0.0125, 0.005, 0.002)
    f.part("metre_block", [block(f.w, "metre_block", lo, hi)])
    out.append(("box_metre.step", f, 1, 1, (lo, hi), "m"))
    return out


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    expected = []
    for name, f, bodies, instances, (lo, hi), unit in fixtures():
        (OUT / name).write_text(f.text(name), newline="\n")
        min_nm, max_nm = box_of(lo, hi, unit)
        expected.append({"file": name, "unit": unit, "bodies": bodies, "placements": instances, "min_nm": min_nm, "max_nm": max_nm})
    rows = ",\n".join("    " + json.dumps(row) for row in expected)
    document = '{\n  "tolerance_nm": 1000,\n  "fixtures": [\n' + rows + "\n  ]\n}\n"
    json.loads(document)
    (OUT / "expected.json").write_text(document, newline="\n")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Convert the pinned OFL Relief SVG font into bounded, deterministic stroke data.

Usage: python3 tools/prepare_technical_font.py SOURCE.svg OUTPUT.json
This original converter reads font data only; it executes no upstream code.
Cubic curves flatten to at most 0.5 source units of control-polygon error.
"""
import hashlib
import json
import math
from pathlib import Path
import re
import sys
import tempfile
import xml.etree.ElementTree as ET

SOURCE_SHA256 = '75f05a5b64ed6039c9816628ee051d98e16c19148a8268c63f5eccf8382479e2'
TOKEN = re.compile(r'[A-Za-z]|[-+]?(?:\d*\.\d+|\d+\.?\d*)(?:[eE][-+]?\d+)?')


def flatten(a, b, c, d, out, depth=0):
    """Adaptively split cubic Béziers; a hard depth bound caps pathological font input."""
    dx, dy = d[0]-a[0], d[1]-a[1]
    length = math.hypot(dx, dy)
    def distance(p):
        t = max(0., min(1., ((p[0]-a[0])*dx+(p[1]-a[1])*dy)/(length*length))) if length else 0.
        return math.dist(p, (a[0]+t*dx, a[1]+t*dy))
    error = max(distance(p) for p in [b, c])
    if error <= 0.5 or depth == 12:
        out.append(d)
        return
    mid = lambda p, q: ((p[0]+q[0])/2, (p[1]+q[1])/2)
    ab, bc, cd = mid(a, b), mid(b, c), mid(c, d)
    abc, bcd = mid(ab, bc), mid(bc, cd)
    center = mid(abc, bcd)
    flatten(a, ab, abc, center, out, depth+1)
    flatten(center, bcd, cd, d, out, depth+1)


def paths(data):
    """Read SVG move/line/cubic/smooth-cubic/close commands, refusing unsupported kinds."""
    tokens = TOKEN.findall(data)
    if TOKEN.sub('', data).replace(',', '').strip():
        raise ValueError('Invalid path tokens')
    result, path = [], []
    point, control, start = (0., 0.), None, None
    i, command = 0, None
    while i < len(tokens):
        if tokens[i].isalpha():
            command = tokens[i]
            i += 1
        if command is None or command.upper() not in 'MLHVCSZ':
            raise ValueError(f'Unsupported SVG command: {command}')
        kind = command.upper()
        if kind == 'Z':
            if point != start:
                path.append(start)
            point, control, command = start, None, None
            continue
        count = {'M':2, 'L':2, 'H':1, 'V':1, 'C':6, 'S':4}[kind]
        values = list(map(float, tokens[i:i+count]))
        if len(values) != count:
            raise ValueError('Truncated SVG path')
        i += count
        def coord(offset):
            x, y = values[offset:offset+2]
            return (x+point[0], y+point[1]) if command.islower() else (x, y)
        previous_control = None
        if kind in 'ML':
            next_point = coord(0)
            if kind == 'M':
                if path:
                    result.append(path)
                path = []
                start = next_point
                command = 'l' if command.islower() else 'L'
            path.append(next_point)
        elif kind == 'H':
            next_point = (values[0]+point[0] if command.islower() else values[0], point[1])
            path.append(next_point)
        elif kind == 'V':
            next_point = (point[0], values[0]+point[1] if command.islower() else values[0])
            path.append(next_point)
        else:
            if kind == 'C':
                first, second, next_point = coord(0), coord(2), coord(4)
            else:
                first = (2*point[0]-control[0], 2*point[1]-control[1]) if control else point
                second, next_point = coord(0), coord(2)
            flatten(point, first, second, next_point, path)
            previous_control = second
        point, control = next_point, previous_control
    if path:
        result.append(path)
    return [[[round(x,4), round(y,4)] for x,y in path] for path in result if len(path) > 1]


def main():
    """Verify the upstream data, preserve proportional advances, and write a renamed OFL derivative."""
    source, destination = map(Path, sys.argv[1:])
    raw = source.read_bytes()
    if hashlib.sha256(raw).hexdigest() != SOURCE_SHA256:
        raise ValueError('Source SVG differs from the pinned font revision')
    font = ET.fromstring(raw).find('.//{*}font')
    glyphs = {}
    for glyph in font.findall('{*}glyph'):
        char = glyph.get('unicode', '')
        if len(char) != 1 or ord(char) < 32:
            continue
        glyphs[char] = {'advance':float(glyph.get('horiz-adv-x', font.get('horiz-adv-x'))), 'paths':paths(glyph.get('d', ''))}
    cap_height = max(p[1] for path in glyphs['H']['paths'] for p in path)
    notices = Path(__file__).resolve().parents[1] / 'crates/engine/assets/fonts/pcbcraft-technical'
    output = {'family':'PCBcraft Technical', 'revision':1, 'license':'OFL-1.1',
              'copyright':'Copyright 2022 The Relief SingleLine Project Authors (https://github.com/isdat-type/Relief-SingleLine)',
              'license_text':(notices/'OFL.txt').read_text(),
              'authors':(notices/'AUTHORS.txt').read_text(),
              'contributors':(notices/'CONTRIBUTORS.txt').read_text(),
              'source_revision':'01dfc5779ec1e9e4b288d96c6c96c23bfccbaf9d',
              'cap_height':cap_height, 'glyphs':glyphs}
    encoded = json.dumps(output, ensure_ascii=True, sort_keys=True, separators=(',',':'))+'\n'
    staged = None
    try:
        with tempfile.NamedTemporaryFile(mode='w', encoding='utf-8', dir=destination.parent, prefix='.pcbcraft-font-', delete=False) as handle:
            staged = Path(handle.name)
            handle.write(encoded)
        staged.replace(destination)
    finally:
        if staged is not None:
            staged.unlink(missing_ok=True)
    print(f'{len(glyphs)} glyphs, flat capital height {cap_height:g}, {destination.stat().st_size} bytes')


if __name__ == '__main__':
    main()
